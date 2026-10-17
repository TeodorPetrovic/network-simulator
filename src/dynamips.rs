use std::{
    collections::HashMap,
    fs,
    io::ErrorKind,
    net::{Ipv4Addr, SocketAddrV4},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use serde::Serialize;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    process::{Child, Command},
    sync::Mutex,
    time::{sleep, timeout},
};
use uuid::Uuid;

use crate::runtime::{NodeAction, RuntimeError};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(8);
const PROCESS_READY_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_HYPERVISOR_RESPONSE: usize = 256 * 1024;
const MAX_HYPERVISOR_LINES: usize = 4096;
const IOS_ELF_HEADER: [u8; 7] = [0x7f, b'E', b'L', b'F', 1, 2, 1];

#[derive(Clone)]
pub struct DynamipsRuntime {
    binary: PathBuf,
    image_dir: PathBuf,
    state_dir: PathBuf,
    routers: Arc<Mutex<HashMap<String, ManagedRouter>>>,
    bindings: Arc<Mutex<HashMap<String, u16>>>,
    operation_lock: Arc<Mutex<()>>,
}

struct ManagedRouter {
    child: Child,
    hypervisor_port: u16,
    log_file: PathBuf,
}

#[derive(Clone, Debug)]
pub struct DynamipsVm {
    pub node_id: String,
    pub name: String,
    pub platform: String,
    pub image: PathBuf,
    pub ram_mb: u64,
    pub idle_pc: Option<String>,
    pub startup_config: String,
    pub working_dir: PathBuf,
    pub hypervisor_port: u16,
    pub console_port: u16,
    pub log_file: PathBuf,
}

#[derive(Serialize)]
pub struct DynamipsRuntimeInfo {
    pub available: bool,
    pub version: Option<String>,
    pub binary: String,
    pub image_directory: String,
    pub image_count: usize,
    pub error: Option<String>,
}

impl DynamipsRuntime {
    pub fn new() -> Self {
        Self {
            binary: crate::image_paths::runtime_binary("NETWORK_SIMULATOR_DYNAMIPS", "dynamips"),
            image_dir: crate::image_paths::dynamips_images(),
            state_dir: std::env::var_os("NETWORK_SIMULATOR_DYNAMIPS_STATE")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("data/dynamips")),
            routers: Arc::new(Mutex::new(HashMap::new())),
            bindings: Arc::new(Mutex::new(HashMap::new())),
            operation_lock: Arc::new(Mutex::new(())),
        }
    }

    #[cfg(test)]
    pub fn with_paths(
        binary: impl Into<PathBuf>,
        image_dir: impl Into<PathBuf>,
        state_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            binary: binary.into(),
            image_dir: image_dir.into(),
            state_dir: state_dir.into(),
            routers: Arc::new(Mutex::new(HashMap::new())),
            bindings: Arc::new(Mutex::new(HashMap::new())),
            operation_lock: Arc::new(Mutex::new(())),
        }
    }

    pub async fn info(&self) -> DynamipsRuntimeInfo {
        let version = self.version().await;
        let images = self.image_files().unwrap_or_default();
        let image_directory = self
            .image_dir
            .canonicalize()
            .unwrap_or_else(|_| self.image_dir.clone())
            .display()
            .to_string();
        DynamipsRuntimeInfo {
            available: version.is_ok(),
            version: version.as_ref().ok().cloned(),
            binary: self.binary.display().to_string(),
            image_directory,
            image_count: images.len(),
            error: version.err().map(|error| error.message().to_owned()),
        }
    }

    pub async fn templates(&self) -> Result<Vec<Value>, RuntimeError> {
        self.version().await?;
        let mut templates = Vec::new();
        for image in self.image_files()? {
            let name = image
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("Cisco IOS image");
            let template_id =
                Uuid::new_v5(&Uuid::NAMESPACE_URL, image.to_string_lossy().as_bytes());
            templates.push(json!({
                "template_id": template_id,
                "name": format!("{name} · c7200"),
                "template_type": "dynamips",
                "compute_id": "local",
                "category": "router",
                "symbol": "router",
                "builtin": false,
                "platform": "c7200",
                "image": image,
                "adapters": 1,
                "memory": 256,
                "nvram": 128,
                "idlepc": ""
            }));
        }
        templates.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
        Ok(templates)
    }

    pub fn upload_paths(&self, file_name: &str) -> Result<(PathBuf, PathBuf), RuntimeError> {
        let safe_name = safe_file_name(file_name)?;
        fs::create_dir_all(&self.image_dir).map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not create Dynamips image directory '{}': {error}",
                self.image_dir.display()
            ))
        })?;
        let directory = self.image_dir.canonicalize().map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not resolve Dynamips image directory: {error}"
            ))
        })?;
        let final_path = directory.join(&safe_name);
        if final_path.exists() {
            return Err(RuntimeError::Failed(format!(
                "A Dynamips image named '{}' already exists",
                final_path.file_name().unwrap_or_default().to_string_lossy()
            )));
        }
        let temporary_path = directory.join(format!(".{}.{}.upload", Uuid::new_v4(), safe_name));
        Ok((temporary_path, final_path))
    }

    pub fn finalize_uploaded_image(
        &self,
        temporary_path: &Path,
        final_path: &Path,
    ) -> Result<PathBuf, RuntimeError> {
        validate_ios_image(temporary_path)?;
        fs::hard_link(temporary_path, final_path).map_err(|error| {
            RuntimeError::Failed(format!("Could not install Cisco IOS image: {error}"))
        })?;
        fs::remove_file(temporary_path).map_err(|error| {
            let _ = fs::remove_file(final_path);
            RuntimeError::Failed(format!(
                "Could not finalize Cisco IOS image upload: {error}"
            ))
        })?;
        final_path.canonicalize().map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not resolve installed Cisco IOS image: {error}"
            ))
        })
    }

    pub async fn create_router(
        &self,
        project_id: &str,
        node_id: &str,
        name: &str,
        image: &str,
        platform: &str,
        ram_mb: u64,
        idle_pc: Option<&str>,
        startup_config: &str,
    ) -> Result<DynamipsVm, RuntimeError> {
        let _guard = self.operation_lock.lock().await;
        let project_id = parse_id(project_id)?;
        let node_id = parse_id(node_id)?;
        validate_router_name(name)?;
        if platform != "c7200" {
            return Err(RuntimeError::Failed(
                "This Dynamips runtime currently supports the GNS3 c7200 platform".to_owned(),
            ));
        }
        if !(128..=2048).contains(&ram_mb) {
            return Err(RuntimeError::Failed(
                "Dynamips c7200 RAM must be between 128 and 2048 MB".to_owned(),
            ));
        }
        let idle_pc = idle_pc
            .filter(|value| !value.trim().is_empty())
            .map(|value| {
                if is_hex_pointer(value) {
                    Ok(value.to_owned())
                } else {
                    Err(RuntimeError::Failed(
                        "Dynamips idle-PC must be a hexadecimal address such as 0x606e0538"
                            .to_owned(),
                    ))
                }
            })
            .transpose()?;
        let image = self.resolve_image(image)?;
        let working_dir = self
            .state_dir
            .join(project_id.to_string())
            .join(node_id.to_string());
        fs::create_dir_all(working_dir.join("configs")).map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not create Dynamips router directory: {error}"
            ))
        })?;
        fs::write(
            working_dir.join("configs/i1_startup-config.cfg"),
            startup_config,
        )
        .map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not write Dynamips startup configuration: {error}"
            ))
        })?;

        let console_port = available_tcp_port().await?;
        let vm = DynamipsVm {
            node_id: node_id.to_string(),
            name: name.to_owned(),
            platform: platform.to_owned(),
            image,
            ram_mb,
            idle_pc,
            startup_config: startup_config.to_owned(),
            log_file: working_dir.join("dynamips_stdout.log"),
            working_dir,
            hypervisor_port: available_tcp_port().await?,
            console_port,
        };
        self.initialize_router(&vm).await?;
        Ok(vm)
    }

    pub fn vm_from_node(&self, node: &Value) -> Result<DynamipsVm, RuntimeError> {
        parse_id(node["project_id"].as_str().unwrap_or_default())?;
        Ok(DynamipsVm {
            node_id: parse_id(node["node_id"].as_str().unwrap_or_default())?.to_string(),
            name: required_string(node, "name")?.to_owned(),
            platform: required_string(node, "platform")?.to_owned(),
            image: PathBuf::from(required_string(node, "image")?),
            ram_mb: node["ram"].as_u64().unwrap_or(256),
            idle_pc: node["idlepc"]
                .as_str()
                .filter(|value| !value.is_empty())
                .map(str::to_owned),
            startup_config: node["startup_config"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            working_dir: PathBuf::from(required_string(node, "runtime_working_dir")?),
            hypervisor_port: node["runtime_hypervisor_port"]
                .as_u64()
                .and_then(|port| u16::try_from(port).ok())
                .filter(|port| *port != 0)
                .ok_or_else(|| {
                    RuntimeError::Failed("Dynamips node has no hypervisor port".to_owned())
                })?,
            console_port: node["console"]
                .as_u64()
                .and_then(|port| u16::try_from(port).ok())
                .unwrap_or(0),
            log_file: PathBuf::from(required_string(node, "runtime_log_file")?),
        })
    }

    pub fn runtime_fields(&self, vm: &DynamipsVm) -> Value {
        json!({
            "platform": vm.platform,
            "image": vm.image,
            "ram": vm.ram_mb,
            "idlepc": vm.idle_pc,
            "console": vm.console_port,
            "runtime_hypervisor_port": vm.hypervisor_port,
            "runtime_working_dir": vm.working_dir,
            "runtime_log_file": vm.log_file,
            "adapter_types": ["C7200-IO-FE"]
        })
    }

    pub async fn lifecycle(
        &self,
        vm: &DynamipsVm,
        action: NodeAction,
    ) -> Result<String, RuntimeError> {
        let _guard = self.operation_lock.lock().await;
        match action {
            NodeAction::Start => self.start_locked(vm).await,
            NodeAction::Stop => self.stop_locked(vm).await,
            NodeAction::Suspend => {
                let status = self.hypervisor_status(vm.hypervisor_port, &vm.name).await?;
                if status == "started" {
                    self.send(
                        vm.hypervisor_port,
                        &format!("vm suspend {}", quote(&vm.name)),
                    )
                    .await?;
                }
                self.hypervisor_status(vm.hypervisor_port, &vm.name).await
            }
            NodeAction::Reload => {
                self.stop_locked(vm).await?;
                self.start_locked(vm).await
            }
        }
    }

    pub async fn status(&self, vm: &DynamipsVm) -> Result<String, RuntimeError> {
        self.hypervisor_status(vm.hypervisor_port, &vm.name).await
    }

    pub async fn console(&self, vm: &DynamipsVm) -> Result<TcpStream, RuntimeError> {
        if vm.console_port == 0 || self.status(vm).await? != "started" {
            return Err(RuntimeError::Failed(
                "Start the Dynamips router before opening its console".to_owned(),
            ));
        }
        timeout(
            Duration::from_secs(3),
            TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, vm.console_port)),
        )
        .await
        .map_err(|_| RuntimeError::Unavailable("Dynamips console connection timed out".to_owned()))?
        .map_err(|error| {
            RuntimeError::Unavailable(format!(
                "Could not connect to the Dynamips console: {error}"
            ))
        })
    }

    pub async fn remove_router(&self, vm: &DynamipsVm) -> Result<(), RuntimeError> {
        let _guard = self.operation_lock.lock().await;
        let mut first_error = None;
        if self.router_process_alive(&vm.node_id).await {
            if let Err(error) = self
                .send(
                    vm.hypervisor_port,
                    &format!("vm clean_delete {}", quote(&vm.name)),
                )
                .await
            {
                first_error = Some(error);
            }
            let _ = self.send(vm.hypervisor_port, "hypervisor stop").await;
        }
        if let Some(mut router) = self.routers.lock().await.remove(&vm.node_id) {
            if router.child.try_wait().ok().flatten().is_none() {
                let _ = router.child.start_kill();
                let _ = timeout(Duration::from_secs(3), router.child.wait()).await;
            }
            let _ = fs::remove_file(router.log_file);
        }
        self.bindings.lock().await.remove(&vm.node_id);
        if vm.working_dir.exists() {
            fs::remove_dir_all(&vm.working_dir).map_err(|error| {
                RuntimeError::Failed(format!("Could not remove Dynamips node files: {error}"))
            })?;
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(())
    }

    pub async fn switch_endpoint(
        &self,
        vm: &DynamipsVm,
        adapter_number: u32,
        port_number: u32,
        switch_port: u16,
    ) -> Result<u16, RuntimeError> {
        let _guard = self.operation_lock.lock().await;
        if adapter_number != 0 || port_number != 0 {
            return Err(RuntimeError::Failed(
                "The c7200 FastEthernet controller provides adapter 0, port 0".to_owned(),
            ));
        }
        self.ensure_hypervisor(vm).await?;
        if let Some(port) = self.bindings.lock().await.get(&vm.node_id).copied() {
            return Ok(port);
        }
        let local_port = available_udp_port().await?;
        let nio_name = format!("udp-{}", vm.node_id.replace('-', ""));
        self.send(
            vm.hypervisor_port,
            &format!("nio create_udp {nio_name} {local_port} 127.0.0.1 {switch_port}"),
        )
        .await?;
        self.send(
            vm.hypervisor_port,
            &format!("vm slot_add_nio_binding {} 0 0 {nio_name}", quote(&vm.name)),
        )
        .await?;
        if self.hypervisor_status(vm.hypervisor_port, &vm.name).await? == "started" {
            self.send(
                vm.hypervisor_port,
                &format!("vm slot_enable_nio {} 0 0", quote(&vm.name)),
            )
            .await?;
        }
        self.bindings
            .lock()
            .await
            .insert(vm.node_id.clone(), local_port);
        Ok(local_port)
    }

    pub async fn remove_switch_endpoint(&self, vm: &DynamipsVm) -> Result<(), RuntimeError> {
        let _guard = self.operation_lock.lock().await;
        let binding = self.bindings.lock().await.remove(&vm.node_id);
        if self.router_process_alive(&vm.node_id).await {
            if binding.is_none() {
                return Ok(());
            }
            let nio_name = format!("udp-{}", vm.node_id.replace('-', ""));
            let _ = self
                .send(
                    vm.hypervisor_port,
                    &format!("vm slot_disable_nio {} 0 0", quote(&vm.name)),
                )
                .await;
            let _ = self
                .send(
                    vm.hypervisor_port,
                    &format!("vm slot_remove_nio_binding {} 0 0", quote(&vm.name)),
                )
                .await;
            let _ = self
                .send(vm.hypervisor_port, &format!("nio delete {nio_name}"))
                .await;
        }
        Ok(())
    }

    pub fn prepare_upload_name(&self, file_name: &str) -> Result<(PathBuf, PathBuf), RuntimeError> {
        self.upload_paths(file_name)
    }

    fn image_files(&self) -> Result<Vec<PathBuf>, RuntimeError> {
        let directory = match self.image_dir.canonicalize() {
            Ok(directory) => directory,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(RuntimeError::Failed(format!(
                    "Could not read Dynamips image directory '{}': {error}",
                    self.image_dir.display()
                )));
            }
        };
        let entries = fs::read_dir(&directory).map_err(|error| {
            RuntimeError::Failed(format!("Could not list Dynamips IOS images: {error}"))
        })?;
        let mut images = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                RuntimeError::Failed(format!("Could not read Dynamips image entry: {error}"))
            })?;
            let path = entry.path();
            if !path.is_file() || validate_ios_image(&path).is_err() {
                continue;
            }
            images.push(path.canonicalize().map_err(|error| {
                RuntimeError::Failed(format!("Could not resolve IOS image path: {error}"))
            })?);
        }
        images.sort();
        Ok(images)
    }

    fn resolve_image(&self, image: &str) -> Result<PathBuf, RuntimeError> {
        let root = self.image_dir.canonicalize().map_err(|error| {
            RuntimeError::Unavailable(format!(
                "Dynamips IOS image directory '{}' is unavailable: {error}",
                self.image_dir.display()
            ))
        })?;
        let path = Path::new(image).canonicalize().map_err(|error| {
            RuntimeError::NotFound(format!("Cisco IOS image '{image}' is unavailable: {error}"))
        })?;
        if !path.starts_with(&root) {
            return Err(RuntimeError::Failed(
                "Cisco IOS images must be inside the configured Dynamips image directory"
                    .to_owned(),
            ));
        }
        validate_ios_image(&path)?;
        Ok(path)
    }

    async fn version(&self) -> Result<String, RuntimeError> {
        let output = timeout(
            COMMAND_TIMEOUT,
            Command::new(&self.binary)
                .arg("-P")
                .arg("none")
                .stdin(Stdio::null())
                .output(),
        )
        .await
        .map_err(|_| RuntimeError::Unavailable("Dynamips version check timed out".to_owned()))?
        .map_err(|error| {
            RuntimeError::Unavailable(format!(
                "Could not run Dynamips executable '{}': {error}",
                self.binary.display()
            ))
        })?;
        // Some distro builds print their version after rejecting the `none`
        // platform used by GNS3's probe, and exit nonzero. Accept only a
        // version banner in that case; the real hypervisor is probed at launch.
        let output = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
            .lines()
            .find(|line| line.contains("Cisco Router Simulation Platform (version "))
            .and_then(|line| line.split("(version ").nth(1))
            .and_then(|value| value.split(')').next())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                RuntimeError::Unavailable(format!(
                    "Could not determine a supported Dynamips version from '{}': {}",
                    self.binary.display(),
                    output.trim()
                ))
            })
            .and_then(|version| {
                if supported_dynamips_version(&version) {
                    Ok(version)
                } else {
                    Err(RuntimeError::Unavailable(format!(
                        "Dynamips version 0.2.11 or newer is required, detected {version}"
                    )))
                }
            })
    }

    async fn initialize_router(&self, vm: &DynamipsVm) -> Result<(), RuntimeError> {
        let router = self.spawn_hypervisor(vm).await?;
        let port = router.hypervisor_port;
        self.routers.lock().await.insert(vm.node_id.clone(), router);
        let initialize = async {
            self.send(port, "hypervisor version").await?;
            self.send(
                port,
                &format!(
                    "hypervisor working_dir {}",
                    quote(&vm.working_dir.display().to_string())
                ),
            )
            .await?;
            self.send(
                port,
                &format!("vm create {} 1 {}", quote(&vm.name), vm.platform),
            )
            .await?;
            self.send(
                port,
                &format!(
                    "vm set_ios {} {}",
                    quote(&vm.name),
                    quote(&vm.image.display().to_string())
                ),
            )
            .await?;
            self.send(
                port,
                &format!("vm set_ram {} {}", quote(&vm.name), vm.ram_mb),
            )
            .await?;
            self.send(port, &format!("vm set_nvram {} 128", quote(&vm.name)))
                .await?;
            self.send(port, &format!("vm set_clock_divisor {} 4", quote(&vm.name)))
                .await?;
            self.send(
                port,
                &format!(
                    "vm set_con_tcp_port {} {}",
                    quote(&vm.name),
                    vm.console_port
                ),
            )
            .await?;
            self.send(
                port,
                &format!("vm slot_add_binding {} 0 0 C7200-IO-FE", quote(&vm.name)),
            )
            .await?;
            if let Some(idle_pc) = &vm.idle_pc {
                self.send(
                    port,
                    &format!("vm set_idle_pc {} {idle_pc}", quote(&vm.name)),
                )
                .await?;
            }
            Ok::<(), RuntimeError>(())
        }
        .await;
        if let Err(error) = initialize {
            let _ = self.stop_process(vm).await;
            return Err(error);
        }
        Ok(())
    }

    async fn start_locked(&self, vm: &DynamipsVm) -> Result<String, RuntimeError> {
        self.ensure_hypervisor(vm).await?;
        match self.hypervisor_status(vm.hypervisor_port, &vm.name).await? {
            status if status == "started" => return Ok(status),
            status if status == "suspended" => {
                self.send(
                    vm.hypervisor_port,
                    &format!("vm resume {}", quote(&vm.name)),
                )
                .await?;
                return self.hypervisor_status(vm.hypervisor_port, &vm.name).await;
            }
            _ => {}
        }
        validate_ios_image(&vm.image)?;
        fs::write(
            vm.working_dir.join("configs/i1_startup-config.cfg"),
            &vm.startup_config,
        )
        .map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not write Dynamips startup configuration: {error}"
            ))
        })?;
        self.send(
            vm.hypervisor_port,
            &format!(
                "vm set_config {} {} \"\"",
                quote(&vm.name),
                quote("configs/i1_startup-config.cfg")
            ),
        )
        .await?;
        self.send(vm.hypervisor_port, &format!("vm start {}", quote(&vm.name)))
            .await?;
        self.hypervisor_status(vm.hypervisor_port, &vm.name).await
    }

    async fn stop_locked(&self, vm: &DynamipsVm) -> Result<String, RuntimeError> {
        let status = self.hypervisor_status(vm.hypervisor_port, &vm.name).await?;
        if status != "stopped" {
            self.send(vm.hypervisor_port, &format!("vm stop {}", quote(&vm.name)))
                .await?;
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let status = self.hypervisor_status(vm.hypervisor_port, &vm.name).await?;
            if status == "stopped" || tokio::time::Instant::now() >= deadline {
                return Ok(status);
            }
            sleep(Duration::from_millis(100)).await;
        }
    }

    async fn ensure_hypervisor(&self, vm: &DynamipsVm) -> Result<(), RuntimeError> {
        if self.router_process_alive(&vm.node_id).await
            && self
                .hypervisor_status(vm.hypervisor_port, &vm.name)
                .await
                .is_ok()
        {
            return Ok(());
        }
        if let Some(mut stale) = self.routers.lock().await.remove(&vm.node_id) {
            if stale.child.try_wait().ok().flatten().is_none() {
                let _ = stale.child.start_kill();
                let _ = timeout(Duration::from_secs(2), stale.child.wait()).await;
            }
            let _ = fs::remove_file(stale.log_file);
        }
        self.bindings.lock().await.remove(&vm.node_id);
        self.initialize_router(vm).await
    }

    async fn spawn_hypervisor(&self, vm: &DynamipsVm) -> Result<ManagedRouter, RuntimeError> {
        fs::create_dir_all(&vm.working_dir).map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not create Dynamips working directory: {error}"
            ))
        })?;
        let log_file = vm.log_file.clone();
        let stdout = fs::File::create(&log_file).map_err(|error| {
            RuntimeError::Failed(format!("Could not create Dynamips log file: {error}"))
        })?;
        let stderr = stdout.try_clone().map_err(|error| {
            RuntimeError::Failed(format!("Could not open Dynamips log file: {error}"))
        })?;
        let port = vm.hypervisor_port;
        let mut child = Command::new(&self.binary)
            .arg("-N1")
            .arg("-l")
            .arg("dynamips_engine.log")
            .arg("-H")
            .arg(format!("127.0.0.1:{port}"))
            .current_dir(&vm.working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                RuntimeError::Unavailable(format!(
                    "Could not start Dynamips executable '{}': {error}",
                    self.binary.display()
                ))
            })?;
        let deadline = tokio::time::Instant::now() + PROCESS_READY_TIMEOUT;
        loop {
            if TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))
                .await
                .is_ok()
            {
                return Ok(ManagedRouter {
                    child,
                    hypervisor_port: port,
                    log_file,
                });
            }
            if let Some(status) = child.try_wait().map_err(|error| {
                RuntimeError::Unavailable(format!("Could not inspect Dynamips startup: {error}"))
            })? {
                let log = fs::read_to_string(&log_file).unwrap_or_default();
                return Err(RuntimeError::Failed(format!(
                    "Dynamips exited during startup ({status}){}",
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
                let log = fs::read_to_string(&log_file).unwrap_or_default();
                return Err(RuntimeError::Unavailable(format!(
                    "Dynamips hypervisor did not listen on 127.0.0.1:{port}{}",
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

    async fn stop_process(&self, vm: &DynamipsVm) -> Result<(), RuntimeError> {
        let _ = self.send(vm.hypervisor_port, "hypervisor stop").await;
        if let Some(mut router) = self.routers.lock().await.remove(&vm.node_id) {
            if timeout(Duration::from_secs(3), router.child.wait())
                .await
                .is_err()
            {
                let _ = router.child.start_kill();
                let _ = timeout(Duration::from_secs(2), router.child.wait()).await;
            }
        }
        Ok(())
    }

    async fn router_process_alive(&self, node_id: &str) -> bool {
        let mut routers = self.routers.lock().await;
        let Some(router) = routers.get_mut(node_id) else {
            return false;
        };
        match router.child.try_wait() {
            Ok(None) => true,
            Ok(Some(_)) | Err(_) => false,
        }
    }

    async fn hypervisor_status(&self, port: u16, name: &str) -> Result<String, RuntimeError> {
        let response = self
            .send(port, &format!("vm get_status {}", quote(name)))
            .await?;
        match response.first().map(String::as_str) {
            Some("0") => Ok("stopped".to_owned()),
            Some("1") => Ok("stopping".to_owned()),
            Some("2") => Ok("started".to_owned()),
            Some("3") => Ok("suspended".to_owned()),
            _ => Err(RuntimeError::Failed(format!(
                "Dynamips returned an invalid router status: {}",
                response.join(" ")
            ))),
        }
    }

    async fn send(&self, port: u16, command: &str) -> Result<Vec<String>, RuntimeError> {
        timeout(COMMAND_TIMEOUT, async {
            let mut stream = TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))
                .await
                .map_err(|error| {
                    RuntimeError::Unavailable(format!(
                        "Could not connect to Dynamips hypervisor on 127.0.0.1:{port}: {error}"
                    ))
                })?;
            stream
                .write_all(format!("{command}\n").as_bytes())
                .await
                .map_err(|error| {
                    RuntimeError::Unavailable(format!("Could not send Dynamips command: {error}"))
                })?;
            stream.shutdown().await.ok();
            let mut reader = BufReader::new(stream);
            let mut response = Vec::new();
            let mut total_bytes = 0;
            for _ in 0..MAX_HYPERVISOR_LINES {
                let mut line = String::new();
                let length = reader.read_line(&mut line).await.map_err(|error| {
                    RuntimeError::Unavailable(format!(
                        "Could not read Dynamips response for '{command}': {error}"
                    ))
                })?;
                if length == 0 {
                    return Err(RuntimeError::Unavailable(format!(
                        "Dynamips closed the connection before completing '{command}'"
                    )));
                }
                total_bytes += length;
                if total_bytes > MAX_HYPERVISOR_RESPONSE {
                    return Err(RuntimeError::Failed(
                        "Dynamips hypervisor response exceeded the 256 KiB limit".to_owned(),
                    ));
                }
                let line = line.trim_end_matches(['\r', '\n']);
                if line.starts_with("2") && line.as_bytes().get(3) == Some(&b'-') {
                    return Err(RuntimeError::Failed(format!(
                        "Dynamips rejected '{command}': {}",
                        line.get(4..).unwrap_or(line)
                    )));
                }
                if line.starts_with("100-") {
                    let message = line.get(4..).unwrap_or_default();
                    if message != "OK" && !message.is_empty() {
                        response.push(message.to_owned());
                    }
                    return Ok(response);
                }
                if line.as_bytes().get(3) == Some(&b' ') {
                    response.push(line.get(4..).unwrap_or_default().to_owned());
                } else if !line.is_empty() {
                    response.push(line.to_owned());
                }
            }
            Err(RuntimeError::Failed(format!(
                "Dynamips returned too many lines for '{command}'"
            )))
        })
        .await
        .map_err(|_| RuntimeError::Unavailable(format!("Dynamips command timed out: {command}")))?
    }
}

impl Default for DynamipsRuntime {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn validate_ios_image(path: &Path) -> Result<(), RuntimeError> {
    let mut file = fs::File::open(path).map_err(|error| {
        RuntimeError::NotFound(format!(
            "Cisco IOS image '{}' is unavailable: {error}",
            path.display()
        ))
    })?;
    let mut header = [0; IOS_ELF_HEADER.len()];
    use std::io::Read;
    file.read_exact(&mut header).map_err(|error| {
        RuntimeError::Failed(format!(
            "Cisco IOS image '{}' is too small: {error}",
            path.display()
        ))
    })?;
    if header != IOS_ELF_HEADER {
        return Err(RuntimeError::Failed(format!(
            "'{}' is not a supported Cisco IOS ELF image (Dynamips expects 32-bit big-endian ELF)",
            path.display()
        )));
    }
    Ok(())
}

fn safe_file_name(file_name: &str) -> Result<String, RuntimeError> {
    if file_name.contains(['/', '\\']) {
        return Err(RuntimeError::Failed(
            "IOS upload filename must be a single file name".to_owned(),
        ));
    }
    let name = Path::new(file_name)
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| RuntimeError::Failed("IOS upload filename is invalid".to_owned()))?;
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.len() > 180
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(RuntimeError::Failed(
            "IOS filename may contain only letters, numbers, dot, underscore, and hyphen"
                .to_owned(),
        ));
    }
    Ok(name.to_owned())
}

fn validate_router_name(name: &str) -> Result<(), RuntimeError> {
    if name.is_empty()
        || name.len() > 63
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(RuntimeError::Failed(
            "Dynamips router names may contain up to 63 letters, numbers, dot, underscore, and hyphen".to_owned(),
        ));
    }
    Ok(())
}

fn is_hex_pointer(value: &str) -> bool {
    value.strip_prefix("0x").is_some_and(|digits| {
        !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn supported_dynamips_version(version: &str) -> bool {
    let mut parts = version.split('.');
    let Some(major) = parts.next().and_then(|part| part.parse::<u32>().ok()) else {
        return false;
    };
    let Some(minor) = parts.next().and_then(|part| part.parse::<u32>().ok()) else {
        return false;
    };
    let patch = parts
        .next()
        .unwrap_or_default()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse::<u32>()
        .unwrap_or(0);
    (major, minor, patch) >= (0, 2, 11)
}

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn required_string<'a>(node: &'a Value, key: &str) -> Result<&'a str, RuntimeError> {
    node.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| RuntimeError::Failed(format!("Dynamips node is missing '{key}'")))
}

fn parse_id(id: &str) -> Result<Uuid, RuntimeError> {
    Uuid::parse_str(id)
        .map_err(|_| RuntimeError::Failed("Dynamips IDs must be valid UUIDs".to_owned()))
}

async fn available_tcp_port() -> Result<u16, RuntimeError> {
    let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not allocate a TCP port: {error}"))
        })?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not read allocated TCP port: {error}"))
        })
}

async fn available_udp_port() -> Result<u16, RuntimeError> {
    let socket = tokio::net::UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not allocate a UDP port: {error}"))
        })?;
    socket
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not read allocated UDP port: {error}"))
        })
}

#[cfg(test)]
mod tests {
    use super::{DynamipsRuntime, safe_file_name, supported_dynamips_version, validate_ios_image};
    use std::{
        env, fs,
        net::{Ipv4Addr, SocketAddrV4},
    };

    #[test]
    fn validates_dynamips_compatible_ios_elf_header() {
        let directory = tempfile::tempdir().unwrap();
        let image = directory.path().join("router.image");
        fs::write(&image, [0x7f, b'E', b'L', b'F', 1, 2, 1, 0]).unwrap();
        assert!(validate_ios_image(&image).is_ok());
        fs::write(&image, b"not an IOS image").unwrap();
        assert!(validate_ios_image(&image).is_err());
    }

    #[test]
    fn upload_names_cannot_escape_or_overwrite_the_image_directory() {
        assert_eq!(safe_file_name("router.image").unwrap(), "router.image");
        assert!(safe_file_name("../router.image").is_err());
        assert!(safe_file_name("router image.image").is_err());
    }

    #[test]
    fn requires_the_minimum_hypervisor_version_supported_by_gns3() {
        assert!(!supported_dynamips_version("0.2.10"));
        assert!(supported_dynamips_version("0.2.11"));
        assert!(supported_dynamips_version("0.2.14-amd64/Linux stable"));
        assert!(supported_dynamips_version("0.3.0"));
        assert!(!supported_dynamips_version("unknown"));
    }

    #[tokio::test]
    #[ignore = "requires the real Dynamips executable; set NETWORK_SIMULATOR_TEST_DYNAMIPS and run with --ignored"]
    async fn upstream_dynamips_creates_and_reports_a_real_c7200_hypervisor_instance() {
        let binary = env::var_os("NETWORK_SIMULATOR_TEST_DYNAMIPS")
            .expect("NETWORK_SIMULATOR_TEST_DYNAMIPS points to upstream Dynamips");
        let directory = tempfile::tempdir().unwrap();
        let runtime = DynamipsRuntime::with_paths(
            binary,
            directory.path().join("ios"),
            directory.path().join("state"),
        );
        fs::create_dir_all(directory.path().join("ios")).unwrap();
        let working_dir = directory.path().join("state/c7200-probe");
        fs::create_dir_all(&working_dir).unwrap();
        let vm = super::DynamipsVm {
            node_id: "00000000-0000-0000-0000-00000000000a".to_owned(),
            name: "R1".to_owned(),
            platform: "c7200".to_owned(),
            image: directory.path().join("IOS image supplied by the operator"),
            ram_mb: 256,
            idle_pc: None,
            startup_config: String::new(),
            working_dir: working_dir.clone(),
            hypervisor_port: super::available_tcp_port().await.unwrap(),
            console_port: super::available_tcp_port().await.unwrap(),
            log_file: working_dir.join("dynamips.log"),
        };
        let router = runtime
            .spawn_hypervisor(&vm)
            .await
            .expect("real Dynamips hypervisor starts");
        runtime
            .routers
            .lock()
            .await
            .insert(vm.node_id.clone(), router);
        runtime
            .send(vm.hypervisor_port, "hypervisor version")
            .await
            .unwrap();
        runtime
            .send(
                vm.hypervisor_port,
                &format!(
                    "hypervisor working_dir {}",
                    super::quote(&working_dir.display().to_string())
                ),
            )
            .await
            .unwrap();
        runtime
            .send(
                vm.hypervisor_port,
                &format!("vm create {} 1 c7200", super::quote(&vm.name)),
            )
            .await
            .unwrap();
        runtime
            .send(
                vm.hypervisor_port,
                &format!(
                    "vm slot_add_binding {} 0 0 C7200-IO-FE",
                    super::quote(&vm.name)
                ),
            )
            .await
            .unwrap();
        assert_eq!(runtime.status(&vm).await.unwrap(), "stopped");
        let switch_socket = tokio::net::UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let switch_port = switch_socket.local_addr().unwrap().port();
        let router_udp_port = runtime
            .switch_endpoint(&vm, 0, 0, switch_port)
            .await
            .unwrap();
        assert_ne!(router_udp_port, 0);
        runtime.remove_switch_endpoint(&vm).await.unwrap();
        runtime
            .send(
                vm.hypervisor_port,
                &format!("vm clean_delete {}", super::quote(&vm.name)),
            )
            .await
            .unwrap();
        runtime.stop_process(&vm).await.unwrap();
    }
}
