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

use serde::{Deserialize, Serialize};
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
const RUNTIME_MANIFEST_FILE: &str = ".dynamips-runtime";

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
    pub chassis: Option<String>,
    pub slot0: String,
    pub image: PathBuf,
    pub ram_mb: u64,
    pub nvram: u64,
    pub idle_pc: Option<String>,
    pub startup_config: String,
    pub working_dir: PathBuf,
    pub hypervisor_port: u16,
    pub console_port: u16,
    pub aux_port: Option<u16>,
    pub log_file: PathBuf,
    pub options: DynamipsOptions,
    pub ghost_file: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct DynamipsOptions {
    pub dynamips_id: u32,
    pub clock_divisor: u32,
    pub mmap: bool,
    pub sparsemem: bool,
    pub idlemax: u32,
    pub idlesleep: u32,
    pub exec_area: u32,
    pub disk0: u32,
    pub disk1: u32,
    pub auto_delete_disks: bool,
    #[serde(alias = "private_config_content")]
    pub private_config: String,
    pub mac_addr: Option<String>,
    pub system_id: Option<String>,
    pub slots: Vec<Option<String>>,
    pub wics: Vec<Option<String>>,
    pub npe: Option<String>,
    pub midplane: Option<String>,
    pub iomem: Option<u8>,
    pub ghost_ios: bool,
    pub console_type: String,
    pub aux_type: String,
}

impl Default for DynamipsOptions {
    fn default() -> Self {
        Self {
            dynamips_id: 1,
            clock_divisor: 8,
            mmap: true,
            sparsemem: true,
            idlemax: 500,
            idlesleep: 30,
            exec_area: 64,
            disk0: 0,
            disk1: 0,
            auto_delete_disks: false,
            private_config: String::new(),
            mac_addr: None,
            system_id: Some("FTX0945W0MY".to_owned()),
            slots: vec![None; 7],
            wics: vec![None; 3],
            npe: Some("npe-400".to_owned()),
            midplane: Some("vxr".to_owned()),
            iomem: Some(5),
            ghost_ios: false,
            console_type: "telnet".to_owned(),
            aux_type: "none".to_owned(),
        }
    }
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
                "idlepc": "",
                "mmap": true,
                "sparsemem": true,
                "clock_divisor": 8,
                "idlemax": 500,
                "idlesleep": 30,
                "exec_area": 64,
                "disk0": 0,
                "disk1": 0,
                "auto_delete_disks": false,
                "private_config": "",
                "mac_addr": "",
                "system_id": "FTX0945W0MY",
                "slot0": "C7200-IO-FE",
                "slot1": null,
                "slot2": null,
                "slot3": null,
                "slot4": null,
                "slot5": null,
                "slot6": null,
                "wic0": null,
                "wic1": null,
                "wic2": null,
                "npe": "npe-400",
                "midplane": "vxr",
                "iomem": null,
                "ghost_ios": false,
                "console_type": "telnet",
                "aux_type": "none"
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
        chassis: Option<&str>,
        slot0: Option<&str>,
        ram_mb: u64,
        nvram: u64,
        idle_pc: Option<&str>,
        startup_config: &str,
    ) -> Result<DynamipsVm, RuntimeError> {
        self.create_router_with_options(
            project_id,
            node_id,
            name,
            image,
            platform,
            chassis,
            slot0,
            ram_mb,
            nvram,
            idle_pc,
            startup_config,
            &DynamipsOptions::default(),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_router_with_options(
        &self,
        project_id: &str,
        node_id: &str,
        name: &str,
        image: &str,
        platform: &str,
        chassis: Option<&str>,
        slot0: Option<&str>,
        ram_mb: u64,
        nvram: u64,
        idle_pc: Option<&str>,
        startup_config: &str,
        options: &DynamipsOptions,
    ) -> Result<DynamipsVm, RuntimeError> {
        let _guard = self.operation_lock.lock().await;
        let project_id = parse_id(project_id)?;
        let node_id = parse_id(node_id)?;
        validate_router_name(name)?;
        validate_platform_settings(platform, chassis, slot0)?;
        validate_dynamips_options(platform, chassis, slot0, options)?;
        if !(128..=2048).contains(&ram_mb) {
            return Err(RuntimeError::Failed(
                "Dynamips router RAM must be between 128 and 2048 MB".to_owned(),
            ));
        }
        if !(64..=2048).contains(&nvram) {
            return Err(RuntimeError::Failed(
                "Dynamips router NVRAM must be between 64 and 2048 KB".to_owned(),
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
            working_dir.join(format!(
                "configs/i{}_startup-config.cfg",
                options.dynamips_id
            )),
            startup_config,
        )
        .map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not write Dynamips startup configuration: {error}"
            ))
        })?;
        if !options.private_config.is_empty() {
            fs::write(
                working_dir.join(format!(
                    "configs/i{}_private-config.cfg",
                    options.dynamips_id
                )),
                &options.private_config,
            )
            .map_err(|error| {
                RuntimeError::Failed(format!(
                    "Could not write Dynamips private configuration: {error}"
                ))
            })?;
        }

        let console_port = available_tcp_port().await?;
        let aux_port = if options.aux_type != "none" {
            Some(available_tcp_port().await?)
        } else {
            None
        };
        let ghost_file = if options.ghost_ios && options.mmap {
            let file_name = image
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("ios");
            Some(
                self.state_dir
                    .join("ghosts")
                    .join(format!("{file_name}-{}.ghost", ram_mb)),
            )
        } else {
            None
        };
        let vm = DynamipsVm {
            node_id: node_id.to_string(),
            name: name.to_owned(),
            platform: platform.to_owned(),
            chassis: chassis.map(str::to_owned),
            slot0: options
                .slots
                .first()
                .and_then(Option::as_deref)
                .or(slot0)
                .unwrap_or_else(|| default_slot0(platform, chassis).expect("validated platform"))
                .to_owned(),
            image,
            ram_mb,
            nvram,
            idle_pc,
            startup_config: startup_config.to_owned(),
            log_file: working_dir.join("dynamips_stdout.log"),
            working_dir,
            hypervisor_port: available_tcp_port().await?,
            console_port,
            aux_port,
            options: options.clone(),
            ghost_file,
        };
        self.write_runtime_manifest(&vm, &project_id, &node_id)?;
        if let Err(error) = self.initialize_router(&vm).await {
            let _ = fs::remove_file(vm.working_dir.join(RUNTIME_MANIFEST_FILE));
            return Err(error);
        }
        Ok(vm)
    }

    pub fn vm_from_node(&self, node: &Value) -> Result<DynamipsVm, RuntimeError> {
        let project_id = parse_id(node["project_id"].as_str().unwrap_or_default())?;
        let node_id = parse_id(node["node_id"].as_str().unwrap_or_default())?;
        let working_dir = self
            .state_dir
            .join(project_id.to_string())
            .join(node_id.to_string());
        let supplied_working_dir = PathBuf::from(required_string(node, "runtime_working_dir")?);
        if supplied_working_dir != working_dir {
            return Err(RuntimeError::Failed(
                "Dynamips runtime_working_dir must stay inside the controller runtime directory"
                    .to_owned(),
            ));
        }
        let expected_log_file = working_dir.join("dynamips_stdout.log");
        let supplied_log_file = PathBuf::from(required_string(node, "runtime_log_file")?);
        if supplied_log_file != expected_log_file {
            return Err(RuntimeError::Failed(
                "Dynamips runtime_log_file must stay inside the controller runtime directory"
                    .to_owned(),
            ));
        }
        let platform = required_string(node, "platform")?.to_owned();
        let chassis = node["chassis"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| (platform == "c3600").then(|| "3620".to_owned()));
        let slot0 = node["slot0"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| {
                default_slot0(&platform, chassis.as_deref())
                    .unwrap_or("C7200-IO-FE")
                    .to_owned()
            });
        validate_platform_settings(&platform, chassis.as_deref(), Some(&slot0))?;
        let name = required_string(node, "name")?;
        validate_router_name(name)?;
        let mut options: DynamipsOptions =
            serde_json::from_value(node.clone()).map_err(|error| {
                RuntimeError::Failed(format!("Dynamips node settings are invalid: {error}"))
            })?;
        options.slots = (0..7)
            .map(|index| {
                node.get(format!("slot{index}"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect();
        options.wics = (0..3)
            .map(|index| {
                node.get(format!("wic{index}"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect();
        validate_dynamips_options(&platform, chassis.as_deref(), Some(&slot0), &options)?;
        let image = self.resolve_image(required_string(node, "image")?)?;
        let idle_pc = node["idlepc"]
            .as_str()
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
        let hypervisor_port = node["runtime_hypervisor_port"]
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .filter(|port| *port != 0)
            .ok_or_else(|| {
                RuntimeError::Failed("Dynamips node has no hypervisor port".to_owned())
            })?;
        let ram_mb = node["ram"].as_u64().unwrap_or(256);
        if !(128..=2048).contains(&ram_mb) {
            return Err(RuntimeError::Failed(
                "Dynamips router RAM must be between 128 and 2048 MB".to_owned(),
            ));
        }
        let nvram = node["nvram"].as_u64().unwrap_or(128);
        if !(64..=2048).contains(&nvram) {
            return Err(RuntimeError::Failed(
                "Dynamips router NVRAM must be between 64 and 2048 KB".to_owned(),
            ));
        }
        let console_port = node["console"]
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .unwrap_or(0);
        let aux_port = node["aux"]
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .filter(|port| *port != 0);
        self.validate_runtime_manifest(
            &working_dir,
            &project_id,
            &node_id,
            hypervisor_port,
            console_port,
            aux_port,
        )?;
        let ghost_file = if options.ghost_ios && options.mmap {
            let file_name = image
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("ios");
            let expected = self
                .state_dir
                .join("ghosts")
                .join(format!("{file_name}-{ram_mb}.ghost"));
            if let Some(supplied) = node["runtime_ghost_file"].as_str()
                && Path::new(supplied) != expected
            {
                return Err(RuntimeError::Failed(
                    "Dynamips runtime_ghost_file must stay inside the controller runtime directory"
                        .to_owned(),
                ));
            }
            Some(expected)
        } else {
            if node
                .get("runtime_ghost_file")
                .is_some_and(|value| !value.is_null())
            {
                return Err(RuntimeError::Failed(
                    "Dynamips runtime_ghost_file is not valid when Ghost IOS is disabled"
                        .to_owned(),
                ));
            }
            None
        };
        Ok(DynamipsVm {
            node_id: node_id.to_string(),
            name: name.to_owned(),
            platform,
            chassis,
            slot0,
            image,
            ram_mb,
            nvram,
            idle_pc,
            startup_config: node["startup_config"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            working_dir,
            hypervisor_port,
            console_port,
            aux_port,
            log_file: expected_log_file,
            options,
            ghost_file,
        })
    }

    fn write_runtime_manifest(
        &self,
        vm: &DynamipsVm,
        project_id: &Uuid,
        node_id: &Uuid,
    ) -> Result<(), RuntimeError> {
        let contents = runtime_manifest_contents(
            project_id,
            node_id,
            vm.hypervisor_port,
            vm.console_port,
            vm.aux_port,
        );
        fs::write(vm.working_dir.join(RUNTIME_MANIFEST_FILE), contents).map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not record Dynamips runtime ownership: {error}"
            ))
        })
    }

    fn validate_runtime_manifest(
        &self,
        working_dir: &Path,
        project_id: &Uuid,
        node_id: &Uuid,
        hypervisor_port: u16,
        console_port: u16,
        aux_port: Option<u16>,
    ) -> Result<(), RuntimeError> {
        let path = working_dir.join(RUNTIME_MANIFEST_FILE);
        let metadata = fs::symlink_metadata(&path).map_err(|_| {
            RuntimeError::Failed(
                "Dynamips runtime state is not owned by this controller".to_owned(),
            )
        })?;
        if !metadata.file_type().is_file() {
            return Err(RuntimeError::Failed(
                "Dynamips runtime ownership marker is invalid".to_owned(),
            ));
        }
        let stored = fs::read_to_string(path).map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not read Dynamips runtime ownership: {error}"
            ))
        })?;
        let expected =
            runtime_manifest_contents(project_id, node_id, hypervisor_port, console_port, aux_port);
        if stored != expected {
            return Err(RuntimeError::Failed(
                "Dynamips runtime fields do not match controller-owned runtime state".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn runtime_fields(&self, vm: &DynamipsVm) -> Value {
        json!({
            "platform": vm.platform,
            "chassis": vm.chassis,
            "slot0": vm.slot0,
            "image": vm.image,
            "ram": vm.ram_mb,
            "nvram": vm.nvram,
            "idlepc": vm.idle_pc,
            "console": vm.console_port,
            "runtime_hypervisor_port": vm.hypervisor_port,
            "runtime_working_dir": vm.working_dir,
            "runtime_log_file": vm.log_file,
            "adapter_types": [vm.slot0],
            "dynamips_id": vm.options.dynamips_id,
            "clock_divisor": vm.options.clock_divisor,
            "mmap": vm.options.mmap,
            "sparsemem": vm.options.sparsemem,
            "idlemax": vm.options.idlemax,
            "idlesleep": vm.options.idlesleep,
            "exec_area": vm.options.exec_area,
            "disk0": vm.options.disk0,
            "disk1": vm.options.disk1,
            "auto_delete_disks": vm.options.auto_delete_disks,
            "private_config": vm.options.private_config,
            "mac_addr": vm.options.mac_addr,
            "system_id": vm.options.system_id,
            "slot0": vm.options.slots.first().cloned().flatten().unwrap_or_else(|| vm.slot0.clone()),
            "slot1": vm.options.slots.get(1).cloned().flatten(),
            "slot2": vm.options.slots.get(2).cloned().flatten(),
            "slot3": vm.options.slots.get(3).cloned().flatten(),
            "slot4": vm.options.slots.get(4).cloned().flatten(),
            "slot5": vm.options.slots.get(5).cloned().flatten(),
            "slot6": vm.options.slots.get(6).cloned().flatten(),
            "wic0": vm.options.wics.first().cloned().flatten(),
            "wic1": vm.options.wics.get(1).cloned().flatten(),
            "wic2": vm.options.wics.get(2).cloned().flatten(),
            "npe": vm.options.npe,
            "midplane": vm.options.midplane,
            "iomem": vm.options.iomem,
            "ghost_ios": vm.options.ghost_ios,
            "console_type": vm.options.console_type,
            "aux_type": vm.options.aux_type,
            "aux": vm.aux_port,
            "runtime_ghost_file": vm.ghost_file
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

    pub async fn idlepc_proposals(&self, vm: &DynamipsVm) -> Result<Vec<String>, RuntimeError> {
        let _guard = self.operation_lock.lock().await;
        let was_auto_started =
            self.hypervisor_status(vm.hypervisor_port, &vm.name).await? != "started";
        let probe = async {
            if was_auto_started {
                self.start_locked(vm).await?;
                sleep(Duration::from_secs(20)).await;
            }
            self.send(
                vm.hypervisor_port,
                &format!("vm get_idle_pc_prop {} 0", quote(&vm.name)),
            )
            .await
        }
        .await;
        let stopped = if was_auto_started {
            self.stop_locked(vm).await.map(|_| ())
        } else {
            Ok(())
        };
        match (probe, stopped) {
            (Ok(proposals), Ok(())) => Ok(proposals),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    pub async fn auto_idlepc(&self, vm: &DynamipsVm) -> Result<String, RuntimeError> {
        let _guard = self.operation_lock.lock().await;
        let was_auto_started =
            self.hypervisor_status(vm.hypervisor_port, &vm.name).await? != "started";
        let probe = async {
            if was_auto_started {
                self.start_locked(vm).await?;
                sleep(Duration::from_secs(20)).await;
            }
            self.send(
                vm.hypervisor_port,
                &format!("vm set_idle_pc_online {} 0 0x0", quote(&vm.name)),
            )
            .await?;
            let proposals = self
                .send(
                    vm.hypervisor_port,
                    &format!("vm get_idle_pc_prop {} 0", quote(&vm.name)),
                )
                .await?;
            for proposal in proposals {
                let Some(candidate) = idlepc_candidate(&proposal) else {
                    continue;
                };
                self.send(
                    vm.hypervisor_port,
                    &format!("vm set_idle_pc_online {} 0 {candidate}", quote(&vm.name)),
                )
                .await?;
                let before = self.cpu_usage(vm.hypervisor_port, &vm.name).await?;
                let started = tokio::time::Instant::now();
                sleep(Duration::from_secs(3)).await;
                let after = self.cpu_usage(vm.hypervisor_port, &vm.name).await?;
                if idlepc_cpu_rate(before, after, started.elapsed()) < 70.0 {
                    return Ok(candidate.to_owned());
                }
            }
            Err(RuntimeError::Failed(
                "Sorry, no idle-pc value was suitable".to_owned(),
            ))
        }
        .await;
        let stopped = if was_auto_started {
            self.stop_locked(vm).await.map(|_| ())
        } else {
            Ok(())
        };
        match (probe, stopped) {
            (Ok(idlepc), Ok(())) => Ok(idlepc),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    pub async fn console(&self, vm: &DynamipsVm) -> Result<TcpStream, RuntimeError> {
        if vm.console_port == 0
            || vm.options.console_type == "none"
            || self.status(vm).await? != "started"
        {
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
        if vm.options.auto_delete_disks {
            remove_auto_delete_disk_files(&vm.working_dir, &vm.platform, vm.options.dynamips_id)?;
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
        // TODO(RUNTIME-NET Dynamips adapter hook): bind all configured router
        // module ports through NET-managed NIOs; this runtime API currently
        // exposes one adapter/port and must not implement attachment itself.
        let _guard = self.operation_lock.lock().await;
        if adapter_number != 0 || port_number != 0 {
            return Err(RuntimeError::Failed(
                "This Dynamips template exposes only adapter 0, port 0".to_owned(),
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

    pub async fn attach_nio(
        &self,
        vm: &DynamipsVm,
        adapter_number: u32,
        port_number: u32,
        local_port: u16,
        remote_port: u16,
    ) -> Result<(), RuntimeError> {
        let _guard = self.operation_lock.lock().await;
        if adapter_number != 0 || port_number != 0 {
            return Err(RuntimeError::Failed(
                "This Dynamips template exposes only adapter 0, port 0".to_owned(),
            ));
        }
        self.ensure_hypervisor(vm).await?;
        if let Some(existing) = self.bindings.lock().await.get(&vm.node_id).copied() {
            if existing == local_port {
                return Ok(());
            }
            return Err(RuntimeError::Failed(
                "Dynamips adapter already has a different NIO binding".to_owned(),
            ));
        }
        let nio_name = format!("udp-{}", vm.node_id.replace('-', ""));
        self.send(
            vm.hypervisor_port,
            &format!("nio create_udp {nio_name} {local_port} 127.0.0.1 {remote_port}"),
        )
        .await?;
        if let Err(error) = self
            .send(
                vm.hypervisor_port,
                &format!(
                    "vm slot_add_nio_binding {} {adapter_number} {port_number} {nio_name}",
                    quote(&vm.name)
                ),
            )
            .await
        {
            let _ = self
                .send(vm.hypervisor_port, &format!("nio delete {nio_name}"))
                .await;
            return Err(error);
        }
        if self.hypervisor_status(vm.hypervisor_port, &vm.name).await? == "started" {
            self.send(
                vm.hypervisor_port,
                &format!(
                    "vm slot_enable_nio {} {adapter_number} {port_number}",
                    quote(&vm.name)
                ),
            )
            .await?;
        }
        self.bindings
            .lock()
            .await
            .insert(vm.node_id.clone(), local_port);
        Ok(())
    }

    pub async fn detach_nio(
        &self,
        vm: &DynamipsVm,
        adapter_number: u32,
        port_number: u32,
    ) -> Result<(), RuntimeError> {
        let _guard = self.operation_lock.lock().await;
        if adapter_number != 0 || port_number != 0 {
            return Ok(());
        }
        let Some(_binding) = self.bindings.lock().await.remove(&vm.node_id) else {
            return Ok(());
        };
        if self.router_process_alive(&vm.node_id).await {
            let nio_name = format!("udp-{}", vm.node_id.replace('-', ""));
            let _ = self
                .send(
                    vm.hypervisor_port,
                    &format!(
                        "vm slot_disable_nio {} {adapter_number} {port_number}",
                        quote(&vm.name)
                    ),
                )
                .await;
            let _ = self
                .send(
                    vm.hypervisor_port,
                    &format!(
                        "vm slot_remove_nio_binding {} {adapter_number} {port_number}",
                        quote(&vm.name)
                    ),
                )
                .await;
            let _ = self
                .send(vm.hypervisor_port, &format!("nio delete {nio_name}"))
                .await;
        }
        Ok(())
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
            for command in router_initialization_commands(vm) {
                self.send(port, &command).await?;
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
        self.ensure_ghost_ios(vm).await;
        fs::write(
            vm.working_dir.join(format!(
                "configs/i{}_startup-config.cfg",
                vm.options.dynamips_id
            )),
            &vm.startup_config,
        )
        .map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not write Dynamips startup configuration: {error}"
            ))
        })?;
        let private_config_path = vm.working_dir.join(format!(
            "configs/i{}_private-config.cfg",
            vm.options.dynamips_id
        ));
        if !vm.options.private_config.is_empty() {
            fs::write(&private_config_path, &vm.options.private_config).map_err(|error| {
                RuntimeError::Failed(format!(
                    "Could not write Dynamips private configuration: {error}"
                ))
            })?;
        }
        self.send(
            vm.hypervisor_port,
            &format!(
                "vm set_config {} {} {}",
                quote(&vm.name),
                quote(&format!(
                    "configs/i{}_startup-config.cfg",
                    vm.options.dynamips_id
                )),
                if vm.options.private_config.is_empty() {
                    "\"\"".to_owned()
                } else {
                    quote(&format!(
                        "configs/i{}_private-config.cfg",
                        vm.options.dynamips_id
                    ))
                }
            ),
        )
        .await?;
        self.send(vm.hypervisor_port, &format!("vm start {}", quote(&vm.name)))
            .await?;
        for command in router_start_settings_commands(vm) {
            self.send(vm.hypervisor_port, &command).await?;
        }
        self.hypervisor_status(vm.hypervisor_port, &vm.name).await
    }

    async fn ensure_ghost_ios(&self, vm: &DynamipsVm) {
        let (Some(ghost_file), true) = (
            vm.ghost_file.as_ref(),
            vm.options.ghost_ios && vm.options.mmap,
        ) else {
            return;
        };
        if vm.platform == "c7200" && vm.options.npe.as_deref() == Some("npe-g2") {
            tracing::warn!(node_id = %vm.node_id, "Dynamips ghost IOS is unsupported for c7200 with NPE-G2");
            return;
        }
        if let Some(parent) = ghost_file.parent()
            && let Err(error) = fs::create_dir_all(parent)
        {
            tracing::warn!(error = %error, "Could not create Dynamips ghost IOS cache directory");
            return;
        }
        if !ghost_file.is_file() {
            let name = "Ghost";
            let create = async {
                for command in ghost_router_initialization_commands(vm, ghost_file) {
                    self.send(vm.hypervisor_port, &command).await?;
                }
                self.send(vm.hypervisor_port, &format!("vm start {}", quote(name)))
                    .await?;
                sleep(Duration::from_secs(20)).await;
                self.send(vm.hypervisor_port, &format!("vm stop {}", quote(name)))
                    .await?;
                Ok::<(), RuntimeError>(())
            }
            .await;
            let cleanup = self
                .send(
                    vm.hypervisor_port,
                    &format!("vm clean_delete {}", quote(name)),
                )
                .await;
            if let Err(error) = create {
                tracing::warn!(error = %error.message(), node_id = %vm.node_id, "Could not generate Dynamips ghost IOS cache");
                let _ = cleanup;
                return;
            }
            if let Err(error) = cleanup {
                tracing::warn!(error = %error.message(), "Could not clean up temporary Dynamips ghost router");
            }
        }
        if ghost_file.is_file() {
            for command in [
                format!("vm set_ghost_status {} 2", quote(&vm.name)),
                format!(
                    "vm set_ghost_file {} {}",
                    quote(&vm.name),
                    quote(&ghost_file.display().to_string())
                ),
            ] {
                if let Err(error) = self.send(vm.hypervisor_port, &command).await {
                    tracing::warn!(error = %error.message(), node_id = %vm.node_id, "Could not attach Dynamips ghost IOS cache");
                    break;
                }
            }
        }
    }

    async fn cpu_usage(&self, port: u16, name: &str) -> Result<i64, RuntimeError> {
        self.send(port, &format!("vm cpu_usage {} 0", quote(name)))
            .await?
            .first()
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| RuntimeError::Failed("Dynamips returned invalid CPU usage".to_owned()))
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
        if command.chars().any(char::is_control) {
            return Err(RuntimeError::Failed(
                "Dynamips protocol commands cannot contain control characters".to_owned(),
            ));
        }
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

fn idlepc_candidate(proposal: &str) -> Option<&str> {
    let candidate = proposal.split_whitespace().next()?;
    (candidate.len() == 10
        && candidate.starts_with("0x")
        && candidate[2..].bytes().all(|byte| byte.is_ascii_hexdigit()))
    .then_some(candidate)
}

fn idlepc_cpu_rate(before: i64, after: i64, elapsed: Duration) -> f64 {
    let elapsed_seconds = elapsed.as_secs_f64().max(0.001);
    (after.saturating_sub(before).unsigned_abs() as f64 * 100.0 / elapsed_seconds).min(100.0)
}

fn default_slot0(platform: &str, chassis: Option<&str>) -> Option<&'static str> {
    match platform {
        "c7200" => Some("C7200-IO-FE"),
        "c3600" if chassis == Some("3660") => Some("Leopard-2FE"),
        "c3600" => Some("NM-1FE-TX"),
        "c2691" | "c3725" | "c3745" => Some("GT96100-FE"),
        "c2600" => Some(match chassis {
            Some("2611") => "C2600-MB-2E",
            Some("2620" | "2610XM" | "2620XM" | "2650XM") => "C2600-MB-1FE",
            Some("2621" | "2611XM" | "2621XM" | "2651XM") => "C2600-MB-2FE",
            _ => "C2600-MB-1E",
        }),
        "c1700" => Some("C1700-MB-1FE"),
        _ => None,
    }
}

fn validate_platform_settings(
    platform: &str,
    chassis: Option<&str>,
    slot0: Option<&str>,
) -> Result<(), RuntimeError> {
    let slot0 = slot0.or_else(|| default_slot0(platform, chassis));
    const PLATFORMS: &[&str] = &[
        "c1700", "c2600", "c2691", "c3600", "c3725", "c3745", "c7200",
    ];
    let chassis_valid = match platform {
        "c1700" => matches!(
            chassis,
            None | Some("1720" | "1721" | "1750" | "1751" | "1760")
        ),
        "c2600" => matches!(
            chassis,
            None | Some(
                "2610"
                    | "2611"
                    | "2620"
                    | "2610XM"
                    | "2620XM"
                    | "2650XM"
                    | "2621"
                    | "2611XM"
                    | "2621XM"
                    | "2651XM"
            )
        ),
        "c3600" => matches!(chassis, None | Some("3620" | "3640" | "3660")),
        _ => chassis.is_none(),
    };
    let supported = PLATFORMS.contains(&platform)
        && chassis_valid
        && slot0.is_some_and(|module| module_supported_on_slot(platform, chassis, 0, module));
    if !supported {
        return Err(RuntimeError::Failed(format!(
            "Dynamips runtime does not support this configuration (platform '{platform}', chassis '{}', slot 0 '{}')",
            chassis.unwrap_or("default"),
            slot0.unwrap_or("empty")
        )));
    }
    Ok(())
}

fn validate_dynamips_options(
    platform: &str,
    chassis: Option<&str>,
    slot0: Option<&str>,
    options: &DynamipsOptions,
) -> Result<(), RuntimeError> {
    validate_platform_settings(platform, chassis, slot0)?;
    if options.ghost_ios && !options.mmap {
        return Err(RuntimeError::Failed(
            "Dynamips mmap support is required to enable ghost IOS".to_owned(),
        ));
    }
    if !(1..=4096).contains(&options.dynamips_id) {
        return Err(RuntimeError::Failed(
            "Dynamips internal ID must be from 1 to 4096".to_owned(),
        ));
    }
    if options.clock_divisor == 0 || options.clock_divisor > 4096 {
        return Err(RuntimeError::Failed(
            "Dynamips clock_divisor must be from 1 to 4096".to_owned(),
        ));
    }
    if options.exec_area > 4096 || options.disk0 > 16_384 || options.disk1 > 16_384 {
        return Err(RuntimeError::Failed(
            "Dynamips exec area and disk sizes are outside the supported range".to_owned(),
        ));
    }
    if options.iomem.is_some_and(|value| value > 100) {
        return Err(RuntimeError::Failed(
            "Dynamips iomem must be from 0 to 100".to_owned(),
        ));
    }
    if options.slots.len() > 7 || options.wics.len() > 3 {
        return Err(RuntimeError::Failed(
            "Dynamips supports slots 0–6 and WIC slots 0–2".to_owned(),
        ));
    }
    let slot_capacity = match platform {
        "c1700" if matches!(chassis, Some("1751" | "1760")) => 2,
        "c1700" => 1,
        "c2600" | "c2691" => 2,
        "c3600" => match chassis.unwrap_or("3640") {
            "3620" => 2,
            "3660" => 7,
            _ => 4,
        },
        "c3725" => 3,
        "c3745" => 5,
        _ => 7,
    };
    if options.slots.iter().enumerate().any(|(slot, module)| {
        slot >= slot_capacity && module.as_deref().is_some_and(|module| !module.is_empty())
    }) {
        return Err(RuntimeError::Failed(format!(
            "Dynamips platform {platform} with chassis '{}' supports slots 0 through {}",
            chassis.unwrap_or("default"),
            slot_capacity - 1
        )));
    }
    if options.slots.iter().enumerate().any(|(slot, module)| {
        module
            .as_deref()
            .is_some_and(|module| !module_supported_on_slot(platform, chassis, slot, module))
    }) {
        return Err(RuntimeError::Failed(
            "Dynamips slot contains a network module that is incompatible with this platform"
                .to_owned(),
        ));
    }
    if options
        .wics
        .iter()
        .flatten()
        .any(|wic| !matches!(wic.as_str(), "" | "WIC-1ENET" | "WIC-1T" | "WIC-2T"))
    {
        return Err(RuntimeError::Failed(
            "Dynamips WIC must be WIC-1ENET, WIC-1T, or WIC-2T".to_owned(),
        ));
    }
    if options.npe.as_deref().is_some_and(|npe| {
        !matches!(
            npe,
            "npe-100"
                | "npe-150"
                | "npe-175"
                | "npe-200"
                | "npe-225"
                | "npe-300"
                | "npe-400"
                | "npe-g2"
        )
    }) || options
        .midplane
        .as_deref()
        .is_some_and(|midplane| !matches!(midplane, "std" | "vxr"))
    {
        return Err(RuntimeError::Failed(
            "Dynamips NPE or midplane model is unsupported".to_owned(),
        ));
    }
    if options.mac_addr.as_deref().is_some_and(|address| {
        !address.is_empty()
            && !(address.len() == 14
                && address.split('.').count() == 3
                && address.split('.').all(|part| {
                    part.len() == 4 && part.bytes().all(|byte| byte.is_ascii_hexdigit())
                }))
    }) {
        return Err(RuntimeError::Failed(
            "Dynamips MAC address must use xxxx.xxxx.xxxx format".to_owned(),
        ));
    }
    if options.system_id.as_deref().is_some_and(|id| {
        !id.is_empty() && (id.len() > 32 || !id.bytes().all(|byte| byte.is_ascii_alphanumeric()))
    }) {
        return Err(RuntimeError::Failed(
            "Dynamips system_id must contain up to 32 letters or numbers".to_owned(),
        ));
    }
    if options.console_type != "telnet"
        && options.console_type != "ssh"
        && options.console_type != "none"
        || options.aux_type != "telnet" && options.aux_type != "ssh" && options.aux_type != "none"
    {
        return Err(RuntimeError::Failed(
            "Dynamips console types must be telnet, ssh, or none".to_owned(),
        ));
    }
    Ok(())
}

fn module_supported_on_slot(
    platform: &str,
    chassis: Option<&str>,
    slot: usize,
    module: &str,
) -> bool {
    if module.is_empty() {
        return true;
    }
    match module {
        "C7200-IO-2FE" | "C7200-IO-FE" | "C7200-IO-GE-E" => platform == "c7200" && slot == 0,
        "PA-2FE-TX" | "PA-4E" | "PA-4T+" | "PA-8E" | "PA-8T" | "PA-A1" | "PA-FE-TX" | "PA-GE"
        | "PA-POS-OC3" => platform == "c7200" && (1..=6).contains(&slot),
        "C2600-MB-2FE" | "C2600-MB-1E" | "C2600-MB-2E" | "C2600-MB-1FE" => {
            platform == "c2600" && slot == 0
        }
        "C1700-MB-1FE" => platform == "c1700" && slot == 0,
        "C1700-MB-WIC1" => {
            platform == "c1700" && slot == 1 && matches!(chassis, Some("1751" | "1760"))
        }
        "GT96100-FE" => matches!(platform, "c2691" | "c3725" | "c3745") && slot == 0,
        "Leopard-2FE" => platform == "c3600" && chassis == Some("3660") && slot == 0,
        "NM-16ESW" | "NM-1E" | "NM-1FE-TX" | "NM-4E" | "NM-4T" => {
            matches!(platform, "c2600" | "c2691" | "c3600" | "c3725" | "c3745")
                && (slot > 0 || (platform == "c3600" && chassis != Some("3660")))
        }
        _ => false,
    }
}

fn router_initialization_commands(vm: &DynamipsVm) -> Vec<String> {
    let mut commands = vec![
        format!(
            "vm create {} {} {}",
            quote(&vm.name),
            vm.options.dynamips_id,
            vm.platform
        ),
        format!(
            "vm set_ios {} {}",
            quote(&vm.name),
            quote(&vm.image.display().to_string())
        ),
        format!("vm set_ram {} {}", quote(&vm.name), vm.ram_mb),
        format!("vm set_nvram {} {}", quote(&vm.name), vm.nvram),
        format!(
            "vm set_clock_divisor {} {}",
            quote(&vm.name),
            vm.options.clock_divisor
        ),
        format!(
            "vm set_ram_mmap {} {}",
            quote(&vm.name),
            u8::from(vm.options.mmap)
        ),
        format!(
            "vm set_sparse_mem {} {}",
            quote(&vm.name),
            u8::from(vm.options.sparsemem)
        ),
        format!(
            "vm set_exec_area {} {}",
            quote(&vm.name),
            vm.options.exec_area
        ),
        format!("vm set_disk0 {} {}", quote(&vm.name), vm.options.disk0),
        format!("vm set_disk1 {} {}", quote(&vm.name), vm.options.disk1),
    ];
    if vm.options.console_type != "none" {
        commands.push(format!(
            "vm set_con_tcp_port {} {}",
            quote(&vm.name),
            vm.console_port
        ));
    }
    if vm.options.aux_type != "none" {
        if let Some(aux_port) = vm.aux_port {
            commands.push(format!(
                "vm set_aux_tcp_port {} {aux_port}",
                quote(&vm.name)
            ));
        }
    }
    if matches!(vm.platform.as_str(), "c1700" | "c2600" | "c3600")
        && let Some(chassis) = &vm.chassis
    {
        commands.push(format!(
            "{} set_chassis {} {chassis}",
            vm.platform,
            quote(&vm.name)
        ));
    }
    if vm.platform == "c7200" {
        if let Some(npe) = &vm.options.npe {
            commands.push(format!("c7200 set_npe {} {npe}", quote(&vm.name)));
        }
        if let Some(midplane) = &vm.options.midplane {
            commands.push(format!("c7200 set_midplane {} {midplane}", quote(&vm.name)));
        }
    } else if let Some(iomem) = vm.options.iomem {
        commands.push(format!(
            "{} set_iomem {} {iomem}",
            vm.platform,
            quote(&vm.name)
        ));
    }
    if let Some(mac_addr) = &vm.options.mac_addr {
        commands.push(format!(
            "{} set_mac_addr {} {mac_addr}",
            vm.platform,
            quote(&vm.name)
        ));
    }
    if let Some(system_id) = &vm.options.system_id {
        commands.push(format!(
            "{} set_system_id {} {system_id}",
            vm.platform,
            quote(&vm.name)
        ));
    }
    let max_slots = match vm.platform.as_str() {
        "c1700" => 2,
        "c2600" | "c2691" => 2,
        "c3600" => match vm.chassis.as_deref().unwrap_or("3640") {
            "3620" => 2,
            "3660" => 7,
            _ => 4,
        },
        "c3725" => 3,
        "c3745" => 5,
        _ => 7,
    };
    for slot in 0..max_slots {
        let module = vm
            .options
            .slots
            .get(slot)
            .and_then(Option::as_deref)
            .filter(|module| !module.is_empty())
            .or_else(|| (slot == 0).then_some(vm.slot0.as_str()));
        if let Some(module) = module {
            commands.push(format!(
                "vm slot_add_binding {} {slot} 0 {module}",
                quote(&vm.name)
            ));
        }
    }
    for (wic_slot, wic) in vm.options.wics.iter().enumerate() {
        if let Some(wic) = wic.as_deref().filter(|wic| !wic.is_empty()) {
            commands.push(format!(
                "vm slot_add_binding {} 0 {} {wic}",
                quote(&vm.name),
                16 * (wic_slot + 1)
            ));
        }
    }
    if vm.options.ghost_ios
        && vm.options.mmap
        && vm.ghost_file.as_ref().is_some_and(|path| path.is_file())
    {
        let ghost_file = vm.ghost_file.as_ref().expect("checked ghost file");
        commands.push(format!("vm set_ghost_status {} 2", quote(&vm.name)));
        commands.push(format!(
            "vm set_ghost_file {} {}",
            quote(&vm.name),
            quote(&ghost_file.display().to_string())
        ));
    }
    if let Some(idle_pc) = &vm.idle_pc {
        commands.push(format!("vm set_idle_pc {} {idle_pc}", quote(&vm.name)));
    }
    commands
}

fn ghost_router_initialization_commands(vm: &DynamipsVm, ghost_file: &Path) -> Vec<String> {
    let name = "Ghost";
    vec![
        format!("vm create {} 0 {}", quote(name), vm.platform),
        format!(
            "vm set_ios {} {}",
            quote(name),
            quote(&vm.image.display().to_string())
        ),
        format!("vm set_ghost_status {} 1", quote(name)),
        format!(
            "vm set_ghost_file {} {}",
            quote(name),
            quote(&ghost_file.display().to_string())
        ),
        format!("vm set_ram {} {}", quote(name), vm.ram_mb),
    ]
}

fn router_start_settings_commands(vm: &DynamipsVm) -> Vec<String> {
    vec![
        format!(
            "vm set_idle_max {} 0 {}",
            quote(&vm.name),
            vm.options.idlemax
        ),
        format!(
            "vm set_idle_sleep_time {} 0 {}",
            quote(&vm.name),
            vm.options.idlesleep
        ),
    ]
}

fn remove_auto_delete_disk_files(
    working_dir: &Path,
    platform: &str,
    dynamips_id: u32,
) -> Result<(), RuntimeError> {
    if !working_dir.exists() {
        return Ok(());
    }
    let entries = fs::read_dir(working_dir).map_err(|error| {
        RuntimeError::Failed(format!("Could not inspect Dynamips disk files: {error}"))
    })?;
    let removable_names = [
        format!("{platform}_i{dynamips_id}_disk0"),
        format!("{platform}_i{dynamips_id}_disk1"),
        format!("{platform}_i{dynamips_id}_slot0"),
        format!("{platform}_i{dynamips_id}_slot1"),
        format!("{platform}_i{dynamips_id}_nvram"),
        format!("{platform}_i{dynamips_id}_flash0"),
        format!("{platform}_i{dynamips_id}_flash1"),
        format!("{platform}_i{dynamips_id}_rom"),
        format!("{platform}_i{dynamips_id}_bootflash"),
        format!("{platform}_i{dynamips_id}_ssa"),
    ];
    for entry in entries {
        let entry = entry.map_err(|error| {
            RuntimeError::Failed(format!("Could not inspect Dynamips disk files: {error}"))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            RuntimeError::Failed(format!("Could not inspect Dynamips disk file: {error}"))
        })?;
        if file_type.is_file()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| removable_names.iter().any(|candidate| candidate == name))
        {
            fs::remove_file(entry.path()).map_err(|error| {
                RuntimeError::Failed(format!("Could not remove Dynamips disk file: {error}"))
            })?;
        }
    }
    Ok(())
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
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_control() {
            let codepoint = character as u32;
            if codepoint <= 0xff {
                escaped.push_str(&format!("\\x{codepoint:02x}"));
            } else {
                escaped.push_str(&format!("\\u{{{codepoint:x}}}"));
            }
        } else if character == '\\' || character == '"' {
            escaped.push('\\');
            escaped.push(character);
        } else {
            escaped.push(character);
        }
    }
    format!("\"{escaped}\"")
}

fn runtime_manifest_contents(
    project_id: &Uuid,
    node_id: &Uuid,
    hypervisor_port: u16,
    console_port: u16,
    aux_port: Option<u16>,
) -> String {
    format!(
        "{project_id}\n{node_id}\n{hypervisor_port}\n{console_port}\n{}\n",
        aux_port.unwrap_or(0)
    )
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
    use super::{
        DynamipsOptions, DynamipsRuntime, DynamipsVm, RUNTIME_MANIFEST_FILE, default_slot0,
        ghost_router_initialization_commands, idlepc_candidate, idlepc_cpu_rate, quote,
        remove_auto_delete_disk_files, router_initialization_commands,
        router_start_settings_commands, runtime_manifest_contents, safe_file_name,
        supported_dynamips_version, validate_dynamips_options, validate_ios_image,
        validate_platform_settings, validate_router_name,
    };
    use serde_json::json;
    use std::{
        env, fs,
        net::{Ipv4Addr, SocketAddrV4},
        path::PathBuf,
    };
    use uuid::Uuid;

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
    fn parses_idlepc_candidates_and_scores_cpu_delta() {
        assert_eq!(idlepc_candidate("0x1234abcd 90%"), Some("0x1234abcd"));
        assert_eq!(idlepc_candidate("not-a-pointer 90%"), None);
        assert_eq!(idlepc_candidate("0x1234 90%"), None);
        assert!(idlepc_cpu_rate(100, 102, std::time::Duration::from_secs(3)) < 70.0);
        assert_eq!(
            idlepc_cpu_rate(0, 1000, std::time::Duration::from_secs(1)),
            100.0
        );
    }

    #[test]
    fn ghost_ios_requires_mmap_support() {
        let options = DynamipsOptions {
            ghost_ios: true,
            mmap: false,
            ..DynamipsOptions::default()
        };
        assert!(validate_dynamips_options("c7200", None, None, &options).is_err());
        let supported = DynamipsOptions {
            ghost_ios: true,
            mmap: true,
            ..DynamipsOptions::default()
        };
        assert!(validate_dynamips_options("c7200", None, Some("C7200-IO-FE"), &supported).is_ok());
    }

    #[test]
    fn dynamips_quoted_strings_and_names_cannot_emit_protocol_newlines() {
        let injected = "R1\nvm clean_delete \"R2\"\r";
        let quoted = quote(injected);
        assert!(!quoted.chars().any(char::is_control));
        assert!(quoted.contains("\\x0a"));
        assert!(quoted.contains("\\x0d"));
        assert!(validate_router_name(injected).is_err());
        assert!(validate_router_name("Router-1").is_ok());
    }

    #[test]
    fn imported_dynamips_runtime_fields_must_match_controller_state() {
        let directory = tempfile::tempdir().unwrap();
        let image_dir = directory.path().join("ios");
        let state_dir = directory.path().join("state");
        let project_id = Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap();
        let node_id = Uuid::parse_str("00000000-0000-4000-8000-000000000002").unwrap();
        let working_dir = state_dir
            .join(project_id.to_string())
            .join(node_id.to_string());
        fs::create_dir_all(&image_dir).unwrap();
        fs::create_dir_all(&working_dir).unwrap();
        let image = image_dir.join("router.image");
        fs::write(&image, [0x7f, b'E', b'L', b'F', 1, 2, 1, 0]).unwrap();
        let runtime = DynamipsRuntime::with_paths("dynamips", &image_dir, &state_dir);
        let manifest = runtime_manifest_contents(&project_id, &node_id, 7200, 7201, None);
        fs::write(working_dir.join(RUNTIME_MANIFEST_FILE), manifest).unwrap();
        let node = json!({
            "project_id": project_id,
            "node_id": node_id,
            "name": "R1",
            "platform": "c7200",
            "slot0": "C7200-IO-FE",
            "image": image,
            "ram": 256,
            "nvram": 128,
            "console": 7201,
            "aux": null,
            "runtime_hypervisor_port": 7200,
            "runtime_working_dir": working_dir,
            "runtime_log_file": working_dir.join("dynamips_stdout.log"),
            "runtime_ghost_file": null
        });
        assert!(runtime.vm_from_node(&node).is_ok());
        for (field, value) in [
            ("runtime_working_dir", json!("/tmp/controller-victim")),
            ("runtime_log_file", json!("/tmp/victim.log")),
            ("runtime_hypervisor_port", json!(22)),
            ("name", json!("R1\nvm clean_delete R2")),
            ("image", json!("/etc/passwd.image")),
        ] {
            let mut imported = node.clone();
            imported[field] = value;
            assert!(
                runtime.vm_from_node(&imported).is_err(),
                "imported Dynamips field {field} was accepted"
            );
        }
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

    #[test]
    fn configures_a_real_gns3_cisco_3620_platform_with_ethernet_module() {
        validate_platform_settings("c3600", Some("3620"), None).unwrap();
        let vm = DynamipsVm {
            node_id: "00000000-0000-0000-0000-00000000000a".to_owned(),
            name: "R1".to_owned(),
            platform: "c3600".to_owned(),
            chassis: Some("3620".to_owned()),
            slot0: "NM-1FE-TX".to_owned(),
            image: PathBuf::from("/images/c3620.image"),
            ram_mb: 192,
            nvram: 256,
            idle_pc: Some("0x603a8bac".to_owned()),
            startup_config: String::new(),
            working_dir: PathBuf::from("/tmp/router"),
            hypervisor_port: 7200,
            console_port: 7201,
            aux_port: None,
            log_file: PathBuf::from("/tmp/router/dynamips.log"),
            options: super::DynamipsOptions::default(),
            ghost_file: None,
        };
        let commands = router_initialization_commands(&vm);
        assert_eq!(commands[0], "vm create \"R1\" 1 c3600");
        assert!(commands.contains(&"vm set_nvram \"R1\" 256".to_owned()));
        assert!(commands.contains(&"c3600 set_chassis \"R1\" 3620".to_owned()));
        assert!(commands.contains(&"vm slot_add_binding \"R1\" 0 0 NM-1FE-TX".to_owned()));
        assert!(commands.contains(&"vm set_idle_pc \"R1\" 0x603a8bac".to_owned()));
        assert!(validate_platform_settings("c3600", Some("3600"), None).is_err());
        assert!(validate_platform_settings("c3600", Some("3620"), Some("C7200-IO-FE")).is_err());
    }

    #[test]
    fn dynamips_launch_commands_apply_platform_hardware_and_runtime_settings() {
        let directory = tempfile::tempdir().unwrap();
        let ghost = directory.path().join("router.image-512.ghost");
        fs::write(&ghost, b"ghost cache").unwrap();
        let options = DynamipsOptions {
            dynamips_id: 42,
            clock_divisor: 16,
            mmap: true,
            sparsemem: false,
            idlemax: 900,
            idlesleep: 80,
            exec_area: 128,
            disk0: 256,
            disk1: 512,
            auto_delete_disks: true,
            mac_addr: Some("aabb.ccdd.eeff".to_owned()),
            system_id: Some("FTX0945W0MY".to_owned()),
            slots: vec![
                Some("C7200-IO-FE".to_owned()),
                Some("PA-4E".to_owned()),
                None,
                None,
                None,
                None,
                None,
            ],
            wics: vec![Some("WIC-1T".to_owned()), None, None],
            npe: Some("npe-200".to_owned()),
            midplane: Some("std".to_owned()),
            iomem: None,
            ghost_ios: true,
            ..DynamipsOptions::default()
        };
        validate_dynamips_options("c7200", None, Some("C7200-IO-FE"), &options).unwrap();
        let vm = DynamipsVm {
            node_id: "00000000-0000-0000-0000-00000000000a".to_owned(),
            name: "R42".to_owned(),
            platform: "c7200".to_owned(),
            chassis: None,
            slot0: "C7200-IO-FE".to_owned(),
            image: PathBuf::from("/images/router.image"),
            ram_mb: 512,
            nvram: 512,
            idle_pc: Some("0x603a8bac".to_owned()),
            startup_config: String::new(),
            working_dir: directory.path().join("router"),
            hypervisor_port: 7200,
            console_port: 7201,
            aux_port: None,
            log_file: directory.path().join("router/dynamips.log"),
            options,
            ghost_file: Some(ghost.clone()),
        };
        let commands = router_initialization_commands(&vm);
        assert_eq!(
            ghost_router_initialization_commands(&vm, &ghost),
            vec![
                "vm create \"Ghost\" 0 c7200".to_owned(),
                "vm set_ios \"Ghost\" \"/images/router.image\"".to_owned(),
                "vm set_ghost_status \"Ghost\" 1".to_owned(),
                format!("vm set_ghost_file \"Ghost\" \"{}\"", ghost.display()),
                "vm set_ram \"Ghost\" 512".to_owned(),
            ]
        );
        for expected in [
            "vm create \"R42\" 42 c7200",
            "vm set_clock_divisor \"R42\" 16",
            "vm set_ram_mmap \"R42\" 1",
            "vm set_sparse_mem \"R42\" 0",
            "vm set_exec_area \"R42\" 128",
            "vm set_disk0 \"R42\" 256",
            "vm set_disk1 \"R42\" 512",
            "c7200 set_npe \"R42\" npe-200",
            "c7200 set_midplane \"R42\" std",
            "c7200 set_mac_addr \"R42\" aabb.ccdd.eeff",
            "c7200 set_system_id \"R42\" FTX0945W0MY",
            "vm slot_add_binding \"R42\" 1 0 PA-4E",
            "vm slot_add_binding \"R42\" 0 16 WIC-1T",
            "vm set_ghost_status \"R42\" 2",
        ] {
            assert!(
                commands.iter().any(|command| command == expected),
                "missing {expected}"
            );
        }
        assert!(
            commands
                .iter()
                .any(|command| command.contains(&ghost.display().to_string()))
        );
        assert_eq!(
            router_start_settings_commands(&vm)
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [
                "vm set_idle_max \"R42\" 0 900",
                "vm set_idle_sleep_time \"R42\" 0 80"
            ]
        );
    }

    #[test]
    fn accepts_every_upstream_dynamips_platform_and_chassis_family() {
        for platform in [
            "c1700", "c2600", "c2691", "c3600", "c3725", "c3745", "c7200",
        ] {
            let chassis = match platform {
                "c1700" => Some("1760"),
                "c2600" => Some("2651XM"),
                "c3600" => Some("3660"),
                _ => None,
            };
            validate_platform_settings(platform, chassis, None).unwrap();
        }
        assert_eq!(default_slot0("c3600", Some("3660")), Some("Leopard-2FE"));
        assert_eq!(default_slot0("c2600", Some("2651XM")), Some("C2600-MB-2FE"));
        validate_platform_settings("c2600", Some("2611"), None).unwrap();
    }

    #[test]
    fn auto_delete_disks_removes_only_hypervisor_artifacts() {
        let directory = tempfile::tempdir().unwrap();
        let disk = directory.path().join("c7200_i42_disk0");
        let nvram = directory.path().join("c7200_i42_nvram");
        let startup = directory.path().join("configs/i42_startup-config.cfg");
        fs::create_dir_all(startup.parent().unwrap()).unwrap();
        fs::write(&disk, b"disk").unwrap();
        fs::write(&nvram, b"nvram").unwrap();
        fs::write(&startup, b"hostname R42\n").unwrap();

        remove_auto_delete_disk_files(directory.path(), "c7200", 42).unwrap();

        assert!(!disk.exists());
        assert!(!nvram.exists());
        assert!(startup.exists());
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
            chassis: None,
            slot0: "C7200-IO-FE".to_owned(),
            image: directory.path().join("IOS image supplied by the operator"),
            ram_mb: 256,
            nvram: 128,
            idle_pc: None,
            startup_config: String::new(),
            working_dir: working_dir.clone(),
            hypervisor_port: super::available_tcp_port().await.unwrap(),
            console_port: super::available_tcp_port().await.unwrap(),
            aux_port: None,
            log_file: working_dir.join("dynamips.log"),
            options: super::DynamipsOptions::default(),
            ghost_file: None,
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

    #[tokio::test]
    #[ignore = "requires the installed Cisco 3620 IOS image and Dynamips; set NETWORK_SIMULATOR_TEST_DYNAMIPS and NETWORK_SIMULATOR_TEST_C3600_IMAGE"]
    async fn upstream_dynamips_boots_a_cisco_3620_and_binds_its_fast_ethernet_port() {
        use crate::runtime::NodeAction;

        let binary = env::var_os("NETWORK_SIMULATOR_TEST_DYNAMIPS")
            .expect("NETWORK_SIMULATOR_TEST_DYNAMIPS points to upstream Dynamips");
        let image = env::var_os("NETWORK_SIMULATOR_TEST_C3600_IMAGE")
            .expect("NETWORK_SIMULATOR_TEST_C3600_IMAGE points to a Cisco 3620 IOS image");
        let image = PathBuf::from(image);
        let image_directory = image.parent().expect("IOS image has a parent directory");
        let directory = tempfile::tempdir().unwrap();
        let runtime =
            DynamipsRuntime::with_paths(binary, image_directory, directory.path().join("state"));
        let project_id = "00000000-0000-0000-0000-00000000000b";
        let node_id = "00000000-0000-0000-0000-00000000000c";
        let vm = runtime
            .create_router(
                project_id,
                node_id,
                "R3620",
                image.to_str().unwrap(),
                "c3600",
                Some("3620"),
                Some("NM-1FE-TX"),
                192,
                256,
                Some("0x603a8bac"),
                "hostname R3620\n",
            )
            .await
            .expect("Dynamips creates the c3600/3620 router and Fast Ethernet module");

        let exercise = async {
            runtime.lifecycle(&vm, NodeAction::Start).await?;
            let switch_socket =
                tokio::net::UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
                    .await
                    .map_err(|error| super::RuntimeError::Unavailable(error.to_string()))?;
            let switch_port = switch_socket.local_addr().unwrap().port();
            runtime.switch_endpoint(&vm, 0, 0, switch_port).await?;
            runtime.remove_switch_endpoint(&vm).await?;
            Ok::<(), super::RuntimeError>(())
        }
        .await;
        let cleanup = runtime.remove_router(&vm).await;
        cleanup.expect("the Dynamips hypervisor and temporary node files are removed");
        exercise.expect("the Cisco 3620 IOS image boots under Dynamips and exposes Ethernet");
    }
}
