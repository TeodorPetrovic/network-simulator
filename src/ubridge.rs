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
    udp_endpoints: Arc<Mutex<HashMap<String, u16>>>,
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
            udp_endpoints: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[cfg(test)]
    pub fn with_paths(binary: impl Into<PathBuf>, socket: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            socket: socket.into(),
            daemon: Arc::new(Mutex::new(None)),
            udp_endpoints: Arc::new(Mutex::new(HashMap::new())),
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
                match self.create_segment(&probe).await {
                    Ok(segment) => match self.delete_segment(&segment).await {
                        Ok(()) => UbridgeRuntimeInfo {
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
                                "uBridge can create a kernel segment but could not remove its capability probe: {}",
                                error.message()
                            )),
                        },
                    },
                    Err(error) => UbridgeRuntimeInfo {
                        available: false,
                        binary: self.binary.display().to_string(),
                        version: Some(version),
                        error: Some(format!(
                            "uBridge {} is installed but cannot create Linux bridges; mixed-runtime Ethernet requires CAP_NET_ADMIN and CAP_NET_RAW: {}",
                            self.binary.display(),
                            error.message()
                        )),
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

    pub async fn create_segment(&self, link_id: &str) -> Result<String, RuntimeError> {
        let bridge = segment_name(link_id)?;
        self.command(&format!("brctl create {bridge}")).await?;
        if let Err(error) = self.set_interface_up(&bridge).await {
            let _ = self.delete_segment(&bridge).await;
            return Err(error);
        }
        Ok(bridge)
    }

    pub async fn create_tap(&self, tap_name: &str, segment: &str) -> Result<(), RuntimeError> {
        validate_interface_name(tap_name)?;
        validate_interface_name(segment)?;
        let uid = libc_uid();
        self.command(&format!("tap create {tap_name}")).await?;
        if let Err(error) = self
            .command(&format!("tap set_owner {tap_name} {uid}"))
            .await
        {
            let _ = self.command(&format!("tap delete {tap_name}")).await;
            return Err(error);
        }
        if let Err(error) = self
            .command(&format!("brctl addif {segment} {tap_name}"))
            .await
        {
            let _ = self.command(&format!("tap delete {tap_name}")).await;
            return Err(error);
        }
        Ok(())
    }

    pub async fn create_veth(
        &self,
        host: &str,
        guest: &str,
        segment: &str,
    ) -> Result<(), RuntimeError> {
        validate_interface_name(host)?;
        validate_interface_name(guest)?;
        validate_interface_name(segment)?;
        self.command(&format!("docker create_veth {host} {guest}"))
            .await?;
        if let Err(error) = self.command(&format!("brctl addif {segment} {host}")).await {
            let _ = self.command(&format!("docker delete_veth {host}")).await;
            return Err(error);
        }
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

    pub async fn set_interface_up(&self, interface: &str) -> Result<(), RuntimeError> {
        validate_interface_name(interface)?;
        self.command(&format!("link set {interface} up")).await?;
        Ok(())
    }

    pub async fn reserve_udp_port(&self) -> Result<u16, RuntimeError> {
        available_udp_port().await
    }

    pub async fn attach_udp_to_tap_at(
        &self,
        endpoint_id: &str,
        tap_name: &str,
        switch_port: u16,
        emulator_port: u16,
    ) -> Result<(), RuntimeError> {
        validate_interface_name(tap_name)?;
        let bridge = endpoint_name(endpoint_id)?;
        if let Some(existing_port) = self.udp_endpoints.lock().await.get(endpoint_id)
            && *existing_port == switch_port
        {
            return Ok(());
        }
        let mut created = false;
        let setup = async {
            self.command(&format!("bridge create {bridge}")).await?;
            created = true;
            self.command(&format!("bridge add_nio_tap {bridge} {tap_name}"))
                .await?;
            self.command(&format!(
                "bridge add_nio_udp {bridge} {switch_port} 127.0.0.1 {emulator_port}"
            ))
            .await?;
            self.command(&format!("bridge start {bridge}")).await?;
            Ok::<(), RuntimeError>(())
        }
        .await;
        if let Err(error) = setup {
            if created {
                let _ = self.command(&format!("bridge delete {bridge}")).await;
            }
            return Err(error);
        }
        self.udp_endpoints
            .lock()
            .await
            .insert(endpoint_id.to_owned(), switch_port);
        Ok(())
    }

    pub async fn delete_udp_tap_endpoint(&self, endpoint_id: &str) -> Result<(), RuntimeError> {
        let bridge = endpoint_name(endpoint_id)?;
        self.command(&format!("bridge delete {bridge}")).await?;
        self.udp_endpoints.lock().await.remove(endpoint_id);
        Ok(())
    }

    pub async fn delete_tap(&self, tap_name: &str, segment: &str) -> Result<(), RuntimeError> {
        validate_interface_name(tap_name)?;
        validate_interface_name(segment)?;
        let detach = self
            .command(&format!("brctl delif {segment} {tap_name}"))
            .await;
        let delete = self.command(&format!("tap delete {tap_name}")).await;
        if delete.is_ok() {
            return Ok(());
        }
        detach?;
        delete.map(|_| ())
    }

    pub async fn delete_veth(&self, host: &str) -> Result<(), RuntimeError> {
        validate_interface_name(host)?;
        self.command(&format!("docker delete_veth {host}")).await?;
        Ok(())
    }

    pub async fn delete_segment(&self, segment: &str) -> Result<(), RuntimeError> {
        validate_interface_name(segment)?;
        self.command(&format!("brctl delete {segment}")).await?;
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

pub fn segment_name(link_id: &str) -> Result<String, RuntimeError> {
    let compact = link_id
        .chars()
        .filter(|character| character.is_ascii_hexdigit())
        .collect::<String>();
    if compact.len() < 8 {
        return Err(RuntimeError::Failed(
            "Link ID must contain at least eight hexadecimal characters".to_owned(),
        ));
    }
    Ok(format!("nsb{}", &compact[..12.min(compact.len())]))
}

pub fn tap_name(endpoint_id: &str) -> Result<String, RuntimeError> {
    Ok(format!("nst{}", compact_id(endpoint_id)?))
}

pub fn host_veth_name(endpoint_id: &str) -> Result<String, RuntimeError> {
    Ok(format!("nsv{}", compact_id(endpoint_id)?))
}

pub fn guest_veth_name(endpoint_id: &str) -> Result<String, RuntimeError> {
    Ok(format!("nsg{}", compact_id(endpoint_id)?))
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

fn libc_uid() -> u32 {
    unsafe { libc::geteuid() }
}

#[cfg(test)]
mod tests {
    use std::{env, path::PathBuf, time::Duration};

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        process::Command,
        time::timeout,
    };

    use super::{
        UbridgeRuntime, compact_id, endpoint_name, guest_veth_name, host_veth_name, is_final_reply,
        segment_name, tap_name,
    };
    use crate::{
        runtime::{DockerRuntime, NodeAction, RuntimeError},
        vpcs::VpcsRuntime,
    };

    #[test]
    fn generates_linux_names_that_fit_interface_limits() {
        let id = "01234567-89ab-cdef-0123-456789abcdef";
        for name in [
            segment_name(id).unwrap(),
            tap_name(id).unwrap(),
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
        assert_eq!(compact_id(id).unwrap(), "0123456789ab");
    }

    #[test]
    fn recognizes_the_final_line_in_the_upstream_control_protocol() {
        assert!(!is_final_reply("101 bridge is running\n"));
        assert!(is_final_reply("100-bridge started\n"));
        assert!(is_final_reply("206-Could not create bridge\n"));
    }

    #[tokio::test]
    #[ignore = "requires Docker, alpine:latest, upstream VPCS, and uBridge CAP_NET_ADMIN/CAP_NET_RAW"]
    async fn alpine_and_vpcs_exchange_icmp_over_a_real_ubridge_segment() {
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
        let link_id = uuid::Uuid::new_v4().to_string();
        let vpcs_endpoint_id = uuid::Uuid::new_v4().to_string();
        let docker_endpoint_id = uuid::Uuid::new_v4().to_string();
        let segment_name = segment_name(&link_id).expect("stable Linux bridge name");
        let vpcs_tap = tap_name(&vpcs_endpoint_id).expect("stable VPCS TAP name");
        let docker_host = host_veth_name(&docker_endpoint_id).expect("stable Docker veth name");
        let docker_guest = guest_veth_name(&docker_endpoint_id).expect("stable Docker peer name");
        let vpcs_switch_port = ubridge.reserve_udp_port().await.expect("VPCS UDP port");
        let mut container_id = None;
        let mut segment_created = false;
        let mut tap_created = false;
        let mut veth_created = false;
        let mut vpcs_started = false;
        let mut udp_bridge_created = false;

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
            let segment = ubridge.create_segment(&link_id).await?;
            segment_created = true;
            ubridge.create_tap(&vpcs_tap, &segment).await?;
            tap_created = true;

            vpcs.set_link(&link_id, vec![vpcs_id.clone()]).await;
            vpcs.set_bridge_port(&vpcs_id, vpcs_switch_port).await;
            vpcs.lifecycle(
                &project_id,
                &vpcs_id,
                "uBridge VPCS peer",
                Some("ip 10.0.0.1/24"),
                NodeAction::Start,
            )
            .await?;
            vpcs_started = true;
            let vpcs_local_port = vpcs.active_udp_port(&vpcs_id).await.ok_or_else(|| {
                RuntimeError::Failed("VPCS did not allocate its Ethernet UDP port".to_owned())
            })?;
            ubridge
                .attach_udp_to_tap_at(
                    &vpcs_endpoint_id,
                    &vpcs_tap,
                    vpcs_switch_port,
                    vpcs_local_port,
                )
                .await?;
            udp_bridge_created = true;

            docker
                .lifecycle(&created_container, NodeAction::Start)
                .await?;
            let namespace_pid = docker.container_pid(&created_container).await?;
            ubridge
                .create_veth(&docker_host, &docker_guest, &segment)
                .await?;
            veth_created = true;
            ubridge.set_interface_up(&docker_guest).await?;
            ubridge
                .move_veth_to_container(&docker_guest, namespace_pid, "eth1")
                .await?;

            let docker_binary = env::var_os("NETWORK_SIMULATOR_DOCKER")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("docker"));
            let output = Command::new(docker_binary)
                .args([
                    "exec",
                    &created_container,
                    "ip",
                    "addr",
                    "add",
                    "10.0.0.2/24",
                    "dev",
                    "eth1",
                ])
                .output()
                .await
                .map_err(|error| {
                    RuntimeError::Unavailable(format!(
                        "Could not configure the Alpine test interface: {error}"
                    ))
                })?;
            if !output.status.success() {
                return Err(RuntimeError::Failed(format!(
                    "Could not configure Alpine eth1: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
            }
            let output = Command::new(
                env::var_os("NETWORK_SIMULATOR_DOCKER")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("docker")),
            )
            .args([
                "exec",
                &created_container,
                "ip",
                "link",
                "set",
                "eth1",
                "up",
            ])
            .output()
            .await
            .map_err(|error| {
                RuntimeError::Unavailable(format!("Could not bring up Alpine eth1: {error}"))
            })?;
            if !output.status.success() {
                return Err(RuntimeError::Failed(format!(
                    "Could not bring up Alpine eth1: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
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
            let ping_result = timeout(Duration::from_secs(15), async {
                loop {
                    let mut chunk = [0_u8; 4096];
                    let length = console.read(&mut chunk).await?;
                    ping_output.extend_from_slice(&chunk[..length]);
                    if String::from_utf8_lossy(&ping_output).contains("bytes from 10.0.0.2") {
                        break Ok::<(), std::io::Error>(());
                    }
                }
            })
            .await;
            if ping_result.is_err() {
                let bridge = endpoint_name(&vpcs_endpoint_id)
                    .expect("the test endpoint ID is valid");
                let stats = ubridge
                    .command(&format!("bridge get_stats {bridge}"))
                    .await
                    .unwrap_or_else(|error| error.message().to_owned());
                let mut interfaces = String::new();
                for interface in [&segment_name, &vpcs_tap, &docker_host] {
                    let output = Command::new("ip")
                        .args(["-s", "link", "show", "dev", interface])
                        .output()
                        .await;
                    interfaces.push_str(&format!(
                        "{}: {}\n",
                        interface,
                        output
                            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                            .unwrap_or_else(|error| error.to_string())
                    ));
                }
                let docker_binary = env::var_os("NETWORK_SIMULATOR_DOCKER")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("docker"));
                let guest_interface = Command::new(docker_binary)
                    .args([
                        "exec",
                        container_id.as_deref().unwrap_or_default(),
                        "ip",
                        "-s",
                        "link",
                        "show",
                        "dev",
                        "eth1",
                    ])
                    .output()
                    .await
                    .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                    .unwrap_or_else(|error| error.to_string());
                return Err(RuntimeError::Unavailable(format!(
                    "No ICMP reply from Alpine over uBridge: {}; bridge counters: {stats}; host interfaces: {interfaces}; Alpine eth1: {guest_interface}",
                    String::from_utf8_lossy(&ping_output).trim(),
                )));
            }
            ping_result
                .expect("timeout was checked above")
                .map_err(|error| RuntimeError::Unavailable(format!("VPCS console failed: {error}")))?;
            Ok(())
        }
        .await;

        if udp_bridge_created {
            let _ = ubridge.delete_udp_tap_endpoint(&vpcs_endpoint_id).await;
        }
        if vpcs_started {
            let _ = vpcs.stop(&vpcs_id).await;
        }
        if veth_created {
            let _ = ubridge.delete_veth(&docker_host).await;
        }
        if let Some(container_id) = container_id.as_deref() {
            let _ = docker
                .remove_node_resources(&docker_node_id, Some(container_id))
                .await;
        }
        if tap_created {
            let _ = ubridge.delete_tap(&vpcs_tap, &segment_name).await;
        }
        if segment_created {
            let _ = ubridge.delete_segment(&segment_name).await;
        }
        vpcs.remove_link(&link_id).await;
        if let Err(error) = run {
            panic!(
                "real Alpine-to-VPCS uBridge test failed: {}",
                error.message()
            );
        }
    }
}
