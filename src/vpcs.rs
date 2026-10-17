use std::{
    collections::{HashMap, HashSet},
    fs,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use serde::Serialize;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UdpSocket},
    process::{Child, Command},
    sync::{Mutex, RwLock},
    task::JoinHandle,
    time::{sleep, timeout},
};
use uuid::Uuid;

use crate::runtime::{NodeAction, RuntimeError};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const START_TIMEOUT: Duration = Duration::from_secs(8);
const TELNET_HANDSHAKE: [u8; 12] = [255, 253, 1, 255, 251, 1, 255, 253, 3, 255, 251, 3];
const TELNET_RESPONSE: [u8; 12] = [255, 251, 1, 255, 253, 1, 255, 251, 3, 255, 253, 3];

#[derive(Clone)]
pub struct VpcsRuntime {
    binary: PathBuf,
    state_dir: PathBuf,
    processes: Arc<Mutex<HashMap<String, ManagedVpcs>>>,
    links: Arc<RwLock<HashMap<String, Vec<String>>>>,
    active_ports: Arc<RwLock<HashMap<String, u16>>>,
    bridge_ports: Arc<RwLock<HashMap<String, u16>>>,
    external_ports: Arc<RwLock<HashMap<String, HashSet<u16>>>>,
    segment_switches: Arc<Mutex<HashMap<String, ManagedSegmentSwitch>>>,
}

struct ManagedVpcs {
    child: Child,
    console: SocketAddr,
}

struct ManagedSegmentSwitch {
    port: u16,
    task: JoinHandle<()>,
}

impl Drop for ManagedSegmentSwitch {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Serialize)]
pub struct VpcsRuntimeInfo {
    pub available: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

impl VpcsRuntime {
    pub fn new() -> Self {
        Self {
            binary: crate::image_paths::runtime_binary("NETWORK_SIMULATOR_VPCS", "vpcs"),
            state_dir: std::env::var_os("NETWORK_SIMULATOR_VPCS_STATE")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("data/vpcs")),
            processes: Arc::new(Mutex::new(HashMap::new())),
            links: Arc::new(RwLock::new(HashMap::new())),
            active_ports: Arc::new(RwLock::new(HashMap::new())),
            bridge_ports: Arc::new(RwLock::new(HashMap::new())),
            external_ports: Arc::new(RwLock::new(HashMap::new())),
            segment_switches: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[cfg(test)]
    pub fn with_paths(binary: impl Into<PathBuf>, state_dir: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            state_dir: state_dir.into(),
            processes: Arc::new(Mutex::new(HashMap::new())),
            links: Arc::new(RwLock::new(HashMap::new())),
            active_ports: Arc::new(RwLock::new(HashMap::new())),
            bridge_ports: Arc::new(RwLock::new(HashMap::new())),
            external_ports: Arc::new(RwLock::new(HashMap::new())),
            segment_switches: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn info(&self) -> VpcsRuntimeInfo {
        match self.version().await {
            Ok(version) => VpcsRuntimeInfo {
                available: true,
                version: Some(version),
                error: None,
            },
            Err(error) => VpcsRuntimeInfo {
                available: false,
                version: None,
                error: Some(error.message().to_owned()),
            },
        }
    }

    pub async fn templates(&self) -> Result<Vec<Value>, RuntimeError> {
        self.version().await?;
        let template_id = Uuid::new_v5(&Uuid::NAMESPACE_URL, b"network-simulator/vpcs");
        Ok(vec![json!({
            "template_id": template_id,
            "name": "VPCS",
            "template_type": "vpcs",
            "compute_id": "local",
            "category": "guest",
            "symbol": "vpcs_guest",
            "builtin": true,
            "adapters": 1,
            "memory": 0,
            "cpus": 0,
            "startup_script": ""
        })])
    }

    pub async fn ensure_available(&self) -> Result<String, RuntimeError> {
        self.version().await
    }

    pub async fn set_link(&self, link_id: &str, node_ids: Vec<String>) {
        self.links
            .write()
            .await
            .insert(link_id.to_owned(), node_ids);
    }

    pub async fn set_bridge_port(&self, node_id: &str, port: u16) {
        self.bridge_ports
            .write()
            .await
            .insert(node_id.to_owned(), port);
    }

    pub async fn remove_bridge_port(&self, node_id: &str) {
        self.bridge_ports.write().await.remove(node_id);
    }

    pub async fn active_udp_port(&self, node_id: &str) -> Option<u16> {
        self.active_ports.read().await.get(node_id).copied()
    }

    pub async fn create_segment(
        &self,
        link_id: &str,
        node_ids: Vec<String>,
    ) -> Result<u16, RuntimeError> {
        self.set_link(link_id, node_ids).await;
        self.ensure_segment_switch(link_id).await
    }

    pub async fn register_external_endpoint(&self, node_id: &str, udp_port: u16) {
        self.external_ports
            .write()
            .await
            .entry(node_id.to_owned())
            .or_default()
            .insert(udp_port);
    }

    pub async fn remove_external_endpoint(&self, node_id: &str, udp_port: Option<u16>) {
        let mut endpoints = self.external_ports.write().await;
        if let Some(ports) = endpoints.get_mut(node_id) {
            if let Some(udp_port) = udp_port {
                ports.remove(&udp_port);
            } else {
                ports.clear();
            }
            if ports.is_empty() {
                endpoints.remove(node_id);
            }
        }
    }

    pub async fn remove_link(&self, link_id: &str) {
        self.links.write().await.remove(link_id);
        self.segment_switches.lock().await.remove(link_id);
    }

    pub async fn lifecycle(
        &self,
        project_id: &str,
        node_id: &str,
        name: &str,
        startup_script: Option<&str>,
        action: NodeAction,
    ) -> Result<String, RuntimeError> {
        match action {
            NodeAction::Start => self.start(project_id, node_id, name, startup_script).await,
            NodeAction::Stop => self.stop(node_id).await,
            NodeAction::Suspend => Err(RuntimeError::Failed(
                "VPCS does not support process suspension".to_owned(),
            )),
            NodeAction::Reload => {
                self.stop(node_id).await?;
                self.start(project_id, node_id, name, startup_script).await
            }
        }
    }

    pub async fn status(&self, node_id: &str) -> Result<String, RuntimeError> {
        let mut processes = self.processes.lock().await;
        let Some(process) = processes.get_mut(node_id) else {
            return Ok("stopped".to_owned());
        };
        let exited = process.child.try_wait().map_err(|error| {
            RuntimeError::Unavailable(format!("Could not inspect VPCS process: {error}"))
        })?;
        if exited.is_some() {
            processes
                .remove(node_id)
                .expect("the exited VPCS process remains present");
            self.active_ports.write().await.remove(node_id);
            return Ok("stopped".to_owned());
        }
        Ok("started".to_owned())
    }

    pub async fn console_address(&self, node_id: &str) -> Result<SocketAddr, RuntimeError> {
        let processes = self.processes.lock().await;
        let process = processes.get(node_id).ok_or_else(|| {
            RuntimeError::Failed("Start the VPCS node before opening its console".to_owned())
        })?;
        Ok(process.console)
    }

    pub async fn connect_console(&self, node_id: &str) -> Result<TcpStream, RuntimeError> {
        let address = self.console_address(node_id).await?;
        connect_vpcs_console(address).await
    }

    pub async fn stop(&self, node_id: &str) -> Result<String, RuntimeError> {
        let process = self.processes.lock().await.remove(node_id);
        let Some(mut process) = process else {
            self.active_ports.write().await.remove(node_id);
            return Ok("stopped".to_owned());
        };
        if let Err(error) = stop_vpcs_process(&mut process).await {
            self.processes
                .lock()
                .await
                .insert(node_id.to_owned(), process);
            return Err(error);
        }
        self.active_ports.write().await.remove(node_id);
        Ok("stopped".to_owned())
    }

    pub async fn remove_node(&self, project_id: &str, node_id: &str) -> Result<(), RuntimeError> {
        self.stop(node_id).await?;
        self.remove_external_endpoint(node_id, None).await;
        let project_id = parse_id(project_id)?;
        let node_id = parse_id(node_id)?;
        let node_dir = self
            .state_dir
            .join(project_id.to_string())
            .join(node_id.to_string());
        if node_dir.exists() {
            fs::remove_dir_all(&node_dir).map_err(|error| {
                RuntimeError::Failed(format!("Could not remove VPCS node files: {error}"))
            })?;
        }
        Ok(())
    }

    async fn start(
        &self,
        project_id: &str,
        node_id: &str,
        name: &str,
        startup_script: Option<&str>,
    ) -> Result<String, RuntimeError> {
        let project_id = parse_id(project_id)?;
        let node_id_uuid = parse_id(node_id)?;
        self.version().await?;

        if self.status(node_id).await? == "started" {
            return Ok("started".to_owned());
        }

        let mut processes = self.processes.lock().await;
        if processes.contains_key(node_id) {
            return Ok("started".to_owned());
        }

        let node_links = {
            let links = self.links.read().await;
            links
                .iter()
                .filter(|(_, members)| members.iter().any(|member| member == node_id))
                .map(|(link_id, _)| link_id.clone())
                .collect::<Vec<_>>()
        };
        if node_links.len() > 1 {
            return Err(RuntimeError::Failed(
                "VPCS has one Ethernet adapter and cannot join multiple segments".to_owned(),
            ));
        }
        let switch_port = if let Some(link_id) = node_links.first() {
            if let Some(port) = self.bridge_ports.read().await.get(node_id).copied() {
                port
            } else {
                self.ensure_segment_switch(link_id).await?
            }
        } else {
            available_udp_port().await?
        };

        let node_dir = self
            .state_dir
            .join(project_id.to_string())
            .join(node_id_uuid.to_string());
        fs::create_dir_all(&node_dir).map_err(|error| {
            RuntimeError::Failed(format!("Could not create VPCS node directory: {error}"))
        })?;

        let script_path =
            if let Some(script) = startup_script.filter(|script| !script.trim().is_empty()) {
                let path = node_dir.join("startup.vpc");
                let script = script.replace("%h", name);
                fs::write(&path, script).map_err(|error| {
                    RuntimeError::Failed(format!("Could not write VPCS startup script: {error}"))
                })?;
                Some(path)
            } else {
                None
            };

        let console_port = available_tcp_port().await?;
        let local_udp_port = available_udp_port().await?;
        let log_path = node_dir.join("vpcs.log");
        let log_file = fs::File::create(&log_path).map_err(|error| {
            RuntimeError::Failed(format!("Could not create VPCS log file: {error}"))
        })?;
        let stderr = log_file.try_clone().map_err(|error| {
            RuntimeError::Failed(format!("Could not open VPCS log file: {error}"))
        })?;
        self.active_ports
            .write()
            .await
            .insert(node_id.to_owned(), local_udp_port);

        let mut command = Command::new(&self.binary);
        command
            .args([
                "-p".to_owned(),
                console_port.to_string(),
                "-m".to_owned(),
                mac_offset(&node_id_uuid).to_string(),
                "-i".to_owned(),
                "1".to_owned(),
                "-F".to_owned(),
                "-R".to_owned(),
                "-s".to_owned(),
                local_udp_port.to_string(),
                "-c".to_owned(),
                switch_port.to_string(),
                "-t".to_owned(),
                "127.0.0.1".to_owned(),
            ])
            .current_dir(&node_dir)
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(stderr))
            .stdin(Stdio::null())
            .kill_on_drop(true);
        if let Some(script_path) = script_path.as_ref() {
            command.arg(script_path.file_name().unwrap_or_default());
        }
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                self.active_ports.write().await.remove(node_id);
                return Err(RuntimeError::Unavailable(format!(
                    "Could not start VPCS executable '{}': {error}",
                    self.binary.display()
                )));
            }
        };

        let console = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, console_port));
        let deadline = tokio::time::Instant::now() + START_TIMEOUT;
        loop {
            if connect_vpcs_console(console).await.is_ok() {
                processes.insert(node_id.to_owned(), ManagedVpcs { child, console });
                return Ok("started".to_owned());
            }
            if let Some(status) = child.try_wait().map_err(|error| {
                RuntimeError::Unavailable(format!("Could not inspect VPCS startup: {error}"))
            })? {
                self.active_ports.write().await.remove(node_id);
                let log = fs::read_to_string(&log_path).unwrap_or_default();
                return Err(RuntimeError::Failed(format!(
                    "VPCS exited during startup ({status}){}",
                    if log.trim().is_empty() {
                        String::new()
                    } else {
                        format!(": {}", log.trim())
                    }
                )));
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = child.start_kill();
                let _ = child.wait().await;
                self.active_ports.write().await.remove(node_id);
                let log = fs::read_to_string(&log_path).unwrap_or_default();
                return Err(RuntimeError::Unavailable(format!(
                    "VPCS console did not become ready{}",
                    if log.trim().is_empty() {
                        String::new()
                    } else {
                        format!(": {}", log.trim())
                    }
                )));
            }
            sleep(Duration::from_millis(50)).await;
        }
    }

    async fn ensure_segment_switch(&self, link_id: &str) -> Result<u16, RuntimeError> {
        let mut switches = self.segment_switches.lock().await;
        if let Some(switch) = switches.get(link_id) {
            return Ok(switch.port);
        }
        let socket = Arc::new(
            UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
                .await
                .map_err(|error| {
                    RuntimeError::Unavailable(format!(
                        "Could not create Ethernet segment switch: {error}"
                    ))
                })?,
        );
        let port = socket
            .local_addr()
            .map_err(|error| {
                RuntimeError::Unavailable(format!(
                    "Could not read Ethernet segment switch port: {error}"
                ))
            })?
            .port();
        let links = Arc::clone(&self.links);
        let active_ports = Arc::clone(&self.active_ports);
        let external_ports = Arc::clone(&self.external_ports);
        let switch_link_id = link_id.to_owned();
        let task = tokio::spawn(async move {
            let mut frame = vec![0_u8; 65_536];
            loop {
                let Ok((length, source)) = socket.recv_from(&mut frame).await else {
                    return;
                };
                let members = {
                    let links = links.read().await;
                    links.get(&switch_link_id).cloned().unwrap_or_default()
                };
                let source_port = source.port();
                let source_node = {
                    let ports = active_ports.read().await;
                    let external = external_ports.read().await;
                    members
                        .iter()
                        .find(|member| {
                            ports.get(*member) == Some(&source_port)
                                || external
                                    .get(*member)
                                    .is_some_and(|member_ports| member_ports.contains(&source_port))
                        })
                        .cloned()
                };
                let mut destinations = HashSet::new();
                {
                    let ports = active_ports.read().await;
                    for member in &members {
                        if source_node.as_ref() == Some(member) {
                            continue;
                        }
                        if let Some(port) = ports.get(member) {
                            destinations.insert(*port);
                        }
                    }
                }
                {
                    let ports = external_ports.read().await;
                    for member in &members {
                        if source_node.as_ref() == Some(member) {
                            continue;
                        }
                        if let Some(member_ports) = ports.get(member) {
                            destinations.extend(member_ports.iter().copied());
                        }
                    }
                }
                for destination_port in destinations {
                    if destination_port == source_port {
                        continue;
                    }
                    let destination = SocketAddrV4::new(Ipv4Addr::LOCALHOST, destination_port);
                    if let Err(error) = socket.send_to(&frame[..length], destination).await {
                        tracing::debug!(%error, %switch_link_id, destination_port, "could not forward Ethernet frame");
                    }
                }
            }
        });
        switches.insert(link_id.to_owned(), ManagedSegmentSwitch { port, task });
        Ok(port)
    }

    async fn version(&self) -> Result<String, RuntimeError> {
        let output = timeout(
            COMMAND_TIMEOUT,
            Command::new(&self.binary)
                .arg("-v")
                .stdin(Stdio::null())
                .output(),
        )
        .await
        .map_err(|_| RuntimeError::Unavailable("VPCS version check timed out".to_owned()))?
        .map_err(|error| {
            RuntimeError::Unavailable(format!(
                "Could not run VPCS executable '{}': {error}",
                self.binary.display()
            ))
        })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(RuntimeError::Failed(if stderr.is_empty() {
                format!("VPCS version check exited with {}", output.status)
            } else {
                stderr
            }));
        }
        let output = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let line = output.lines().find(|line| line.contains("version"));
        line.map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| RuntimeError::Failed("Could not determine the VPCS version".to_owned()))
    }
}

async fn connect_vpcs_console(address: SocketAddr) -> Result<TcpStream, RuntimeError> {
    timeout(Duration::from_secs(3), async {
        let mut stream = TcpStream::connect(address).await.map_err(|error| {
            RuntimeError::Unavailable(format!("Could not connect to the VPCS console: {error}"))
        })?;
        let mut handshake = [0_u8; TELNET_HANDSHAKE.len()];
        stream.read_exact(&mut handshake).await.map_err(|error| {
            RuntimeError::Unavailable(format!(
                "Could not negotiate the VPCS Telnet console: {error}"
            ))
        })?;
        if handshake != TELNET_HANDSHAKE {
            return Err(RuntimeError::Failed(
                "VPCS sent an unsupported Telnet console handshake".to_owned(),
            ));
        }
        stream.write_all(&TELNET_RESPONSE).await.map_err(|error| {
            RuntimeError::Unavailable(format!(
                "Could not answer the VPCS Telnet handshake: {error}"
            ))
        })?;
        Ok(stream)
    })
    .await
    .map_err(|_| RuntimeError::Unavailable("VPCS Telnet console handshake timed out".to_owned()))?
}

async fn stop_vpcs_process(process: &mut ManagedVpcs) -> Result<(), RuntimeError> {
    if process
        .child
        .try_wait()
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not inspect VPCS process: {error}"))
        })?
        .is_some()
    {
        return Ok(());
    }

    // Upstream VPCS forks its PC process behind the console daemon. Its `quit`
    // command notifies that daemon to stop the forked process as well; killing
    // only the tracked daemon PID leaves the emulator child orphaned.
    if let Ok(mut console) = connect_vpcs_console(process.console).await {
        let _ = console.write_all(b"quit\r").await;
    }
    if wait_vpcs_process(&mut process.child).await? {
        return Ok(());
    }

    // Match VPCS's own daemon shutdown path if the console command could not
    // complete (for example, a console client already owns the connection).
    signal_vpcs_process(&process.child, libc::SIGUSR1)?;
    if wait_vpcs_process(&mut process.child).await? {
        return Ok(());
    }
    signal_vpcs_process(&process.child, libc::SIGTERM)?;
    if wait_vpcs_process(&mut process.child).await? {
        return Ok(());
    }
    Err(RuntimeError::Failed(
        "VPCS daemon did not stop after its console quit command and shutdown signals".to_owned(),
    ))
}

async fn wait_vpcs_process(child: &mut Child) -> Result<bool, RuntimeError> {
    match timeout(Duration::from_secs(3), child.wait()).await {
        Ok(Ok(_)) => Ok(true),
        Ok(Err(error)) => Err(RuntimeError::Unavailable(format!(
            "Could not wait for the VPCS process: {error}"
        ))),
        Err(_) => Ok(false),
    }
}

fn signal_vpcs_process(child: &Child, signal: libc::c_int) -> Result<(), RuntimeError> {
    let Some(pid) = child.id() else {
        return Ok(());
    };
    // SAFETY: The PID is owned and tracked by this child handle.
    if unsafe { libc::kill(pid as libc::pid_t, signal) } == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(RuntimeError::Failed(format!(
            "Could not signal the VPCS daemon: {error}"
        )))
    }
}

impl Default for VpcsRuntime {
    fn default() -> Self {
        Self::new()
    }
}

async fn available_udp_port() -> Result<u16, RuntimeError> {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not allocate a UDP port: {error}"))
        })?;
    socket
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not read an allocated UDP port: {error}"))
        })
}

async fn available_tcp_port() -> Result<u16, RuntimeError> {
    let listener = tokio::net::TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not allocate a TCP port: {error}"))
        })?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not read an allocated TCP port: {error}"))
        })
}

fn parse_id(id: &str) -> Result<Uuid, RuntimeError> {
    Uuid::parse_str(id).map_err(|_| RuntimeError::Failed("VPCS IDs must be valid UUIDs".to_owned()))
}

fn mac_offset(node_id: &Uuid) -> u8 {
    node_id.as_bytes()[15]
}

#[cfg(test)]
mod tests {
    use std::{env, path::PathBuf, time::Duration};

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        time::timeout,
    };
    use uuid::Uuid;

    use super::{VpcsRuntime, connect_vpcs_console, mac_offset};

    #[test]
    fn derives_mac_offset_from_the_node_identifier() {
        let id = Uuid::parse_str("00000000-0000-0000-0000-00000000002a").unwrap();
        assert_eq!(mac_offset(&id), 42);
    }

    #[tokio::test]
    #[ignore = "requires the upstream VPCS executable; set NETWORK_SIMULATOR_TEST_VPCS and run with --ignored"]
    async fn upstream_vpcs_nodes_exchange_ethernet_frames_over_a_shared_segment() {
        let binary = env::var_os("NETWORK_SIMULATOR_TEST_VPCS")
            .expect("NETWORK_SIMULATOR_TEST_VPCS is set for this integration test");
        let directory = tempfile::tempdir().expect("temporary VPCS state directory");
        let runtime = VpcsRuntime::with_paths(PathBuf::from(binary), directory.path());
        runtime
            .ensure_available()
            .await
            .expect("upstream VPCS is available");

        let project_id = "00000000-0000-0000-0000-000000000001";
        let node_a = "00000000-0000-0000-0000-00000000000a";
        let node_b = "00000000-0000-0000-0000-00000000000b";
        runtime
            .set_link(
                "00000000-0000-0000-0000-0000000000ff",
                vec![node_a.into(), node_b.into()],
            )
            .await;
        runtime
            .lifecycle(
                project_id,
                node_a,
                "PC A",
                Some("ip 10.0.0.1/24"),
                crate::runtime::NodeAction::Start,
            )
            .await
            .expect("first VPCS starts");
        runtime
            .lifecycle(
                project_id,
                node_b,
                "PC B",
                Some("ip 10.0.0.2/24"),
                crate::runtime::NodeAction::Start,
            )
            .await
            .expect("second VPCS starts");

        let address = runtime
            .console_address(node_a)
            .await
            .expect("VPCS console address");
        let mut console = connect_vpcs_console(address)
            .await
            .expect("VPCS Telnet console connects");
        let mut output = Vec::new();
        timeout(Duration::from_secs(5), async {
            loop {
                let mut chunk = [0_u8; 4096];
                let length = console
                    .read(&mut chunk)
                    .await
                    .expect("VPCS console is readable");
                output.extend_from_slice(&chunk[..length]);
                if String::from_utf8_lossy(&output).contains("VPCS>") {
                    break;
                }
            }
        })
        .await
        .expect("VPCS startup script reaches the interactive prompt");
        output.clear();
        console
            .write_all(b"ping 10.0.0.2 -c 1\r")
            .await
            .expect("ping command reaches VPCS");

        let ping_finished = timeout(Duration::from_secs(12), async {
            loop {
                let mut chunk = [0_u8; 4096];
                let length = console
                    .read(&mut chunk)
                    .await
                    .expect("VPCS console remains open");
                if length == 0 {
                    break;
                }
                output.extend_from_slice(&chunk[..length]);
                if String::from_utf8_lossy(&output).contains("bytes from 10.0.0.2") {
                    break;
                }
            }
        })
        .await;
        let output = String::from_utf8_lossy(&output);
        assert!(
            ping_finished.is_ok(),
            "VPCS did not return ping output: {output}"
        );
        assert!(
            output.contains("bytes from 10.0.0.2"),
            "unexpected VPCS console output: {output}"
        );

        drop(console);
        runtime.stop(node_a).await.expect("first VPCS stops");
        runtime.stop(node_b).await.expect("second VPCS stops");
        assert!(
            connect_vpcs_console(address).await.is_err(),
            "stopped VPCS console should no longer accept connections"
        );
    }
}
