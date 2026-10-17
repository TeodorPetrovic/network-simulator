use std::{
    fs,
    io::ErrorKind,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use serde::Serialize;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    process::Command,
    time::{sleep, timeout},
};
use uuid::Uuid;

use crate::runtime::{NodeAction, RuntimeError};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const QMP_TIMEOUT: Duration = Duration::from_secs(5);
const START_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone)]
pub struct QemuRuntime {
    binary: PathBuf,
    qemu_img: PathBuf,
    image_dir: PathBuf,
    state_dir: PathBuf,
    qmp_dir: PathBuf,
}

#[derive(Clone, Debug)]
pub struct QemuVm {
    pub node_id: String,
    pub image: PathBuf,
    pub overlay: PathBuf,
    pub qmp_socket: PathBuf,
    pub serial_socket: PathBuf,
    pub pid_file: PathBuf,
    pub log_file: PathBuf,
    pub memory_mb: u64,
    pub cpus: u32,
    pub adapters: u32,
}

#[derive(Clone, Debug)]
pub struct QemuLink {
    pub link_id: String,
    pub adapter_number: u32,
    pub multicast_address: Ipv4Addr,
    pub multicast_port: u16,
    pub tap: Option<String>,
}

#[derive(Serialize)]
pub struct QemuRuntimeInfo {
    pub available: bool,
    pub version: Option<String>,
    pub qemu_img_available: bool,
    pub image_directory: String,
    pub image_count: usize,
    pub acceleration: String,
    pub kvm_accessible: bool,
    pub acceleration_error: Option<String>,
    pub error: Option<String>,
}

impl QemuRuntime {
    pub fn new() -> Self {
        let state_dir = std::env::var_os("NETWORK_SIMULATOR_QEMU_STATE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("data/qemu"));
        Self {
            binary: std::env::var_os("NETWORK_SIMULATOR_QEMU")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("qemu-system-x86_64")),
            qemu_img: std::env::var_os("NETWORK_SIMULATOR_QEMU_IMG")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("qemu-img")),
            image_dir: crate::image_paths::qemu_images(),
            qmp_dir: std::env::var_os("NETWORK_SIMULATOR_QEMU_SOCKETS")
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::temp_dir().join("network-simulator-qemu")),
            state_dir,
        }
    }

    #[cfg(test)]
    pub fn with_paths(
        binary: impl Into<PathBuf>,
        qemu_img: impl Into<PathBuf>,
        image_dir: impl Into<PathBuf>,
        state_dir: impl Into<PathBuf>,
        qmp_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            binary: binary.into(),
            qemu_img: qemu_img.into(),
            image_dir: image_dir.into(),
            state_dir: state_dir.into(),
            qmp_dir: qmp_dir.into(),
        }
    }

    pub async fn info(&self) -> QemuRuntimeInfo {
        let images = self.image_files().unwrap_or_default();
        let image_directory = self
            .image_dir
            .canonicalize()
            .unwrap_or_else(|_| self.image_dir.clone())
            .display()
            .to_string();
        let qemu_img_available = self
            .command(&self.qemu_img, ["--version"], COMMAND_TIMEOUT)
            .await
            .is_ok();
        let version = match self
            .command(&self.binary, ["-version"], COMMAND_TIMEOUT)
            .await
        {
            Ok(output) => output.lines().next().map(str::to_owned),
            Err(_) => None,
        };
        let error = if version.is_none() {
            Some(format!(
                "QEMU binary '{}' is unavailable",
                self.binary.display()
            ))
        } else if !qemu_img_available {
            Some(format!(
                "qemu-img binary '{}' is unavailable",
                self.qemu_img.display()
            ))
        } else {
            None
        };
        let acceleration_error = kvm_access_error();
        let kvm_accessible = acceleration_error.is_none();
        QemuRuntimeInfo {
            available: error.is_none(),
            version,
            qemu_img_available,
            image_directory,
            image_count: images.len(),
            acceleration: if kvm_accessible { "KVM" } else { "TCG" }.to_owned(),
            kvm_accessible,
            acceleration_error,
            error,
        }
    }

    pub async fn templates(&self) -> Result<Vec<Value>, RuntimeError> {
        self.command(&self.binary, ["-version"], COMMAND_TIMEOUT)
            .await?;
        self.command(&self.qemu_img, ["--version"], COMMAND_TIMEOUT)
            .await?;
        let mut templates = Vec::new();
        for path in self.image_files()? {
            let canonical = path.canonicalize().map_err(|error| {
                RuntimeError::Failed(format!("Could not resolve QEMU image path: {error}"))
            })?;
            let template_id =
                Uuid::new_v5(&Uuid::NAMESPACE_URL, canonical.to_string_lossy().as_bytes());
            let name = path
                .file_name()
                .and_then(|file| file.to_str())
                .unwrap_or("QEMU disk image");
            templates.push(json!({
                "template_id": template_id,
                "name": name,
                "template_type": "qemu",
                "compute_id": "local",
                "category": "guest",
                "symbol": "qemu_guest",
                "builtin": false,
                "hda_disk_image": canonical,
                "adapters": 8,
                "memory": 512,
                "cpus": 1
            }));
        }
        templates.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
        Ok(templates)
    }

    pub async fn create_vm(
        &self,
        project_id: &str,
        node_id: &str,
        image: &str,
        memory_mb: u64,
        cpus: u32,
        adapters: u32,
    ) -> Result<QemuVm, RuntimeError> {
        let project_id = parse_id(project_id)?;
        let node_id = parse_id(node_id)?;
        let image = self.resolve_image(image)?;
        let output = self
            .command(
                &self.qemu_img,
                [
                    "info",
                    "--output=json",
                    image.to_str().ok_or_else(|| {
                        RuntimeError::Failed("QEMU image path is not valid UTF-8".to_owned())
                    })?,
                ],
                COMMAND_TIMEOUT,
            )
            .await?;
        let info: ImageInfo = serde_json::from_str(&output).map_err(|error| {
            RuntimeError::Failed(format!("qemu-img returned invalid image metadata: {error}"))
        })?;
        if info.format.is_empty() || !info.format.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Err(RuntimeError::Failed(
                "qemu-img returned an invalid disk format".to_owned(),
            ));
        }

        let vm_dir = self.state_dir.join(project_id.to_string());
        fs::create_dir_all(&vm_dir).map_err(|error| {
            RuntimeError::Failed(format!("Could not create QEMU state directory: {error}"))
        })?;
        fs::create_dir_all(&self.qmp_dir).map_err(|error| {
            RuntimeError::Failed(format!("Could not create QEMU socket directory: {error}"))
        })?;
        let vm = QemuVm {
            node_id: node_id.to_string(),
            image,
            overlay: vm_dir.join(format!("{node_id}.qcow2")),
            qmp_socket: self.qmp_dir.join(format!("{node_id}.sock")),
            serial_socket: self.qmp_dir.join(format!("{node_id}.serial.sock")),
            pid_file: vm_dir.join(format!("{node_id}.pid")),
            log_file: vm_dir.join(format!("{node_id}.log")),
            memory_mb,
            cpus,
            adapters,
        };
        if !vm.overlay.exists() {
            self.command(
                &self.qemu_img,
                [
                    "create",
                    "-f",
                    "qcow2",
                    "-F",
                    info.format.as_str(),
                    "-b",
                    vm.image.to_str().ok_or_else(|| {
                        RuntimeError::Failed("QEMU image path is not valid UTF-8".to_owned())
                    })?,
                    vm.overlay.to_str().ok_or_else(|| {
                        RuntimeError::Failed("QEMU overlay path is not valid UTF-8".to_owned())
                    })?,
                ],
                COMMAND_TIMEOUT,
            )
            .await?;
        }
        Ok(vm)
    }

    pub async fn lifecycle(
        &self,
        vm: &QemuVm,
        action: NodeAction,
        links: &[QemuLink],
    ) -> Result<String, RuntimeError> {
        match action {
            NodeAction::Start => self.start(vm, links).await,
            NodeAction::Stop => self.stop(vm).await,
            NodeAction::Suspend => {
                self.qmp_execute(&vm.qmp_socket, "stop", None).await?;
                self.status(vm).await
            }
            NodeAction::Reload => {
                self.qmp_execute(&vm.qmp_socket, "system_reset", None)
                    .await?;
                self.status(vm).await
            }
        }
    }

    pub async fn status(&self, vm: &QemuVm) -> Result<String, RuntimeError> {
        if !vm.qmp_socket.exists() {
            return Ok("stopped".to_owned());
        }
        let status = self
            .qmp_execute(&vm.qmp_socket, "query-status", None)
            .await?;
        let status = status
            .get("status")
            .and_then(Value::as_str)
            .ok_or_else(|| RuntimeError::Failed("QMP query-status omitted VM status".to_owned()))?;
        Ok(match status {
            "running" | "inmigrate" | "postmigrate" => "started",
            "paused" | "prelaunch" | "suspended" => "suspended",
            _ => "stopped",
        }
        .to_owned())
    }

    pub async fn remove_vm(&self, vm: &QemuVm) -> Result<(), RuntimeError> {
        self.stop(vm).await?;
        for path in [
            &vm.overlay,
            &vm.qmp_socket,
            &vm.serial_socket,
            &vm.pid_file,
            &vm.log_file,
        ] {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(RuntimeError::Failed(format!(
                        "Could not remove QEMU runtime file '{}': {error}",
                        path.display()
                    )));
                }
            }
        }
        Ok(())
    }

    pub fn vm_from_node(&self, node: &Value) -> Result<QemuVm, RuntimeError> {
        let _project_id = parse_id(node["project_id"].as_str().unwrap_or_default())?;
        let node_id = parse_id(node["node_id"].as_str().unwrap_or_default())?;
        let image = required_path(node, "hda_disk_image")?;
        let overlay = required_path(node, "runtime_overlay")?;
        let qmp_socket = required_path(node, "runtime_qmp_socket")?;
        let serial_socket = node["runtime_serial_socket"]
            .as_str()
            .map(PathBuf::from)
            .unwrap_or_else(|| qmp_socket.with_file_name(format!("{node_id}.serial.sock")));
        let pid_file = required_path(node, "runtime_pid_file")?;
        let log_file = required_path(node, "runtime_log_file")?;
        Ok(QemuVm {
            node_id: node_id.to_string(),
            image,
            overlay,
            qmp_socket,
            serial_socket,
            pid_file,
            log_file,
            memory_mb: node["memory"].as_u64().unwrap_or(512).max(1),
            cpus: node["cpus"].as_u64().unwrap_or(1).max(1) as u32,
            adapters: node["adapters"].as_u64().unwrap_or(8).max(1) as u32,
        })
    }

    pub fn runtime_fields(&self, vm: &QemuVm) -> Value {
        json!({
            "hda_disk_image": vm.image,
            "runtime_overlay": vm.overlay,
            "runtime_qmp_socket": vm.qmp_socket,
            "runtime_serial_socket": vm.serial_socket,
            "runtime_pid_file": vm.pid_file,
            "runtime_log_file": vm.log_file
        })
    }

    pub fn multicast_endpoint(link_id: &str) -> Result<(Ipv4Addr, u16), RuntimeError> {
        let id = parse_id(link_id)?;
        let bytes = id.as_bytes();
        let address = Ipv4Addr::new(239, bytes[0], bytes[1], bytes[2]);
        let port_seed = u16::from_be_bytes([bytes[3], bytes[4]]);
        let port = 1024 + (port_seed % (u16::MAX - 1024));
        Ok((address, port))
    }

    pub fn prepare_upload_name(&self, file_name: &str) -> Result<(PathBuf, PathBuf), RuntimeError> {
        let safe_name = safe_disk_image_name(file_name)?;
        fs::create_dir_all(&self.image_dir).map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not create QEMU image directory '{}': {error}",
                self.image_dir.display()
            ))
        })?;
        let directory = self.image_dir.canonicalize().map_err(|error| {
            RuntimeError::Failed(format!("Could not resolve QEMU image directory: {error}"))
        })?;
        let final_path = directory.join(&safe_name);
        if final_path.exists() {
            return Err(RuntimeError::Failed(format!(
                "A QEMU image named '{safe_name}' already exists"
            )));
        }
        let temporary_path = directory.join(format!(".{}.{}.upload", Uuid::new_v4(), safe_name));
        Ok((temporary_path, final_path))
    }

    pub async fn finalize_uploaded_image(
        &self,
        temporary_path: &Path,
        final_path: &Path,
    ) -> Result<PathBuf, RuntimeError> {
        let metadata = fs::metadata(temporary_path).map_err(|error| {
            RuntimeError::Failed(format!("Could not read uploaded QEMU image: {error}"))
        })?;
        if metadata.len() == 0 {
            return Err(RuntimeError::Failed(
                "QEMU image upload is empty".to_owned(),
            ));
        }
        let output = self
            .command(
                &self.qemu_img,
                [
                    "info",
                    "--output=json",
                    temporary_path.to_str().ok_or_else(|| {
                        RuntimeError::Failed("QEMU image path is not valid UTF-8".to_owned())
                    })?,
                ],
                COMMAND_TIMEOUT,
            )
            .await?;
        let info: Value = serde_json::from_str(&output).map_err(|error| {
            RuntimeError::Failed(format!("qemu-img returned invalid image metadata: {error}"))
        })?;
        let format = info["format"].as_str().unwrap_or_default();
        if !image_format_matches(final_path, format) {
            return Err(RuntimeError::Failed(format!(
                "QEMU image extension does not match detected format '{format}'"
            )));
        }
        fs::hard_link(temporary_path, final_path).map_err(|error| {
            RuntimeError::Failed(format!("Could not install QEMU image: {error}"))
        })?;
        fs::remove_file(temporary_path).map_err(|error| {
            RuntimeError::Failed(format!("Could not finalize QEMU image upload: {error}"))
        })?;
        final_path.canonicalize().map_err(|error| {
            RuntimeError::Failed(format!("Could not resolve installed QEMU image: {error}"))
        })
    }

    fn image_files(&self) -> Result<Vec<PathBuf>, RuntimeError> {
        let directory = match self.image_dir.canonicalize() {
            Ok(directory) => directory,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(RuntimeError::Failed(format!(
                    "Could not read QEMU image directory '{}': {error}",
                    self.image_dir.display()
                )));
            }
        };
        let entries = fs::read_dir(&directory).map_err(|error| {
            RuntimeError::Failed(format!("Could not list QEMU images: {error}"))
        })?;
        let mut files = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                RuntimeError::Failed(format!("Could not read QEMU image entry: {error}"))
            })?;
            let path = entry.path();
            if !path.is_file() || !is_disk_image(&path) {
                continue;
            }
            let canonical = path.canonicalize().map_err(|error| {
                RuntimeError::Failed(format!("Could not resolve QEMU image path: {error}"))
            })?;
            if canonical.starts_with(&directory) {
                files.push(canonical);
            }
        }
        files.sort();
        Ok(files)
    }

    fn resolve_image(&self, image: &str) -> Result<PathBuf, RuntimeError> {
        let root = self.image_dir.canonicalize().map_err(|error| {
            RuntimeError::Unavailable(format!(
                "QEMU image directory '{}' is unavailable: {error}",
                self.image_dir.display()
            ))
        })?;
        let path = Path::new(image).canonicalize().map_err(|error| {
            RuntimeError::NotFound(format!("QEMU disk image '{image}' is unavailable: {error}"))
        })?;
        if !path.starts_with(&root) || !path.is_file() || !is_disk_image(&path) {
            return Err(RuntimeError::Failed(
                "QEMU disk images must be supported image files inside the configured image directory".to_owned(),
            ));
        }
        Ok(path)
    }

    async fn start(&self, vm: &QemuVm, links: &[QemuLink]) -> Result<String, RuntimeError> {
        if !vm.overlay.is_file() {
            return Err(RuntimeError::NotFound(format!(
                "QEMU node disk overlay '{}' is missing",
                vm.overlay.display()
            )));
        }
        if process_matches(&vm.pid_file, &vm.qmp_socket) {
            if !vm.qmp_socket.exists() {
                return Err(RuntimeError::Unavailable(
                    "QEMU is running but its QMP monitor socket is missing".to_owned(),
                ));
            }
            match self.status(vm).await {
                Ok(status) if status == "started" => return Ok("started".to_owned()),
                Ok(status) if status == "suspended" => {
                    self.qmp_execute(&vm.qmp_socket, "cont", None).await?;
                    return self.status(vm).await;
                }
                Ok(status) if status == "stopped" => {
                    self.qmp_execute(&vm.qmp_socket, "system_reset", None)
                        .await?;
                    return self.status(vm).await;
                }
                Ok(_) => {
                    return Err(RuntimeError::Failed(
                        "QEMU is running in a state that cannot be resumed".to_owned(),
                    ));
                }
                Err(error) => {
                    return Err(RuntimeError::Unavailable(format!(
                        "QEMU is running but its QMP monitor is not responding: {}",
                        error.message()
                    )));
                }
            }
        }
        let _ = fs::remove_file(&vm.qmp_socket);
        let _ = fs::remove_file(&vm.serial_socket);
        let _ = fs::remove_file(&vm.pid_file);
        let _ = fs::remove_file(&vm.log_file);
        let mut command = Command::new(&self.binary);
        command
            .arg("-daemonize")
            .arg("-pidfile")
            .arg(&vm.pid_file)
            .arg("-D")
            .arg(&vm.log_file)
            .arg("-qmp")
            .arg(format!(
                "unix:{},server=on,wait=off",
                vm.qmp_socket.display()
            ))
            .arg("-display")
            .arg("none")
            .arg("-monitor")
            .arg("none")
            .arg("-chardev")
            .arg(format!(
                "socket,id=serial0,path={},server=on,wait=off",
                qemu_keyval_escape(&vm.serial_socket.display().to_string())
            ))
            .arg("-serial")
            .arg("chardev:serial0")
            .arg("-no-reboot")
            .arg("-accel")
            .arg(if kvm_accessible() { "kvm" } else { "tcg" })
            .arg("-m")
            .arg(vm.memory_mb.to_string())
            .arg("-smp")
            .arg(vm.cpus.to_string())
            .arg("-drive")
            .arg(format!(
                "file={},if=virtio,format=qcow2",
                qemu_keyval_escape(&vm.overlay.display().to_string())
            ));
        if links.is_empty() {
            command.arg("-nic").arg("none");
        } else {
            for link in links {
                if link.adapter_number >= vm.adapters {
                    return Err(RuntimeError::Failed(format!(
                        "Invalid QEMU link configuration for adapter {}",
                        link.adapter_number
                    )));
                }
                let net_id = format!("ns_{}", link.link_id.replace('-', ""));
                command
                    .arg("-netdev")
                    .arg(qemu_netdev_argument(link, &net_id))
                    .arg("-device")
                    .arg(format!(
                        "virtio-net-pci,netdev={net_id},mac={}",
                        mac_address(&vm.node_id, link.adapter_number)
                    ));
            }
        }
        let output = timeout(COMMAND_TIMEOUT, command.stdin(Stdio::null()).output())
            .await
            .map_err(|_| RuntimeError::Unavailable("QEMU startup timed out".to_owned()))?
            .map_err(|error| {
                RuntimeError::Unavailable(format!(
                    "Could not run QEMU binary '{}': {error}",
                    self.binary.display()
                ))
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(RuntimeError::Failed(if stderr.is_empty() {
                format!("QEMU exited with {}", output.status)
            } else {
                stderr
            }));
        }
        let deadline = tokio::time::Instant::now() + START_TIMEOUT;
        loop {
            if vm.qmp_socket.exists() {
                match self.status(vm).await {
                    Ok(status) if status == "started" || status == "suspended" => {
                        return Ok(status);
                    }
                    Ok(_) => {}
                    Err(error) => {
                        if tokio::time::Instant::now() >= deadline {
                            let log = fs::read_to_string(&vm.log_file).unwrap_or_default();
                            return Err(RuntimeError::Failed(format!(
                                "QEMU started without a usable QMP monitor: {}{}",
                                error.message(),
                                if log.is_empty() {
                                    String::new()
                                } else {
                                    format!("; {log}")
                                }
                            )));
                        }
                    }
                }
            }
            if tokio::time::Instant::now() >= deadline {
                let log = fs::read_to_string(&vm.log_file).unwrap_or_default();
                return Err(RuntimeError::Failed(format!(
                    "QEMU did not create its QMP monitor{}",
                    if log.is_empty() {
                        String::new()
                    } else {
                        format!(": {log}")
                    }
                )));
            }
            sleep(Duration::from_millis(100)).await;
        }
    }

    async fn stop(&self, vm: &QemuVm) -> Result<String, RuntimeError> {
        let pid_matches = process_matches(&vm.pid_file, &vm.qmp_socket);
        if !vm.qmp_socket.exists() && pid_matches {
            return Err(RuntimeError::Unavailable(
                "QEMU is running but its QMP monitor socket is missing".to_owned(),
            ));
        }
        if !vm.qmp_socket.exists() {
            let _ = fs::remove_file(&vm.qmp_socket);
            let _ = fs::remove_file(&vm.serial_socket);
            let _ = fs::remove_file(&vm.pid_file);
            return Ok("stopped".to_owned());
        }
        if let Err(error) = self.qmp_execute(&vm.qmp_socket, "quit", None).await {
            if pid_matches {
                return Err(RuntimeError::Unavailable(format!(
                    "Could not stop QEMU because its QMP monitor is unavailable: {}",
                    error.message()
                )));
            }
            let _ = fs::remove_file(&vm.qmp_socket);
            let _ = fs::remove_file(&vm.serial_socket);
            let _ = fs::remove_file(&vm.pid_file);
            return Ok("stopped".to_owned());
        }
        let deadline = tokio::time::Instant::now() + START_TIMEOUT;
        while process_matches(&vm.pid_file, &vm.qmp_socket) {
            if tokio::time::Instant::now() >= deadline {
                return Err(RuntimeError::Failed(
                    "QEMU did not exit after the QMP quit request".to_owned(),
                ));
            }
            sleep(Duration::from_millis(100)).await;
        }
        let _ = fs::remove_file(&vm.qmp_socket);
        let _ = fs::remove_file(&vm.serial_socket);
        let _ = fs::remove_file(&vm.pid_file);
        Ok("stopped".to_owned())
    }

    async fn qmp_execute(
        &self,
        socket: &Path,
        execute: &str,
        arguments: Option<Value>,
    ) -> Result<Value, RuntimeError> {
        timeout(QMP_TIMEOUT, qmp_execute_inner(socket, execute, arguments))
            .await
            .map_err(|_| RuntimeError::Unavailable("QEMU QMP request timed out".to_owned()))?
    }

    async fn command<I, S>(
        &self,
        binary: &Path,
        args: I,
        duration: Duration,
    ) -> Result<String, RuntimeError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let mut command = Command::new(binary);
        command.args(args).stdin(Stdio::null()).kill_on_drop(true);
        let output = timeout(duration, command.output())
            .await
            .map_err(|_| RuntimeError::Unavailable("QEMU utility command timed out".to_owned()))?
            .map_err(|error| {
                RuntimeError::Unavailable(format!(
                    "Could not run QEMU utility '{}': {error}",
                    binary.display()
                ))
            })?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(RuntimeError::Failed(if stderr.is_empty() {
            format!("QEMU utility exited with {}", output.status)
        } else {
            stderr
        }))
    }
}

impl Default for QemuRuntime {
    fn default() -> Self {
        Self::new()
    }
}

async fn qmp_execute_inner(
    socket: &Path,
    execute: &str,
    arguments: Option<Value>,
) -> Result<Value, RuntimeError> {
    let stream = UnixStream::connect(socket).await.map_err(|error| {
        RuntimeError::Unavailable(format!("Could not connect to QEMU QMP socket: {error}"))
    })?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await.map_err(|error| {
        RuntimeError::Unavailable(format!("Could not read the QEMU QMP greeting: {error}"))
    })?;
    let greeting: Value = serde_json::from_str(&line).map_err(|error| {
        RuntimeError::Failed(format!("QEMU returned an invalid QMP greeting: {error}"))
    })?;
    if greeting.get("QMP").is_none() {
        return Err(RuntimeError::Failed(
            "QEMU QMP socket did not send a protocol greeting".to_owned(),
        ));
    }
    send_qmp_command(&mut reader, "qmp_capabilities", None, "capabilities").await?;
    send_qmp_command(&mut reader, execute, arguments, "request").await
}

async fn send_qmp_command(
    reader: &mut BufReader<UnixStream>,
    execute: &str,
    arguments: Option<Value>,
    id: &str,
) -> Result<Value, RuntimeError> {
    let mut request = json!({ "execute": execute, "id": id });
    if let Some(arguments) = arguments {
        request["arguments"] = arguments;
    }
    let mut serialized = serde_json::to_vec(&request)
        .map_err(|error| RuntimeError::Failed(format!("Could not encode QMP request: {error}")))?;
    serialized.push(b'\n');
    reader
        .get_mut()
        .write_all(&serialized)
        .await
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not write a QEMU QMP request: {error}"))
        })?;
    loop {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).await.map_err(|error| {
            RuntimeError::Unavailable(format!("Could not read a QEMU QMP response: {error}"))
        })?;
        if bytes == 0 {
            return Err(RuntimeError::Unavailable(
                "QEMU closed the QMP connection before replying".to_owned(),
            ));
        }
        let response: Value = serde_json::from_str(&line).map_err(|error| {
            RuntimeError::Failed(format!("QEMU returned an invalid QMP response: {error}"))
        })?;
        if response.get("event").is_some() {
            continue;
        }
        if response.get("id").and_then(Value::as_str) != Some(id) {
            continue;
        }
        if let Some(error) = response.get("error") {
            return Err(RuntimeError::Failed(format!(
                "QEMU QMP {execute} failed: {error}"
            )));
        }
        return Ok(response.get("return").cloned().unwrap_or(Value::Null));
    }
}

fn parse_id(id: &str) -> Result<Uuid, RuntimeError> {
    Uuid::parse_str(id).map_err(|_| RuntimeError::Failed("QEMU IDs must be valid UUIDs".to_owned()))
}

fn required_path(node: &Value, key: &str) -> Result<PathBuf, RuntimeError> {
    node.get(key)
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| RuntimeError::Failed(format!("QEMU node is missing runtime field '{key}'")))
}

fn is_disk_image(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("qcow2" | "qcow" | "raw" | "img" | "vmdk" | "vdi")
    )
}

fn safe_disk_image_name(file_name: &str) -> Result<String, RuntimeError> {
    if file_name.is_empty()
        || file_name.len() > 180
        || file_name.starts_with('.')
        || !file_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+'))
        || !is_disk_image(Path::new(file_name))
    {
        return Err(RuntimeError::Failed(
            "QEMU image name must be a single supported disk-image filename".to_owned(),
        ));
    }
    Ok(file_name.to_owned())
}

fn image_format_matches(path: &Path, format: &str) -> bool {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("qcow2") => format == "qcow2",
        Some("qcow") => format == "qcow",
        Some("raw" | "img") => format == "raw",
        Some("vmdk") => format == "vmdk",
        Some("vdi") => format == "vdi",
        _ => false,
    }
}

fn qemu_keyval_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace(',', "\\,")
}

fn mac_address(node_id: &str, adapter_number: u32) -> String {
    let bytes = Uuid::parse_str(node_id)
        .map(|id| *id.as_bytes())
        .unwrap_or([0; 16]);
    format!(
        "52:54:00:{:02x}:{:02x}:{:02x}",
        bytes[13] ^ ((adapter_number >> 16) as u8),
        bytes[14] ^ ((adapter_number >> 8) as u8),
        bytes[15] ^ (adapter_number as u8)
    )
}

fn qemu_netdev_argument(link: &QemuLink, net_id: &str) -> String {
    if let Some(tap) = &link.tap {
        format!("tap,id={net_id},ifname={tap},script=no,downscript=no")
    } else {
        format!(
            "socket,id={net_id},mcast={}:{},localaddr=127.0.0.1",
            link.multicast_address, link.multicast_port
        )
    }
}

fn process_matches(pid_file: &Path, qmp_socket: &Path) -> bool {
    let Ok(pid) = fs::read_to_string(pid_file) else {
        return false;
    };
    let pid = pid.trim();
    if !pid.bytes().all(|byte| byte.is_ascii_digit()) || pid.is_empty() {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        let command_line = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        let command_line = String::from_utf8_lossy(&command_line);
        let socket_path = qmp_socket.display().to_string();
        if command_line.contains(&socket_path) {
            return true;
        }
        let environment = fs::read(format!("/proc/{pid}/environ")).unwrap_or_default();
        environment
            .windows(socket_path.len())
            .any(|value| value == socket_path.as_bytes())
    }
    #[cfg(not(target_os = "linux"))]
    {
        qmp_socket.exists()
    }
}

fn kvm_accessible() -> bool {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .is_ok()
}

fn kvm_access_error() -> Option<String> {
    match fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
    {
        Ok(_) => None,
        Err(error) => Some(format!(
            "KVM is unavailable to this service account: {error}"
        )),
    }
}

#[derive(serde::Deserialize)]
struct ImageInfo {
    format: String,
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::BufRead,
        os::unix::{fs::PermissionsExt, net::UnixListener},
        thread,
        time::Duration,
    };

    use serde_json::{Value, json};
    use tempfile::TempDir;

    use super::{
        QemuLink, QemuRuntime, image_format_matches, mac_address, qemu_keyval_escape,
        qemu_netdev_argument, qmp_execute_inner, safe_disk_image_name,
    };

    #[test]
    fn qmp_process_fixture() {
        let Ok(socket_path) = std::env::var("NETWORK_SIMULATOR_TEST_QMP_SOCKET") else {
            return;
        };
        let pid_file = std::env::var("NETWORK_SIMULATOR_TEST_QMP_PIDFILE")
            .expect("QMP fixture pid file is provided");
        let state_file = format!("{socket_path}.state");
        for stale in [&socket_path, &state_file] {
            let _ = fs::remove_file(stale);
        }
        fs::write(&state_file, "running").expect("QMP state file is created");
        let listener = UnixListener::bind(&socket_path).expect("QMP fixture socket binds");
        let mut alive = true;
        while alive {
            let (mut connection, _) = listener.accept().expect("QMP fixture accepts client");
            use std::io::{BufRead, BufReader, Write};
            connection
                .write_all(b"{\"QMP\":{\"version\":{}}}\n")
                .expect("QMP fixture greeting writes");
            let mut reader = BufReader::new(connection.try_clone().expect("QMP socket clones"));
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .expect("QMP capabilities arrive");
            connection
                .write_all(b"{\"return\":{},\"id\":\"capabilities\"}\n")
                .expect("QMP capabilities reply writes");
            line.clear();
            reader.read_line(&mut line).expect("QMP request arrives");
            let request: Value = serde_json::from_str(&line).expect("QMP request is JSON");
            let command = request["execute"].as_str().expect("QMP command exists");
            let result = match command {
                "query-status" => json!({
                    "return": { "status": fs::read_to_string(&state_file).expect("QMP state reads") },
                    "id": request["id"]
                }),
                "stop" => {
                    fs::write(&state_file, "paused").expect("paused state is stored");
                    json!({ "return": {}, "id": request["id"] })
                }
                "cont" => {
                    fs::write(&state_file, "running").expect("running state is stored");
                    json!({ "return": {}, "id": request["id"] })
                }
                "quit" => {
                    alive = false;
                    json!({ "return": {}, "id": request["id"] })
                }
                _ => json!({ "return": {}, "id": request["id"] }),
            };
            connection
                .write_all(format!("{}\n", result).as_bytes())
                .expect("QMP response writes");
            drop(reader);
            drop(connection);
        }
        drop(listener);
        for stale in [&socket_path, &state_file, &pid_file] {
            let _ = fs::remove_file(stale);
        }
    }

    #[test]
    fn escapes_qemu_key_value_arguments_and_generates_local_macs() {
        assert_eq!(
            qemu_keyval_escape("/tmp/vm, one\\disk.qcow2"),
            "/tmp/vm\\, one\\\\disk.qcow2"
        );
        assert_eq!(
            mac_address("00000000-0000-0000-0000-000000000001", 0),
            "52:54:00:00:00:01"
        );
        assert_ne!(
            mac_address("00000000-0000-0000-0000-000000000001", 0),
            mac_address("00000000-0000-0000-0000-000000000001", 1)
        );
    }

    #[test]
    fn mixed_runtime_qemu_links_use_the_shared_tap_device() {
        let link = QemuLink {
            link_id: "00000000-0000-0000-0000-000000000001".to_owned(),
            adapter_number: 2,
            multicast_address: "239.1.2.3".parse().unwrap(),
            multicast_port: 20_000,
            tap: Some("nst0123456789ab".to_owned()),
        };
        assert_eq!(
            qemu_netdev_argument(&link, "ns_00000000000000000000000000000001"),
            "tap,id=ns_00000000000000000000000000000001,ifname=nst0123456789ab,script=no,downscript=no"
        );
    }

    #[test]
    fn serial_socket_is_persisted_and_derived_for_older_qemu_nodes() {
        let directory = TempDir::new().expect("temporary QEMU directory");
        let runtime = QemuRuntime::with_paths(
            "qemu-system-x86_64",
            "qemu-img",
            directory.path().join("images"),
            directory.path().join("state"),
            directory.path().join("sockets"),
        );
        let node = json!({
            "project_id": "00000000-0000-4000-8000-000000000001",
            "node_id": "00000000-0000-4000-8000-000000000002",
            "hda_disk_image": "/tmp/router.qcow2",
            "runtime_overlay": "/tmp/router-overlay.qcow2",
            "runtime_qmp_socket": "/tmp/controller/node.sock",
            "runtime_pid_file": "/tmp/node.pid",
            "runtime_log_file": "/tmp/node.log"
        });
        let vm = runtime.vm_from_node(&node).expect("node runtime is valid");
        assert_eq!(
            vm.serial_socket,
            std::path::Path::new(
                "/tmp/controller/00000000-0000-4000-8000-000000000002.serial.sock",
            )
        );
        assert_eq!(
            runtime.runtime_fields(&vm)["runtime_serial_socket"],
            vm.serial_socket.display().to_string()
        );
    }

    #[tokio::test]
    async fn performs_qmp_handshake_and_skips_events_before_matching_reply() {
        let directory = TempDir::new().expect("temporary QMP directory");
        let socket = directory.path().join("qmp.sock");
        let listener = UnixListener::bind(&socket).expect("QMP listener binds");
        let thread = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("QMP client connects");
            use std::io::Write;
            stream
                .write_all(b"{\"QMP\":{\"version\":{}}}\r\n")
                .expect("QMP greeting writes");
            let mut reader = std::io::BufReader::new(stream.try_clone().expect("socket clones"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("capabilities arrive");
            stream
                .write_all(b"{\"return\":{},\"id\":\"capabilities\"}\r\n")
                .expect("capability reply writes");
            line.clear();
            reader.read_line(&mut line).expect("status query arrives");
            stream
                .write_all(b"{\"event\":\"STOP\",\"data\":{}}\r\n{\"return\":{\"status\":\"running\"},\"id\":\"request\"}\r\n")
                .expect("status reply writes");
            thread::sleep(Duration::from_millis(20));
        });
        let response = qmp_execute_inner(&socket, "query-status", None)
            .await
            .expect("QMP status query succeeds");
        assert_eq!(response["status"], "running");
        thread.join().expect("QMP fixture finishes");
    }

    #[tokio::test]
    async fn qemu_templates_are_real_files_inside_the_configured_image_directory() {
        let directory = TempDir::new().expect("temporary QEMU directory");
        let images = directory.path().join("images");
        fs::create_dir_all(&images).expect("image directory exists");
        fs::write(images.join("router.qcow2"), b"test image").expect("image is present");
        fs::write(images.join("notes.txt"), b"ignore").expect("non-image file is present");
        let qemu = directory.path().join("qemu-test-double");
        let qemu_img = directory.path().join("qemu-img-test-double");
        for (path, script) in [
            (
                &qemu,
                "#!/bin/sh\n[ \"$1\" = -version ] && echo 'QEMU emulator version 9.0'\n",
            ),
            (
                &qemu_img,
                "#!/bin/sh\n[ \"$1\" = --version ] && echo 'qemu-img version 9.0'\n",
            ),
        ] {
            fs::write(path, script).expect("write utility double");
            fs::set_permissions(path, fs::Permissions::from_mode(0o755))
                .expect("utility double is executable");
        }
        let runtime = QemuRuntime::with_paths(
            qemu,
            qemu_img,
            &images,
            directory.path().join("state"),
            directory.path().join("sockets"),
        );
        let templates = runtime.templates().await.expect("templates list");
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0]["template_type"], "qemu");
        assert_eq!(templates[0]["name"], "router.qcow2");
        assert!(templates[0]["template_id"].as_str().is_some());
    }

    #[test]
    fn qemu_upload_names_and_formats_are_checked_before_becoming_templates() {
        assert_eq!(
            safe_disk_image_name("router.qcow2").unwrap(),
            "router.qcow2"
        );
        assert!(safe_disk_image_name("../router.qcow2").is_err());
        assert!(safe_disk_image_name("router.iso").is_err());
        assert!(safe_disk_image_name(".hidden.qcow2").is_err());
        assert!(image_format_matches(
            std::path::Path::new("router.qcow2"),
            "qcow2"
        ));
        assert!(image_format_matches(
            std::path::Path::new("router.img"),
            "raw"
        ));
        assert!(!image_format_matches(
            std::path::Path::new("router.qcow2"),
            "raw"
        ));
    }
}
