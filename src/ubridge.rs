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

mod bridge_forwarding;

#[cfg(test)]
mod live_tests;

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
    allocated_udp_ports: Arc<Mutex<HashMap<u16, String>>>,
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
            allocated_udp_ports: Arc::new(Mutex::new(HashMap::new())),
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
            allocated_udp_ports: Arc::new(Mutex::new(HashMap::new())),
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
        self.reserve_udp_port_for(&uuid::Uuid::new_v4().to_string())
            .await
    }

    pub async fn reserve_udp_port_for(&self, owner: &str) -> Result<u16, RuntimeError> {
        let mut allocated = self.allocated_udp_ports.lock().await;
        for _ in 0..128 {
            let port = available_udp_port().await?;
            if !allocated.contains_key(&port) {
                allocated.insert(port, owner.to_owned());
                return Ok(port);
            }
        }
        Err(RuntimeError::Unavailable(
            "Could not reserve a unique UDP port".to_owned(),
        ))
    }

    /// Claim a persisted port during startup reconciliation so new links do
    /// not reuse ports that belong to links created by an earlier process.
    pub async fn claim_udp_port(&self, owner: &str, port: u16) -> Result<(), RuntimeError> {
        if port == 0 {
            return Err(RuntimeError::Failed(
                "Cannot reserve UDP port zero".to_owned(),
            ));
        }
        let mut allocated = self.allocated_udp_ports.lock().await;
        match allocated.get(&port) {
            Some(current) if current == owner => Ok(()),
            Some(_) => Err(RuntimeError::Failed(format!(
                "UDP port {port} is already reserved by another link"
            ))),
            None => {
                allocated.insert(port, owner.to_owned());
                Ok(())
            }
        }
    }

    pub async fn release_udp_ports(&self, owner: &str, ports: impl IntoIterator<Item = u16>) {
        let mut allocated = self.allocated_udp_ports.lock().await;
        for port in ports {
            if allocated.get(&port).is_some_and(|current| current == owner) {
                allocated.remove(&port);
            }
        }
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
        if capture_file.is_some_and(|path| path.chars().any(char::is_control)) {
            return Err(RuntimeError::Failed(
                "uBridge capture paths cannot contain control characters".to_owned(),
            ));
        }
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
            bridge_forwarding::enable_edge_forwarding(
                host_bridge_tap.as_ref().map(|(_, tap)| tap.as_str()),
            )
            .await?;
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
        let mut commands = Vec::new();
        if suspended {
            commands.push(format!(
                "bridge add_packet_filter {bridge} suspended frequency_drop -1"
            ));
        } else if let Some(filters) = filters.as_object() {
            let mut index = 0usize;
            for (kind, values) in filters {
                if !matches!(
                    kind.as_str(),
                    "delay" | "frequency_drop" | "packet_loss" | "corrupt" | "bpf"
                ) {
                    return Err(RuntimeError::Failed(format!(
                        "Unsupported uBridge link filter '{kind}'"
                    )));
                }
                let Some(values) = values.as_array() else {
                    return Err(RuntimeError::Failed(format!(
                        "Link filter '{kind}' must contain an array of values"
                    )));
                };
                validate_filter_values(kind, values)?;
                if kind == "bpf" {
                    let Some(expression) = values.first().and_then(serde_json::Value::as_str)
                    else {
                        return Err(RuntimeError::Failed(
                            "BPF link filter must begin with a text expression".to_owned(),
                        ));
                    };
                    if expression.chars().any(char::is_control) {
                        return Err(RuntimeError::Failed(
                            "BPF link filters cannot contain control characters".to_owned(),
                        ));
                    }
                    let expression = expression.trim();
                    if !expression.is_empty() {
                        commands.push(format!(
                            "bridge add_packet_filter {bridge} filter{index} bpf \"{}\"",
                            escape_ubridge_arg(expression)
                        ));
                    }
                    index += 1;
                } else {
                    let arguments = values
                        .iter()
                        .map(filter_value)
                        .collect::<Result<Vec<_>, _>>()?
                        .join(" ");
                    commands.push(format!(
                        "bridge add_packet_filter {bridge} filter{index} {kind} {arguments}"
                    ));
                    index += 1;
                }
            }
        } else {
            return Err(RuntimeError::Failed(
                "Link filters must be an object".to_owned(),
            ));
        }
        // Validate the complete replacement before clearing the currently
        // working filters. uBridge applies rules one command at a time, so
        // keeping preparation separate prevents invalid updates from losing
        // the active set.
        self.command(&format!("bridge reset_packet_filters {bridge}"))
            .await?;
        for command in commands {
            self.command(&command).await?;
        }
        Ok(())
    }

    pub async fn start_link_capture(
        &self,
        endpoint_id: &str,
        capture_file: &str,
    ) -> Result<(), RuntimeError> {
        if capture_file.chars().any(char::is_control) {
            return Err(RuntimeError::Failed(
                "uBridge capture paths cannot contain control characters".to_owned(),
            ));
        }
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

    /// Release relays and stop the uBridge daemon owned by this controller.
    pub async fn shutdown(&self) {
        let endpoint_ids = self
            .link_bridges
            .lock()
            .await
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        if UnixStream::connect(&self.socket).await.is_ok() {
            for endpoint_id in endpoint_ids {
                if let Err(error) = self.delete_link_endpoint(&endpoint_id).await {
                    tracing::warn!(error = error.message(), %endpoint_id, "could not detach uBridge endpoint during shutdown");
                }
            }
        }
        self.link_bridges.lock().await.clear();
        self.host_bridge_taps.lock().await.clear();

        let mut daemon = self.daemon.lock().await;
        if let Some(mut child) = daemon.take() {
            if child.try_wait().ok().flatten().is_none() {
                if let Err(error) = child.start_kill() {
                    tracing::warn!(%error, "could not stop controller-owned uBridge daemon");
                }
                if let Err(error) = timeout(Duration::from_secs(3), child.wait()).await {
                    tracing::warn!(%error, "timed out waiting for controller-owned uBridge daemon");
                }
            }
            if let Err(error) = fs::remove_file(&self.socket)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(%error, socket = %self.socket.display(), "could not remove uBridge control socket");
            }
        }
    }

    async fn command(&self, command: &str) -> Result<String, RuntimeError> {
        if command.chars().any(char::is_control) {
            return Err(RuntimeError::Failed(
                "uBridge commands cannot contain control characters".to_owned(),
            ));
        }
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
        let mut daemon = self.daemon.lock().await;
        let owned_daemon_running = match daemon.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(None) => true,
                Ok(Some(_)) => {
                    daemon.take();
                    false
                }
                Err(error) => {
                    return Err(RuntimeError::Unavailable(format!(
                        "Could not inspect the uBridge process: {error}"
                    )));
                }
            },
            None => false,
        };
        if owned_daemon_running {
            if UnixStream::connect(&self.socket).await.is_ok() {
                return Ok(());
            }
            wait_for_socket(&self.socket).await?;
            return Ok(());
        }

        if UnixStream::connect(&self.socket).await.is_ok() {
            stop_existing_ubridge(&self.socket).await?;
            self.link_bridges.lock().await.clear();
            self.host_bridge_taps.lock().await.clear();
        }
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

/// Stop a uBridge process left behind by a previous controller instance. The
/// old process has no child handle here, so rebuilding requires asking the
/// daemon itself to stop before launching a fresh process.
async fn stop_existing_ubridge(socket: &PathBuf) -> Result<(), RuntimeError> {
    let mut stream = UnixStream::connect(socket).await.map_err(|error| {
        RuntimeError::Unavailable(format!(
            "Could not connect to the existing uBridge socket '{}': {error}",
            socket.display()
        ))
    })?;
    stream
        .write_all(b"hypervisor stop\n")
        .await
        .map_err(|error| {
            RuntimeError::Unavailable(format!(
                "Could not stop the existing uBridge daemon: {error}"
            ))
        })?;
    let _ = timeout(Duration::from_secs(2), async {
        let mut reader = BufReader::new(stream);
        let mut reply = String::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).await? == 0 {
                break;
            }
            reply.push_str(&line);
            if is_final_reply(&line) {
                break;
            }
        }
        Ok::<(), std::io::Error>(())
    })
    .await;

    let deadline = tokio::time::Instant::now() + START_TIMEOUT;
    loop {
        if UnixStream::connect(socket).await.is_err() {
            match fs::remove_file(socket) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(RuntimeError::Failed(format!(
                        "Could not remove stopped uBridge socket: {error}"
                    )));
                }
            }
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(RuntimeError::Unavailable(
                "The existing uBridge daemon did not stop during startup recovery".to_owned(),
            ));
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
        serde_json::Value::String(text) if !text.chars().any(char::is_control) => {
            Ok(format!("\"{}\"", escape_ubridge_arg(text)))
        }
        _ => Err(RuntimeError::Failed(
            "Link filter values must be numbers or single-line text".to_owned(),
        )),
    }
}

fn validate_filter_values(kind: &str, values: &[serde_json::Value]) -> Result<(), RuntimeError> {
    let invalid = || RuntimeError::Failed(format!("Link filter '{kind}' has invalid values"));
    let integer = |index: usize, min: i64, max: i64| {
        values
            .get(index)
            .and_then(|value| match value {
                serde_json::Value::Number(number) => number.as_i64(),
                serde_json::Value::String(text) => text.trim().parse::<i64>().ok(),
                _ => None,
            })
            .filter(|value| (min..=max).contains(value))
    };
    match kind {
        "frequency_drop" if values.len() == 1 && integer(0, -1, 32767).is_some() => Ok(()),
        "packet_loss" | "corrupt" if values.len() == 1 && integer(0, 0, 100).is_some() => Ok(()),
        "delay"
            if (1..=3).contains(&values.len())
                && integer(0, 1, 32767).is_some()
                && (values.len() < 2 || integer(1, 0, 32767).is_some())
                && (values.len() < 3
                    || values[2].as_str().is_some_and(|distribution| {
                        matches!(
                            distribution.trim().to_ascii_lowercase().as_str(),
                            "" | "uniform" | "normal" | "pareto" | "paretonormal"
                        )
                    })) =>
        {
            Ok(())
        }
        "bpf"
            if values.len() == 1
                && values[0].as_str().is_some_and(|expression| {
                    expression.len() <= 4096 && !expression.chars().any(char::is_control)
                }) =>
        {
            Ok(())
        }
        _ => Err(invalid()),
    }
}

fn escape_ubridge_arg(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_control() {
            escaped.push_str(&format!("%{:02X}", character as u32));
        } else {
            if matches!(character, '\\' | '"') {
                escaped.push('\\');
            }
            escaped.push(character);
        }
    }
    escaped
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
    use std::{env, fs, path::PathBuf, time::Duration};

    use tokio::{
        io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
        net::UnixListener,
        process::Command,
        sync::Mutex,
        time::timeout,
    };

    use super::{
        UbridgeRuntime, endpoint_name, escape_ubridge_arg, guest_veth_name, host_veth_name,
        is_final_reply, stop_existing_ubridge,
    };
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
    async fn ubridge_arguments_cannot_inject_a_second_command_line() {
        let runtime = UbridgeRuntime::with_paths("unused-ubridge", "/tmp/unused-ubridge.sock");
        let error = runtime
            .command("x\nhypervisor stop\n")
            .await
            .expect_err("control characters must be rejected before connecting");
        assert!(error.message().contains("control characters"));
        assert_eq!(
            escape_ubridge_arg("x\nhypervisor stop\n"),
            "x%0Ahypervisor stop%0A"
        );
    }

    #[tokio::test]
    async fn invalid_filter_replacement_does_not_reset_active_rules_and_frequency_drop_works() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("ubridge.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let commands = std::sync::Arc::new(Mutex::new(Vec::new()));
        let received = commands.clone();
        let server = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let (read, mut write) = stream.into_split();
                let mut reader = BufReader::new(read);
                let mut command = String::new();
                if reader.read_line(&mut command).await.unwrap_or_default() == 0 {
                    continue;
                }
                received.lock().await.push(command.trim().to_owned());
                let _ = write.write_all(b"100-OK\n").await;
            }
        });
        let runtime = UbridgeRuntime::with_paths("unused-ubridge", &socket);
        let child = Command::new("sleep")
            .arg("60")
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        *runtime.daemon.lock().await = Some(child);
        let endpoint_id = "01234567-89ab-cdef-0123-456789abcdef";
        runtime
            .link_bridges
            .lock()
            .await
            .insert(endpoint_id.to_owned(), endpoint_name(endpoint_id).unwrap());

        runtime
            .update_link_endpoint(
                endpoint_id,
                &serde_json::json!({"packet_loss": [25]}),
                false,
            )
            .await
            .expect("initial filter set is installed");
        let working_commands = commands.lock().await.clone();
        assert_eq!(working_commands.len(), 2);

        let invalid = runtime
            .update_link_endpoint(endpoint_id, &serde_json::json!({"unsupported": [1]}), false)
            .await;
        assert!(invalid.is_err());
        assert_eq!(*commands.lock().await, working_commands);

        runtime
            .update_link_endpoint(
                endpoint_id,
                &serde_json::json!({"frequency_drop": [3]}),
                false,
            )
            .await
            .expect("frequency_drop is a supported uBridge rule");
        assert_eq!(
            *commands.lock().await,
            vec![
                format!(
                    "bridge reset_packet_filters {}",
                    endpoint_name(endpoint_id).unwrap()
                ),
                format!(
                    "bridge add_packet_filter {} filter0 packet_loss 25",
                    endpoint_name(endpoint_id).unwrap()
                ),
                format!(
                    "bridge reset_packet_filters {}",
                    endpoint_name(endpoint_id).unwrap()
                ),
                format!(
                    "bridge add_packet_filter {} filter0 frequency_drop 3",
                    endpoint_name(endpoint_id).unwrap()
                )
            ]
        );

        runtime.shutdown().await;
        server.abort();
    }

    #[tokio::test]
    async fn persisted_udp_port_claims_are_idempotent_and_owner_scoped() {
        let runtime = UbridgeRuntime::with_paths("unused-ubridge", "/tmp/unused-ubridge.sock");
        let (allocated_a, allocated_b) = tokio::join!(
            runtime.reserve_udp_port_for("allocated-a"),
            runtime.reserve_udp_port_for("allocated-b")
        );
        let allocated_a = allocated_a.unwrap();
        let allocated_b = allocated_b.unwrap();
        assert_ne!(allocated_a, allocated_b);
        runtime
            .release_udp_ports("allocated-a", [allocated_a])
            .await;
        runtime
            .release_udp_ports("allocated-b", [allocated_b])
            .await;

        runtime.claim_udp_port("link-a", 39001).await.unwrap();
        runtime.claim_udp_port("link-a", 39001).await.unwrap();
        assert!(runtime.claim_udp_port("link-b", 39001).await.is_err());
        runtime.release_udp_ports("link-b", [39001]).await;
        assert!(runtime.claim_udp_port("link-b", 39001).await.is_err());
        runtime.release_udp_ports("link-a", [39001]).await;
        runtime.claim_udp_port("link-b", 39001).await.unwrap();
    }

    #[tokio::test]
    async fn startup_recovery_stops_a_surviving_unowned_daemon() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("ubridge.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut reader = BufReader::new(read);
            let mut command = String::new();
            reader.read_line(&mut command).await.unwrap();
            assert_eq!(command, "hypervisor stop\n");
            write.write_all(b"100-OK\n").await.unwrap();
        });

        stop_existing_ubridge(&socket).await.unwrap();
        server.await.unwrap();
        assert!(!socket.exists());
    }

    #[tokio::test]
    async fn shutdown_kills_owned_daemon_and_removes_its_socket() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("run/ubridge.sock");
        fs::create_dir_all(socket.parent().unwrap()).unwrap();
        fs::write(&socket, b"stale control socket").unwrap();
        let runtime = UbridgeRuntime::with_paths("unused-ubridge", &socket);
        let child = Command::new("sleep").arg("60").spawn().unwrap();
        let pid = child.id().unwrap();
        *runtime.daemon.lock().await = Some(child);

        runtime.shutdown().await;

        assert!(!socket.exists());
        assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
        assert!(runtime.daemon.lock().await.is_none());
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
