use std::{
    collections::HashMap,
    fs,
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use serde::Serialize;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UdpSocket, UnixStream},
    process::{Child, Command},
    sync::Mutex,
    time::{sleep, timeout},
};

use crate::runtime::RuntimeError;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const START_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REPLY_BYTES: usize = 256 * 1024;

#[derive(Clone)]
pub struct UbridgeRuntime {
    binary: PathBuf,
    socket: PathBuf,
    daemon: Arc<Mutex<Option<Child>>>,
    link_bridges: Arc<Mutex<HashMap<String, String>>>,
    host_bridge_taps: Arc<Mutex<HashMap<String, (String, String)>>>,
}

#[derive(Serialize)]
pub struct UbridgeRuntimeInfo {
    pub available: bool,
    pub binary: String,
    pub version: Option<String>,
    pub error: Option<String>,
}

impl UbridgeRuntime {
    pub fn new() -> Self {
        let state_dir = std::env::var_os("NETWORK_SIMULATOR_UBRIDGE_STATE")
            .map(PathBuf::from)
            .unwrap_or_else(|| crate::image_paths::application_data_directory().join("run"));
        Self {
            binary: crate::image_paths::runtime_binary_prefer_path(
                "NETWORK_SIMULATOR_UBRIDGE",
                "ubridge",
            ),
            socket: state_dir.join("ubridge.sock"),
            daemon: Arc::new(Mutex::new(None)),
            link_bridges: Arc::new(Mutex::new(HashMap::new())),
            host_bridge_taps: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[cfg(test)]
    pub fn with_paths(binary: impl Into<PathBuf>, socket: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            socket: socket.into(),
            daemon: Arc::new(Mutex::new(None)),
            link_bridges: Arc::new(Mutex::new(HashMap::new())),
            host_bridge_taps: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn version(&self) -> Result<String, RuntimeError> {
        let reply = self.command("hypervisor version").await?;
        Ok(reply
            .lines()
            .find_map(|line| line.strip_prefix("100-"))
            .unwrap_or("unknown")
            .trim()
            .to_owned())
    }

    pub async fn info(&self) -> UbridgeRuntimeInfo {
        match self.version().await {
            Ok(version) => {
                let probe = uuid::Uuid::new_v4().to_string();
                match endpoint_name(&probe) {
                    Ok(bridge) => match self.command(&format!("bridge create {bridge}")).await {
                        Ok(_) => match self.command(&format!("bridge delete {bridge}")).await {
                            Ok(_) => UbridgeRuntimeInfo {
                                available: true,
                                binary: self.binary.display().to_string(),
                                version: Some(version),
                                error: None,
                            },
                            Err(error) => UbridgeRuntimeInfo {
                                available: false,
                                binary: self.binary.display().to_string(),
                                version: Some(version),
                                error: Some(format!(
                                    "Could not remove uBridge capability probe: {}",
                                    error.message()
                                )),
                            },
                        },
                        Err(error) => UbridgeRuntimeInfo {
                            available: false,
                            binary: self.binary.display().to_string(),
                            version: Some(version),
                            error: Some(format!(
                                "Could not create a uBridge NIO relay: {}",
                                error.message()
                            )),
                        },
                    },
                    Err(error) => UbridgeRuntimeInfo {
                        available: false,
                        binary: self.binary.display().to_string(),
                        version: Some(version),
                        error: Some(error.message().to_owned()),
                    },
                }
            }
            Err(error) => UbridgeRuntimeInfo {
                available: false,
                binary: self.binary.display().to_string(),
                version: None,
                error: Some(error.message().to_owned()),
            },
        }
    }

    pub async fn create_veth_pair(&self, host: &str, guest: &str) -> Result<(), RuntimeError> {
        validate_interface_name(host)?;
        validate_interface_name(guest)?;
        self.command(&format!("docker create_veth {host} {guest}"))
            .await?;
        Ok(())
    }

    pub async fn move_veth_to_container(
        &self,
        guest_if: &str,
        container_pid: u32,
        guest_name: &str,
    ) -> Result<(), RuntimeError> {
        validate_interface_name(guest_if)?;
        validate_interface_name(guest_name)?;
        if container_pid == 0 {
            return Err(RuntimeError::Failed(
                "Docker did not report a running container network namespace".to_owned(),
            ));
        }
        self.command(&format!(
            "docker move_to_ns {guest_if} {container_pid} {guest_name}"
        ))
        .await?;
        Ok(())
    }

    pub async fn set_docker_mac_address(
        &self,
        interface: &str,
        mac_address: &str,
    ) -> Result<(), RuntimeError> {
        validate_interface_name(interface)?;
        let octets = mac_address.split(':').collect::<Vec<_>>();
        if octets.len() != 6
            || octets.iter().any(|octet| {
                octet.len() != 2 || !octet.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
            return Err(RuntimeError::Failed(
                "Docker interface MAC address must contain six hexadecimal octets".to_owned(),
            ));
        }
        self.command(&format!("docker set_mac_addr {interface} {mac_address}"))
            .await?;
        Ok(())
    }

    pub async fn set_interface_up(&self, interface: &str) -> Result<(), RuntimeError> {
        validate_interface_name(interface)?;
        self.command(&format!("link set {interface} up")).await?;
        Ok(())
    }

    pub async fn reserve_udp_port(&self) -> Result<u16, RuntimeError> {
        available_udp_port().await
    }

    /// Attach one endpoint of a point-to-point link to a runtime adapter.
    /// Each endpoint owns a uBridge bridge joining the runtime NIO to the
    /// reciprocal link UDP tunnel NIO. Docker adapters use an Ethernet NIO;
    /// the emulators use a local UDP NIO.
    pub async fn attach_link_endpoint(
        &self,
        endpoint_id: &str,
        link_local_port: u16,
        peer_link_port: u16,
        runtime_local_port: Option<u16>,
        runtime_peer_port: Option<u16>,
        ethernet_interface: Option<&str>,
        filters: &serde_json::Value,
        suspended: bool,
        capture_file: Option<&str>,
    ) -> Result<(), RuntimeError> {
        self.attach_link_endpoint_with_peer(
            endpoint_id,
            link_local_port,
            peer_link_port,
            runtime_local_port,
            runtime_peer_port,
            ethernet_interface,
            None,
            None,
            filters,
            suspended,
            capture_file,
        )
        .await
    }

    /// Attach an endpoint to either a runtime UDP NIO or a host-side Cloud
    /// NIO. Cloud ports use Ethernet, TAP, or a separately configured UDP
    /// peer; ordinary VM ports keep the reciprocal runtime UDP tunnel.
    pub async fn attach_link_endpoint_with_peer(
        &self,
        endpoint_id: &str,
        link_local_port: u16,
        peer_link_port: u16,
        runtime_local_port: Option<u16>,
        runtime_peer_port: Option<u16>,
        ethernet_interface: Option<&str>,
        tap_interface: Option<&str>,
        external_udp: Option<(u16, &str, u16)>,
        filters: &serde_json::Value,
        suspended: bool,
        capture_file: Option<&str>,
    ) -> Result<(), RuntimeError> {
        self.attach_link_endpoint_internal(
            endpoint_id,
            link_local_port,
            peer_link_port,
            runtime_local_port,
            runtime_peer_port,
            ethernet_interface,
            tap_interface,
            external_udp,
            None,
            filters,
            suspended,
            capture_file,
        )
        .await
    }

    /// Attach a host Ethernet interface through Linux's appropriate NIO.
    /// Physical interfaces use uBridge raw Ethernet; bridge interfaces use a
    /// temporary TAP enslaved to the selected bridge, matching Cloud/NAT.
    pub async fn attach_link_endpoint_with_host_ethernet(
        &self,
        endpoint_id: &str,
        link_local_port: u16,
        peer_link_port: u16,
        host_interface: &str,
        tap_interface: &str,
        filters: &serde_json::Value,
        suspended: bool,
        capture_file: Option<&str>,
    ) -> Result<(), RuntimeError> {
        self.attach_link_endpoint_internal(
            endpoint_id,
            link_local_port,
            peer_link_port,
            None,
            None,
            None,
            None,
            None,
            Some((host_interface, tap_interface)),
            filters,
            suspended,
            capture_file,
        )
        .await
    }

    async fn attach_link_endpoint_internal(
        &self,
        endpoint_id: &str,
        link_local_port: u16,
        peer_link_port: u16,
        runtime_local_port: Option<u16>,
        runtime_peer_port: Option<u16>,
        ethernet_interface: Option<&str>,
        tap_interface: Option<&str>,
        external_udp: Option<(u16, &str, u16)>,
        host_ethernet: Option<(&str, &str)>,
        filters: &serde_json::Value,
        suspended: bool,
        capture_file: Option<&str>,
    ) -> Result<(), RuntimeError> {
        let bridge = endpoint_name(endpoint_id)?;
        if self.link_bridges.lock().await.get(endpoint_id) == Some(&bridge) {
            self.update_link_endpoint(endpoint_id, filters, suspended)
                .await?;
            if let Some(path) = capture_file {
                self.start_link_capture(endpoint_id, path).await?;
            }
            return Ok(());
        }

        let mut created = false;
        let mut host_bridge_tap = None;
        let setup = async {
            self.command(&format!("bridge create {bridge}")).await?;
            created = true;
            if let Some((interface, tap)) = host_ethernet {
                validate_interface_name(interface)?;
                validate_interface_name(tap)?;
                if is_linux_bridge_interface(interface) {
                    self.command(&format!("bridge add_nio_tap {bridge} \"{tap}\""))
                        .await?;
                    self.command(&format!("brctl addif \"{interface}\" \"{tap}\""))
                        .await?;
                    host_bridge_tap = Some((interface.to_owned(), tap.to_owned()));
                } else {
                    self.command(&format!(
                        "bridge add_nio_linux_raw {bridge} \"{interface}\""
                    ))
                    .await?;
                }
            } else if let Some(interface) = ethernet_interface {
                validate_interface_name(interface)?;
                self.command(&format!("bridge add_nio_ethernet {bridge} {interface}"))
                    .await?;
            } else if let Some(interface) = tap_interface {
                validate_interface_name(interface)?;
                self.command(&format!("bridge add_nio_tap {bridge} {interface}"))
                    .await?;
            } else if let Some((local_port, remote_host, remote_port)) = external_udp {
                validate_remote_host(remote_host)?;
                if local_port == 0 || remote_port == 0 {
                    return Err(RuntimeError::Failed(
                        "Cloud UDP ports must be between 1 and 65535".to_owned(),
                    ));
                }
                self.command(&format!(
                    "bridge add_nio_udp {bridge} {local_port} {remote_host} {remote_port}"
                ))
                .await?;
            } else {
                let local = runtime_local_port.ok_or_else(|| {
                    RuntimeError::Failed("runtime UDP NIO has no local port".to_owned())
                })?;
                let peer = runtime_peer_port.ok_or_else(|| {
                    RuntimeError::Failed("runtime UDP NIO has no peer port".to_owned())
                })?;
                self.command(&format!(
                    "bridge add_nio_udp {bridge} {local} 127.0.0.1 {peer}"
                ))
                .await?;
            }
            self.command(&format!(
                "bridge add_nio_udp {bridge} {link_local_port} 127.0.0.1 {peer_link_port}"
            ))
            .await?;
            self.command(&format!("bridge start {bridge}")).await?;
            self.apply_link_filters(&bridge, filters, suspended).await?;
            if let Some(path) = capture_file {
                self.command(&format!(
                    "bridge start_capture {bridge} \"{}\"",
                    escape_ubridge_arg(path)
                ))
                .await?;
            }
            Ok::<(), RuntimeError>(())
        }
        .await;
        if let Err(error) = setup {
            if let Some((interface, tap)) = host_bridge_tap.as_ref() {
                let _ = self
                    .command(&format!("brctl delif \"{interface}\" \"{tap}\""))
                    .await;
            }
            if created {
                let _ = self.command(&format!("bridge delete {bridge}")).await;
            }
            return Err(error);
        }
        if let Some(host_bridge_tap) = host_bridge_tap {
            self.host_bridge_taps
                .lock()
                .await
                .insert(endpoint_id.to_owned(), host_bridge_tap);
        }
        self.link_bridges
            .lock()
            .await
            .insert(endpoint_id.to_owned(), bridge);
        Ok(())
    }

    pub async fn update_link_endpoint(
        &self,
        endpoint_id: &str,
        filters: &serde_json::Value,
        suspended: bool,
    ) -> Result<(), RuntimeError> {
        let bridge = endpoint_name(endpoint_id)?;
        if !self.link_bridges.lock().await.contains_key(endpoint_id) {
            return Ok(());
        }
        self.apply_link_filters(&bridge, filters, suspended).await
    }

    async fn apply_link_filters(
        &self,
        bridge: &str,
        filters: &serde_json::Value,
        suspended: bool,
    ) -> Result<(), RuntimeError> {
        self.command(&format!("bridge reset_packet_filters {bridge}"))
            .await?;
        if suspended {
            self.command(&format!(
                "bridge add_packet_filter {bridge} suspended frequency_drop -1"
            ))
            .await?;
            return Ok(());
        }
        let Some(filters) = filters.as_object() else {
            return Ok(());
        };
        let mut index = 0usize;
        for (kind, values) in filters {
            if !matches!(kind.as_str(), "delay" | "packet_loss" | "corrupt" | "bpf") {
                return Err(RuntimeError::Failed(format!(
                    "Unsupported uBridge link filter '{kind}'"
                )));
            }
            let Some(values) = values.as_array() else {
                return Err(RuntimeError::Failed(format!(
                    "Link filter '{kind}' must contain an array of values"
                )));
            };
            if kind == "bpf" {
                let Some(expression) = values.first().and_then(serde_json::Value::as_str) else {
                    return Err(RuntimeError::Failed(
                        "BPF link filter must begin with a text expression".to_owned(),
                    ));
                };
                for line in expression
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty())
                {
                    let extra = values
                        .iter()
                        .skip(1)
                        .map(filter_value)
                        .collect::<Result<Vec<_>, _>>()?
                        .join(" ");
                    self.command(&format!(
                        "bridge add_packet_filter {bridge} filter{index} bpf \"{}\" {extra}",
                        escape_ubridge_arg(line)
                    ))
                    .await?;
                    index += 1;
                }
            } else {
                let arguments = values
                    .iter()
                    .map(filter_value)
                    .collect::<Result<Vec<_>, _>>()?
                    .join(" ");
                self.command(&format!(
                    "bridge add_packet_filter {bridge} filter{index} {kind} {arguments}"
                ))
                .await?;
                index += 1;
            }
        }
        Ok(())
    }

    pub async fn start_link_capture(
        &self,
        endpoint_id: &str,
        capture_file: &str,
    ) -> Result<(), RuntimeError> {
        if !self.link_bridges.lock().await.contains_key(endpoint_id) {
            return Err(RuntimeError::Failed(
                "Link capture requires a running endpoint".to_owned(),
            ));
        }
        let bridge = endpoint_name(endpoint_id)?;
        self.command(&format!(
            "bridge start_capture {bridge} \"{}\"",
            escape_ubridge_arg(capture_file)
        ))
        .await?;
        Ok(())
    }

    pub async fn stop_link_capture(&self, endpoint_id: &str) -> Result<(), RuntimeError> {
        if !self.link_bridges.lock().await.contains_key(endpoint_id) {
            return Ok(());
        }
        let bridge = endpoint_name(endpoint_id)?;
        self.command(&format!("bridge stop_capture {bridge}"))
            .await?;
        Ok(())
    }

    pub async fn delete_link_endpoint(&self, endpoint_id: &str) -> Result<(), RuntimeError> {
        if !self.link_bridges.lock().await.contains_key(endpoint_id) {
            return Ok(());
        }
        let bridge = endpoint_name(endpoint_id)?;
        if let Some((interface, tap)) = self.host_bridge_taps.lock().await.get(endpoint_id).cloned()
        {
            let _ = self
                .command(&format!("brctl delif \"{interface}\" \"{tap}\""))
                .await;
        }
        self.command(&format!("bridge delete {bridge}")).await?;
        self.link_bridges.lock().await.remove(endpoint_id);
        self.host_bridge_taps.lock().await.remove(endpoint_id);
        Ok(())
    }

    pub async fn has_link_endpoint(&self, endpoint_id: &str) -> bool {
        self.link_bridges.lock().await.contains_key(endpoint_id)
    }

    pub async fn delete_veth(&self, host: &str) -> Result<(), RuntimeError> {
        validate_interface_name(host)?;
        self.command(&format!("docker delete_veth {host}")).await?;
        Ok(())
    }

    async fn command(&self, command: &str) -> Result<String, RuntimeError> {
        self.ensure_started().await?;
        timeout(COMMAND_TIMEOUT, async {
            let stream = UnixStream::connect(&self.socket).await.map_err(|error| {
                RuntimeError::Unavailable(format!(
                    "Could not connect to the uBridge control socket '{}': {error}",
                    self.socket.display()
                ))
            })?;
            let mut stream = BufReader::new(stream);
            stream
                .get_mut()
                .write_all(format!("{command}\n").as_bytes())
                .await
                .map_err(|error| {
                    RuntimeError::Unavailable(format!("Could not send uBridge command: {error}"))
                })?;
            let mut reply = String::new();
            loop {
                let mut line = String::new();
                let read = stream.read_line(&mut line).await.map_err(|error| {
                    RuntimeError::Unavailable(format!("Could not read uBridge response: {error}"))
                })?;
                if read == 0 {
                    return Err(RuntimeError::Unavailable(
                        "uBridge closed its control connection without replying".to_owned(),
                    ));
                }
                if reply.len() + line.len() > MAX_REPLY_BYTES {
                    return Err(RuntimeError::Failed(
                        "uBridge response exceeded the controller limit".to_owned(),
                    ));
                }
                let final_reply = is_final_reply(&line);
                reply.push_str(&line);
                if final_reply {
                    break;
                }
            }
            if reply
                .lines()
                .last()
                .is_some_and(|line| !line.starts_with("100-"))
            {
                if reply.contains("Operation not permitted") || reply.contains("EPERM") {
                    return Err(RuntimeError::Unavailable(format!(
                        "uBridge needs CAP_NET_ADMIN and CAP_NET_RAW for kernel networking: {}",
                        reply.trim()
                    )));
                }
                return Err(RuntimeError::Failed(format!(
                    "uBridge command failed: {}",
                    reply.trim()
                )));
            }
            Ok(reply)
        })
        .await
        .map_err(|_| RuntimeError::Unavailable("uBridge command timed out".to_owned()))?
    }

    async fn ensure_started(&self) -> Result<(), RuntimeError> {
        if UnixStream::connect(&self.socket).await.is_ok() {
            return Ok(());
        }
        let mut daemon = self.daemon.lock().await;
        if let Some(child) = daemon.as_mut()
            && child.try_wait().ok().flatten().is_none()
        {
            wait_for_socket(&self.socket).await?;
            return Ok(());
        }
        self.link_bridges.lock().await.clear();
        if let Some(parent) = self.socket.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                RuntimeError::Failed(format!("Could not create uBridge state directory: {error}"))
            })?;
        }
        match fs::remove_file(&self.socket) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(RuntimeError::Failed(format!(
                    "Could not remove stale uBridge socket: {error}"
                )));
            }
        }
        let child = Command::new(&self.binary)
            .arg("-U")
            .arg(&self.socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                RuntimeError::Unavailable(format!(
                    "Could not start upstream uBridge executable '{}': {error}",
                    self.binary.display()
                ))
            })?;
        *daemon = Some(child);
        wait_for_socket(&self.socket).await
    }
}

impl Default for UbridgeRuntime {
    fn default() -> Self {
        Self::new()
    }
}

fn is_final_reply(line: &str) -> bool {
    line.as_bytes().get(3) == Some(&b'-')
}

async fn wait_for_socket(socket: &PathBuf) -> Result<(), RuntimeError> {
    let deadline = tokio::time::Instant::now() + START_TIMEOUT;
    loop {
        if UnixStream::connect(socket).await.is_ok() {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(RuntimeError::Unavailable(format!(
                "uBridge did not create its control socket '{}'; mixed-runtime links need upstream uBridge and CAP_NET_ADMIN/CAP_NET_RAW",
                socket.display()
            )));
        }
        sleep(Duration::from_millis(40)).await;
    }
}

fn validate_interface_name(name: &str) -> Result<(), RuntimeError> {
    if name.is_empty()
        || name.len() > 15
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(RuntimeError::Failed(
            "uBridge interface names must contain 1–15 safe ASCII characters".to_owned(),
        ));
    }
    Ok(())
}

fn validate_remote_host(host: &str) -> Result<(), RuntimeError> {
    if host.is_empty()
        || host.len() > 255
        || !host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b':' | b'%'))
    {
        return Err(RuntimeError::Failed(
            "Cloud UDP remote host must be an IP address or safe hostname".to_owned(),
        ));
    }
    Ok(())
}

fn filter_value(value: &serde_json::Value) -> Result<String, RuntimeError> {
    match value {
        serde_json::Value::Number(number) => Ok(number.to_string()),
        serde_json::Value::String(text) if !text.contains(['\n', '\r']) && !text.is_empty() => {
            Ok(format!("\"{}\"", escape_ubridge_arg(text)))
        }
        _ => Err(RuntimeError::Failed(
            "Link filter values must be numbers or single-line text".to_owned(),
        )),
    }
}

fn escape_ubridge_arg(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn host_veth_name(endpoint_id: &str) -> Result<String, RuntimeError> {
    Ok(format!("nsv{}", compact_id(endpoint_id)?))
}

#[cfg(test)]
pub fn guest_veth_name(endpoint_id: &str) -> Result<String, RuntimeError> {
    Ok(format!("nsg{}", compact_id(endpoint_id)?))
}

fn is_linux_bridge_interface(interface: &str) -> bool {
    cfg!(target_os = "linux")
        && std::path::Path::new("/sys/class/net")
            .join(interface)
            .join("bridge")
            .exists()
}

fn endpoint_name(endpoint_id: &str) -> Result<String, RuntimeError> {
    Ok(format!("nse{}", compact_id(endpoint_id)?))
}

fn compact_id(value: &str) -> Result<String, RuntimeError> {
    let compact = value
        .chars()
        .filter(|character| character.is_ascii_hexdigit())
        .collect::<String>();
    if compact.len() < 8 {
        return Err(RuntimeError::Failed(
            "Endpoint ID must contain at least eight hexadecimal characters".to_owned(),
        ));
    }
    Ok(compact[..12.min(compact.len())].to_owned())
}

async fn available_udp_port() -> Result<u16, RuntimeError> {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not allocate a uBridge UDP port: {error}"))
        })?;
    socket
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not inspect the uBridge UDP port: {error}"))
        })
}

#[cfg(test)]
mod tests {
    use std::{env, path::PathBuf, time::Duration};

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        process::Command,
        time::timeout,
    };

    use super::{UbridgeRuntime, endpoint_name, guest_veth_name, host_veth_name, is_final_reply};
    use crate::{
        runtime::{DockerRuntime, NodeAction, RuntimeError},
        vpcs::VpcsRuntime,
    };

    #[test]
    fn generates_endpoint_and_veth_names_that_fit_linux_limits() {
        let id = "01234567-89ab-cdef-0123-456789abcdef";
        for name in [
            host_veth_name(id).unwrap(),
            guest_veth_name(id).unwrap(),
            endpoint_name(id).unwrap(),
        ] {
            assert!(name.len() <= 15);
            assert!(
                name.bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            );
        }
        assert_eq!(super::compact_id(id).unwrap(), "0123456789ab");
    }

    #[test]
    fn recognizes_the_final_line_in_the_upstream_control_protocol() {
        assert!(!is_final_reply("101 bridge is running\n"));
        assert!(is_final_reply("100-bridge started\n"));
        assert!(is_final_reply("206-Could not create bridge\n"));
    }

    #[tokio::test]
    #[ignore = "requires Docker, alpine:latest, upstream VPCS, and uBridge CAP_NET_ADMIN/CAP_NET_RAW"]
    async fn alpine_and_vpcs_exchange_icmp_over_reciprocal_udp_nio_bridges() {
        let ubridge_binary = env::var_os("NETWORK_SIMULATOR_TEST_UBRIDGE")
            .expect("NETWORK_SIMULATOR_TEST_UBRIDGE names a capable upstream uBridge binary");
        let vpcs_binary = env::var_os("NETWORK_SIMULATOR_TEST_VPCS")
            .expect("NETWORK_SIMULATOR_TEST_VPCS names the upstream VPCS binary");
        let directory = tempfile::tempdir().expect("temporary uBridge and VPCS state");
        let ubridge = UbridgeRuntime::with_paths(
            PathBuf::from(ubridge_binary),
            directory.path().join("ubridge.sock"),
        );
        let vpcs =
            VpcsRuntime::with_paths(PathBuf::from(vpcs_binary), directory.path().join("vpcs"));
        let docker = DockerRuntime::new();
        let project_id = uuid::Uuid::new_v4().to_string();
        let vpcs_id = uuid::Uuid::new_v4().to_string();
        let docker_node_id = uuid::Uuid::new_v4().to_string();
        let vpcs_endpoint_id = uuid::Uuid::new_v4().to_string();
        let docker_endpoint_id = uuid::Uuid::new_v4().to_string();
        let vpcs_link_port = ubridge.reserve_udp_port().await.expect("VPCS link port");
        let docker_link_port = ubridge.reserve_udp_port().await.expect("Docker link port");
        let adapter_port = ubridge.reserve_udp_port().await.expect("VPCS adapter port");
        let runtime_port = ubridge.reserve_udp_port().await.expect("VPCS runtime port");
        let docker_host = host_veth_name(&docker_endpoint_id).expect("stable Docker host veth");
        let docker_guest = guest_veth_name(&docker_endpoint_id).expect("stable Docker guest veth");
        let docker_binary = env::var_os("NETWORK_SIMULATOR_DOCKER")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("docker"));
        let mut container_id: Option<String> = None;
        let mut vpcs_started = false;

        let run: Result<(), RuntimeError> = async {
            let (created_container, _) = docker
                .create_container(
                    &project_id,
                    &docker_node_id,
                    "alpine:latest",
                    "uBridge Alpine peer",
                    Some("/bin/sh -c 'sleep 3600'"),
                    None,
                    None,
                )
                .await?;
            container_id = Some(created_container.clone());
            docker
                .lifecycle(&created_container, NodeAction::Start)
                .await?;
            let namespace_pid = docker.container_pid(&created_container).await?;
            ubridge
                .create_veth_pair(&docker_host, &docker_guest)
                .await?;
            ubridge
                .set_docker_mac_address(&docker_guest, "02:00:00:00:00:01")
                .await?;
            ubridge.set_interface_up(&docker_guest).await?;
            ubridge.set_interface_up(&docker_host).await?;
            ubridge
                .move_veth_to_container(&docker_guest, namespace_pid, "eth0")
                .await?;

            vpcs.set_nio_ports(&vpcs_id, adapter_port, runtime_port)
                .await;
            vpcs.lifecycle(
                &project_id,
                &vpcs_id,
                "uBridge VPCS peer",
                Some("ip 10.0.0.1/24"),
                NodeAction::Start,
            )
            .await?;
            vpcs_started = true;

            ubridge
                .attach_link_endpoint(
                    &docker_endpoint_id,
                    docker_link_port,
                    vpcs_link_port,
                    None,
                    None,
                    Some(&docker_host),
                    &serde_json::json!({}),
                    false,
                    None,
                )
                .await?;
            ubridge
                .attach_link_endpoint(
                    &vpcs_endpoint_id,
                    vpcs_link_port,
                    docker_link_port,
                    Some(adapter_port),
                    Some(runtime_port),
                    None,
                    &serde_json::json!({}),
                    false,
                    None,
                )
                .await?;

            for args in [
                vec!["ip", "addr", "add", "10.0.0.2/24", "dev", "eth0"],
                vec!["ip", "link", "set", "eth0", "up"],
            ] {
                let output = Command::new(&docker_binary)
                    .args(["exec", &created_container])
                    .args(args)
                    .output()
                    .await
                    .map_err(|error| {
                        RuntimeError::Unavailable(format!(
                            "Could not configure Alpine eth0: {error}"
                        ))
                    })?;
                if !output.status.success() {
                    return Err(RuntimeError::Failed(format!(
                        "Could not configure Alpine eth0: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    )));
                }
            }

            let mut console = vpcs.connect_console(&vpcs_id).await?;
            let mut output = Vec::new();
            timeout(Duration::from_secs(5), async {
                loop {
                    let mut chunk = [0_u8; 4096];
                    let length = console.read(&mut chunk).await?;
                    output.extend_from_slice(&chunk[..length]);
                    if String::from_utf8_lossy(&output).contains("VPCS>") {
                        break Ok::<(), std::io::Error>(());
                    }
                }
            })
            .await
            .map_err(|_| RuntimeError::Unavailable("VPCS prompt timed out".to_owned()))?
            .map_err(|error| RuntimeError::Unavailable(format!("VPCS console failed: {error}")))?;
            console
                .write_all(b"ping 10.0.0.2 -c 1\r")
                .await
                .map_err(|error| {
                    RuntimeError::Unavailable(format!("VPCS console failed: {error}"))
                })?;
            let mut ping_output = Vec::new();
            timeout(Duration::from_secs(15), async {
                loop {
                    let mut chunk = [0_u8; 4096];
                    let length = console.read(&mut chunk).await?;
                    ping_output.extend_from_slice(&chunk[..length]);
                    if String::from_utf8_lossy(&ping_output).contains("bytes from 10.0.0.2") {
                        break Ok::<(), std::io::Error>(());
                    }
                }
            })
            .await
            .map_err(|_| {
                RuntimeError::Unavailable(format!(
                    "No ICMP reply from Alpine: {}",
                    String::from_utf8_lossy(&ping_output).trim()
                ))
            })?
            .map_err(|error| RuntimeError::Unavailable(format!("VPCS console failed: {error}")))?;
            Ok(())
        }
        .await;

        let _ = ubridge.delete_link_endpoint(&vpcs_endpoint_id).await;
        let _ = ubridge.delete_link_endpoint(&docker_endpoint_id).await;
        if vpcs_started {
            let _ = vpcs.stop(&vpcs_id).await;
        }
        if let Some(container_id) = container_id.as_deref() {
            let _ = docker
                .remove_node_resources(&docker_node_id, Some(container_id))
                .await;
        }
        let _ = ubridge.delete_veth(&docker_host).await;
        if let Err(error) = run {
            panic!(
                "real Alpine-to-VPCS UDP NIO test failed: {}",
                error.message()
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires Docker, alpine:latest, and uBridge CAP_NET_ADMIN/CAP_NET_RAW"]
    async fn alpine_containers_exchange_icmp_over_reciprocal_udp_nio_bridges() {
        let ubridge_binary = env::var_os("NETWORK_SIMULATOR_TEST_UBRIDGE")
            .expect("NETWORK_SIMULATOR_TEST_UBRIDGE names a capable upstream uBridge binary");
        let directory = tempfile::tempdir().expect("temporary uBridge state");
        let ubridge = UbridgeRuntime::with_paths(
            PathBuf::from(ubridge_binary),
            directory.path().join("ubridge.sock"),
        );
        let docker = DockerRuntime::new();
        let project_id = uuid::Uuid::new_v4().to_string();
        let nodes = [
            uuid::Uuid::new_v4().to_string(),
            uuid::Uuid::new_v4().to_string(),
        ];
        let endpoints = [
            uuid::Uuid::new_v4().to_string(),
            uuid::Uuid::new_v4().to_string(),
        ];
        let link_ports = [
            ubridge.reserve_udp_port().await.expect("first link port"),
            ubridge.reserve_udp_port().await.expect("second link port"),
        ];
        let hosts = [
            host_veth_name(&endpoints[0]).expect("first host veth name"),
            host_veth_name(&endpoints[1]).expect("second host veth name"),
        ];
        let guests = [
            guest_veth_name(&endpoints[0]).expect("first guest veth name"),
            guest_veth_name(&endpoints[1]).expect("second guest veth name"),
        ];
        let mut containers = [None::<String>, None::<String>];

        let run: Result<(), RuntimeError> = async {
            for index in 0..2 {
                let (container_id, _) = docker
                    .create_container(
                        &project_id,
                        &nodes[index],
                        "alpine:latest",
                        &format!("UDP NIO Alpine {index}"),
                        Some("/bin/sh -c 'sleep 3600'"),
                        None,
                        None,
                    )
                    .await?;
                containers[index] = Some(container_id.clone());
                docker.lifecycle(&container_id, NodeAction::Start).await?;
                let pid = docker.container_pid(&container_id).await?;
                ubridge
                    .create_veth_pair(&hosts[index], &guests[index])
                    .await?;
                ubridge
                    .set_docker_mac_address(
                        &guests[index],
                        if index == 0 {
                            "02:00:00:00:00:01"
                        } else {
                            "02:00:00:00:00:02"
                        },
                    )
                    .await?;
                ubridge.set_interface_up(&guests[index]).await?;
                ubridge.set_interface_up(&hosts[index]).await?;
                ubridge
                    .move_veth_to_container(&guests[index], pid, "eth0")
                    .await?;
                ubridge
                    .attach_link_endpoint(
                        &endpoints[index],
                        link_ports[index],
                        link_ports[1 - index],
                        None,
                        None,
                        Some(&hosts[index]),
                        &serde_json::json!({}),
                        false,
                        None,
                    )
                    .await?;
                let docker_binary = env::var_os("NETWORK_SIMULATOR_DOCKER")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("docker"));
                for args in [
                    vec![
                        "ip",
                        "addr",
                        "add",
                        if index == 0 {
                            "10.0.0.1/24"
                        } else {
                            "10.0.0.2/24"
                        },
                        "dev",
                        "eth0",
                    ],
                    vec!["ip", "link", "set", "eth0", "up"],
                ] {
                    let output = Command::new(&docker_binary)
                        .args(["exec", &container_id])
                        .args(args)
                        .output()
                        .await
                        .map_err(|error| {
                            RuntimeError::Unavailable(format!(
                                "Could not configure Alpine interface: {error}"
                            ))
                        })?;
                    if !output.status.success() {
                        return Err(RuntimeError::Failed(format!(
                            "Could not configure Alpine interface: {}",
                            String::from_utf8_lossy(&output.stderr).trim()
                        )));
                    }
                }
            }

            let docker_binary = env::var_os("NETWORK_SIMULATOR_DOCKER")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("docker"));
            let output = Command::new(docker_binary)
                .args([
                    "exec",
                    containers[0].as_deref().unwrap_or_default(),
                    "ping",
                    "-c",
                    "1",
                    "-W",
                    "3",
                    "10.0.0.2",
                ])
                .output()
                .await
                .map_err(|error| {
                    RuntimeError::Unavailable(format!(
                        "Could not run Alpine-to-Alpine ping: {error}"
                    ))
                })?;
            if !output.status.success()
                || !String::from_utf8_lossy(&output.stdout).contains("1 packets received")
            {
                return Err(RuntimeError::Failed(format!(
                    "Alpine-to-Alpine ICMP failed: {}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                )));
            }
            Ok(())
        }
        .await;

        for endpoint in &endpoints {
            let _ = ubridge.delete_link_endpoint(endpoint).await;
        }
        for (index, container_id) in containers.iter().enumerate() {
            if let Some(container_id) = container_id.as_deref() {
                let _ = docker
                    .remove_node_resources(&nodes[index], Some(container_id))
                    .await;
            }
            let _ = ubridge.delete_veth(&hosts[index]).await;
        }
        for host in &hosts {
            let output = Command::new("ip")
                .args(["link", "show", "dev", host])
                .output()
                .await
                .expect("inspect cleaned-up veth");
            assert!(
                !output.status.success(),
                "orphan host veth remained: {host}"
            );
        }
        if let Err(error) = run {
            panic!(
                "real Alpine-to-Alpine UDP NIO test failed: {}",
                error.message()
            );
        }
    }
}
