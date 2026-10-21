use std::{
    collections::HashMap,
    fs,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4},
    path::{Path, PathBuf},
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
    active_ports: Arc<RwLock<HashMap<String, u16>>>,
    bridge_ports: Arc<RwLock<HashMap<String, u16>>>,
    requested_runtime_ports: Arc<RwLock<HashMap<String, u16>>>,
}

struct ManagedVpcs {
    child: Child,
    console: SocketAddr,
}

impl ManagedVpcs {
    /// Kill every process in this VPCS's own process group. VPCS forks its PC
    /// process behind the tracked console daemon, and killing only the daemon
    /// (as `kill_on_drop` does) leaves the fork running forever.
    fn kill_group(&self) {
        if let Some(pid) = self.child.id() {
            kill_process_group(pid);
        }
    }
}

impl Drop for ManagedVpcs {
    fn drop(&mut self) {
        self.kill_group();
    }
}

/// SIGKILL `pid`, its process group, and every descendant. The real VPCS
/// daemon forks its PC process into a *new session*, so the process group alone
/// does not reach it; the descendants are collected before anything is killed
/// because a child is re-parented (and untraceable) once its parent dies.
fn kill_process_group(pid: u32) {
    let mut victims = descendants_of(pid);
    victims.push(pid);
    // SAFETY: every PID was read from /proc as a descendant of the VPCS daemon
    // this runtime spawned, and killpg targets the group it created.
    unsafe {
        libc::killpg(pid as libc::pid_t, libc::SIGKILL);
        for victim in victims {
            libc::kill(victim as libc::pid_t, libc::SIGKILL);
        }
    }
}

/// All transitive children of `root`, read from `/proc/*/stat`.
fn descendants_of(root: u32) -> Vec<u32> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|n| n.parse::<u32>().ok())
            else {
                continue;
            };
            let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
                continue;
            };
            // The command name may contain spaces and parentheses; the
            // parent PID is the second field after the last ')'.
            let Some(rest) = stat.rfind(')').map(|index| &stat[index + 1..]) else {
                continue;
            };
            if let Some(parent) = rest.split_whitespace().nth(1).and_then(|f| f.parse().ok()) {
                children.entry(parent).or_default().push(pid);
            }
        }
    }
    let mut found = Vec::new();
    let mut queue = vec![root];
    while let Some(parent) = queue.pop() {
        for child in children.get(&parent).into_iter().flatten() {
            found.push(*child);
            queue.push(*child);
        }
    }
    found
}

/// Kill leftover VPCS processes whose working directory lies inside
/// `state_dir`. Run once at controller start-up: a previous controller that
/// crashed cannot have stopped them, and this controller keeps no handle to
/// them. Returns the number of processes killed.
pub fn reap_orphaned_processes(binary: &Path, state_dir: &Path) -> usize {
    let Ok(state_dir) = state_dir.canonicalize() else {
        return 0;
    };
    let binary_name = binary.file_name().map(|name| name.to_owned());
    let Ok(entries) = fs::read_dir("/proc") else {
        return 0;
    };
    let mut killed = 0;
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        let exe_name = fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .and_then(|path| path.file_name().map(|name| name.to_owned()));
        if exe_name.is_none() || exe_name != binary_name {
            continue;
        }
        let Ok(cwd) = fs::read_link(format!("/proc/{pid}/cwd")) else {
            continue;
        };
        if cwd.starts_with(&state_dir) {
            // SAFETY: the PID belongs to a VPCS process confirmed above.
            if unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) } == 0 {
                killed += 1;
            }
        }
    }
    killed
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
            active_ports: Arc::new(RwLock::new(HashMap::new())),
            bridge_ports: Arc::new(RwLock::new(HashMap::new())),
            requested_runtime_ports: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Kill VPCS processes left behind by an earlier controller run.
    pub fn reap_orphans(&self) -> usize {
        reap_orphaned_processes(&self.binary, &self.state_dir)
    }

    /// Kill every VPCS process this runtime started (controller shutdown).
    pub async fn kill_all(&self) {
        let mut processes = self.processes.lock().await;
        for (_, process) in processes.drain() {
            process.kill_group();
        }
        self.active_ports.write().await.clear();
    }

    #[cfg(test)]
    pub fn with_paths(binary: impl Into<PathBuf>, state_dir: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            state_dir: state_dir.into(),
            processes: Arc::new(Mutex::new(HashMap::new())),
            active_ports: Arc::new(RwLock::new(HashMap::new())),
            bridge_ports: Arc::new(RwLock::new(HashMap::new())),
            requested_runtime_ports: Arc::new(RwLock::new(HashMap::new())),
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
            "startup_script": "",
            "base_script_file": "vpcs_base_config.txt",
            "console_type": "telnet",
            "console_auto_start": false
        })])
    }

    pub async fn ensure_available(&self) -> Result<String, RuntimeError> {
        self.version().await
    }

    pub async fn set_bridge_port(&self, node_id: &str, port: u16) {
        self.bridge_ports
            .write()
            .await
            .insert(node_id.to_owned(), port);
    }

    pub async fn adapter_udp_port(&self, node_id: &str) -> Option<u16> {
        self.bridge_ports.read().await.get(node_id).copied()
    }

    pub async fn set_nio_ports(&self, node_id: &str, adapter_port: u16, runtime_port: u16) {
        self.bridge_ports
            .write()
            .await
            .insert(node_id.to_owned(), adapter_port);
        self.requested_runtime_ports
            .write()
            .await
            .insert(node_id.to_owned(), runtime_port);
    }

    pub async fn active_udp_port(&self, node_id: &str) -> Option<u16> {
        self.active_ports.read().await.get(node_id).copied()
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
            // TODO(RUNTIME-NET VPCS suspend hook): apply node suspension by
            // dropping the connected NIO carrier; the VPCS process itself stays running.
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
        process.kill_group();
        self.active_ports.write().await.remove(node_id);
        Ok("stopped".to_owned())
    }

    pub async fn remove_node(&self, project_id: &str, node_id: &str) -> Result<(), RuntimeError> {
        self.stop(node_id).await?;
        self.bridge_ports.write().await.remove(node_id);
        self.requested_runtime_ports.write().await.remove(node_id);
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

        let switch_port = if let Some(port) = self.adapter_udp_port(node_id).await {
            port
        } else {
            let port = available_udp_port().await?;
            self.set_bridge_port(node_id, port).await;
            port
        };

        let node_dir = self
            .state_dir
            .join(project_id.to_string())
            .join(node_id_uuid.to_string());
        fs::create_dir_all(&node_dir).map_err(|error| {
            RuntimeError::Failed(format!("Could not create VPCS node directory: {error}"))
        })?;

        let script_path = write_startup_script(&node_dir, startup_script, name)?;

        let console_port = available_tcp_port().await?;
        let local_udp_port = if let Some(port) = self
            .requested_runtime_ports
            .read()
            .await
            .get(node_id)
            .copied()
        {
            port
        } else {
            available_udp_port().await?
        };
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
            .process_group(0)
            .kill_on_drop(true);
        command.arg(script_path.file_name().unwrap_or_default());
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
            match child.try_wait() {
                Ok(Some(status)) => {
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
                Ok(None) => {}
                Err(error) => {
                    kill_vpcs_startup_tree(&mut child).await;
                    self.active_ports.write().await.remove(node_id);
                    return Err(RuntimeError::Unavailable(format!(
                        "Could not inspect VPCS startup: {error}"
                    )));
                }
            }
            if tokio::time::Instant::now() >= deadline {
                kill_vpcs_startup_tree(&mut child).await;
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

async fn kill_vpcs_startup_tree(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        kill_process_group(pid);
    }
    let _ = child.start_kill();
    let _ = child.wait().await;
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

fn write_startup_script(
    node_dir: &std::path::Path,
    startup_script: Option<&str>,
    name: &str,
) -> Result<PathBuf, RuntimeError> {
    let path = node_dir.join("startup.vpc");
    let script = startup_script.unwrap_or_default().replace("%h", name);
    fs::write(&path, script).map_err(|error| {
        RuntimeError::Failed(format!("Could not write VPCS startup script: {error}"))
    })?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use std::{env, fs, path::PathBuf, time::Duration};

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        time::timeout,
    };
    use uuid::Uuid;

    use std::{
        net::{Ipv4Addr, SocketAddr, SocketAddrV4},
        process::Stdio,
    };

    use tokio::process::Command;

    use super::{
        ManagedVpcs, VpcsRuntime, connect_vpcs_console, kill_vpcs_startup_tree, mac_offset,
        write_startup_script,
    };

    #[test]
    fn derives_mac_offset_from_the_node_identifier() {
        let id = Uuid::parse_str("00000000-0000-0000-0000-00000000002a").unwrap();
        assert_eq!(mac_offset(&id), 42);
    }

    #[test]
    fn startup_script_write_clears_old_content_and_expands_hostname() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_startup_script(directory.path(), Some("name %h"), "PC 1").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "name PC 1");
        write_startup_script(directory.path(), None, "PC 1").unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "");
    }

    #[test]
    fn separate_instances_keep_independent_base_startup_scripts() {
        let directory = tempfile::tempdir().unwrap();
        let first_dir = directory.path().join("project/first");
        let second_dir = directory.path().join("project/second");
        fs::create_dir_all(&first_dir).unwrap();
        fs::create_dir_all(&second_dir).unwrap();
        let first =
            write_startup_script(&first_dir, Some("ip 192.0.2.1/24 name %h"), "PC1").unwrap();
        let second =
            write_startup_script(&second_dir, Some("ip 192.0.2.2/24 name %h"), "PC2").unwrap();
        assert_eq!(
            fs::read_to_string(first).unwrap(),
            "ip 192.0.2.1/24 name PC1"
        );
        assert_eq!(
            fs::read_to_string(second).unwrap(),
            "ip 192.0.2.2/24 name PC2"
        );
    }

    #[tokio::test]
    #[ignore = "requires upstream VPCS and uBridge; set NETWORK_SIMULATOR_TEST_VPCS and NETWORK_SIMULATOR_TEST_UBRIDGE"]
    async fn upstream_vpcs_nodes_exchange_ethernet_frames_over_gns3_udp_nios() {
        let binary = env::var_os("NETWORK_SIMULATOR_TEST_VPCS")
            .expect("NETWORK_SIMULATOR_TEST_VPCS is set for this integration test");
        let directory = tempfile::tempdir().expect("temporary VPCS state directory");
        let runtime = VpcsRuntime::with_paths(PathBuf::from(binary), directory.path());
        let ubridge_binary = env::var_os("NETWORK_SIMULATOR_TEST_UBRIDGE")
            .expect("NETWORK_SIMULATOR_TEST_UBRIDGE is set for this integration test");
        let ubridge = crate::ubridge::UbridgeRuntime::with_paths(
            PathBuf::from(ubridge_binary),
            directory.path().join("ubridge.sock"),
        );
        runtime
            .ensure_available()
            .await
            .expect("upstream VPCS is available");

        let project_id = "00000000-0000-0000-0000-000000000001";
        let node_a = "00000000-0000-0000-0000-00000000000a";
        let node_b = "00000000-0000-0000-0000-00000000000b";
        let endpoint_a = "a0000000-0000-0000-0000-000000000001";
        let endpoint_b = "b0000000-0000-0000-0000-000000000002";
        let link_a = ubridge
            .reserve_udp_port()
            .await
            .expect("first link NIO port");
        let link_b = ubridge
            .reserve_udp_port()
            .await
            .expect("second link NIO port");
        let adapter_a = ubridge
            .reserve_udp_port()
            .await
            .expect("first adapter NIO port");
        let adapter_b = ubridge
            .reserve_udp_port()
            .await
            .expect("second adapter NIO port");
        let runtime_a = ubridge
            .reserve_udp_port()
            .await
            .expect("first VPCS NIO port");
        let runtime_b = ubridge
            .reserve_udp_port()
            .await
            .expect("second VPCS NIO port");
        runtime.set_nio_ports(node_a, adapter_a, runtime_a).await;
        runtime.set_nio_ports(node_b, adapter_b, runtime_b).await;
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
        ubridge
            .attach_link_endpoint(
                endpoint_a,
                link_a,
                link_b,
                Some(adapter_a),
                Some(runtime_a),
                None,
                &serde_json::json!({}),
                false,
                None,
            )
            .await
            .expect("first VPCS uBridge NIO attaches");
        ubridge
            .attach_link_endpoint(
                endpoint_b,
                link_b,
                link_a,
                Some(adapter_b),
                Some(runtime_b),
                None,
                &serde_json::json!({}),
                false,
                None,
            )
            .await
            .expect("second VPCS uBridge NIO attaches");

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
        ubridge
            .delete_link_endpoint(endpoint_a)
            .await
            .expect("first endpoint detaches");
        ubridge
            .delete_link_endpoint(endpoint_b)
            .await
            .expect("second endpoint detaches");
        runtime.stop(node_a).await.expect("first VPCS stops");
        runtime.stop(node_b).await.expect("second VPCS stops");
        assert!(
            connect_vpcs_console(address).await.is_err(),
            "stopped VPCS console should no longer accept connections"
        );
    }

    #[tokio::test]
    async fn dropping_a_managed_process_kills_its_forked_children() {
        use tokio::io::AsyncReadExt as _;

        // Mimic VPCS: a tracked daemon that forks a long-lived child.
        let mut child = Command::new("sh")
            .args(["-c", "setsid sleep 300 & echo $!; wait"])
            .stdout(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true)
            .spawn()
            .expect("spawn fake VPCS");
        let mut stdout = child.stdout.take().expect("stdout pipe");
        let mut buffer = [0_u8; 32];
        let read = stdout.read(&mut buffer).await.expect("fork pid");
        let forked: libc::pid_t = String::from_utf8_lossy(&buffer[..read])
            .trim()
            .parse()
            .expect("numeric pid");
        // SAFETY: signal 0 only probes for existence.
        assert_eq!(unsafe { libc::kill(forked, 0) }, 0, "fork is running");

        drop(ManagedVpcs {
            child,
            console: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 1)),
        });
        for _ in 0..50 {
            // SAFETY: as above.
            if unsafe { libc::kill(forked, 0) } != 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("forked child survived the drop of its managed process");
    }

    #[tokio::test]
    async fn failed_start_cleanup_kills_the_vpcs_process_tree() {
        use tokio::io::AsyncReadExt as _;

        let mut child = Command::new("sh")
            .args(["-c", "setsid sleep 300 & echo $!; wait"])
            .stdout(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true)
            .spawn()
            .expect("spawn fake VPCS startup");
        let mut stdout = child.stdout.take().expect("stdout pipe");
        let mut buffer = [0_u8; 32];
        let read = stdout.read(&mut buffer).await.expect("fork pid");
        let forked: libc::pid_t = String::from_utf8_lossy(&buffer[..read])
            .trim()
            .parse()
            .expect("numeric fork pid");
        // SAFETY: signal 0 only probes that the test child is running.
        assert_eq!(unsafe { libc::kill(forked, 0) }, 0);

        kill_vpcs_startup_tree(&mut child).await;
        for _ in 0..50 {
            // SAFETY: signal 0 only probes for existence.
            if unsafe { libc::kill(forked, 0) } != 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("forked process survived failed-start cleanup");
    }
}
