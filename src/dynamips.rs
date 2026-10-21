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

#[path = "dynamips/process.rs"]
mod process;
#[path = "dynamips/version.rs"]
mod version;

use process::{kill_dynamips_process_tree, reap_orphaned_dynamips_processes};
#[cfg(test)]
use version::supported_dynamips_version;
use version::{
    CONSOLE_BINDING_MIN_VERSION, dynamips_version_error, dynamips_version_tuple,
    dynamips_version_warning,
};

const GNS3_DEFAULT_IDLE_MAX: u32 = 500;
const GNS3_DEFAULT_IDLE_SLEEP: u32 = 30;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(8);
const IDLE_PC_PROPOSAL_TIMEOUT: Duration = Duration::from_secs(30);
/// Stop probing once a candidate keeps the router below this share of a core.
const IDLE_PC_CPU_TARGET_PERCENT: f64 = 10.0;
/// GNS3's `auto_idlepc` acceptance limit.
const GNS3_IDLE_PC_CPU_LIMIT_PERCENT: f64 = 70.0;
const IDLE_PC_CPU_SAMPLE_TIME: Duration = Duration::from_secs(5);
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
    process_group_id: u32,
    group_killed: bool,
    hypervisor_port: u16,
    log_file: PathBuf,
}

impl ManagedRouter {
    fn kill_group(&mut self) {
        if !self.group_killed {
            kill_dynamips_process_tree(self.process_group_id);
            self.group_killed = true;
        }
    }
}

impl Drop for ManagedRouter {
    fn drop(&mut self) {
        self.kill_group();
    }
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
            iomem: None,
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
    /// Non-blocking compatibility note for a usable binary.
    pub warning: Option<String>,
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
        let detected_version = self.detected_version().await;
        let (version, error) = match detected_version {
            Ok(version) => {
                let error = dynamips_version_error(&version);
                (Some(version), error)
            }
            Err(error) => (None, Some(error.message().to_owned())),
        };
        let warning = version
            .as_deref()
            .filter(|_| error.is_none())
            .and_then(dynamips_version_warning);
        let images = self.image_files().unwrap_or_default();
        let image_directory = self
            .image_dir
            .canonicalize()
            .unwrap_or_else(|_| self.image_dir.clone())
            .display()
            .to_string();
        DynamipsRuntimeInfo {
            available: error.is_none(),
            version,
            binary: self.binary.display().to_string(),
            image_directory,
            image_count: images.len(),
            error,
            warning,
        }
    }

    /// Kill Dynamips processes left behind by an earlier controller run.
    /// Dynamips is scoped by its working directory under `state_dir`.
    pub fn reap_orphans(&self) -> usize {
        reap_orphaned_dynamips_processes(&self.binary, &self.state_dir)
    }

    /// Kill every hypervisor process owned by this runtime before shutdown.
    pub async fn kill_all(&self) {
        let mut routers = self.routers.lock().await;
        for (_, mut router) in routers.drain() {
            router.kill_group();
            let _ = timeout(Duration::from_secs(2), router.child.wait()).await;
            let _ = fs::remove_file(&router.log_file);
        }
        self.bindings.lock().await.clear();
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
                        "Dynamips idle-PC must be a hexadecimal address".to_owned(),
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
                        "Dynamips idle-PC must be a hexadecimal address".to_owned(),
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
            "iomem": vm.options.iomem.or_else(|| default_iomem(&vm.platform)),
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
            NodeAction::Stop => {
                if self.reap_dead_hypervisor(&vm.node_id).await {
                    self.bindings.lock().await.remove(&vm.node_id);
                    return Ok("stopped".to_owned());
                }
                self.stop_locked(vm).await
            }
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
        if self.router_process_exited(&vm.node_id).await {
            return Ok("stopped".to_owned());
        }
        self.hypervisor_status(vm.hypervisor_port, &vm.name).await
    }

    pub async fn set_idle_pc(&self, vm: &DynamipsVm, idle_pc: &str) -> Result<(), RuntimeError> {
        if !is_hex_pointer(idle_pc) {
            return Err(RuntimeError::Failed(
                "Idle-PC must be a hexadecimal pointer".to_owned(),
            ));
        }
        let _guard = self.operation_lock.lock().await;
        if self.hypervisor_status(vm.hypervisor_port, &vm.name).await? != "stopped" {
            return Err(RuntimeError::Failed(
                "Stop the Dynamips router before changing its Idle-PC value".to_owned(),
            ));
        }
        self.send(
            vm.hypervisor_port,
            &format!("vm set_idle_pc {} {idle_pc}", quote(&vm.name)),
        )
        .await?;
        Ok(())
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
            // Dynamips only searches for candidates while no online Idle-PC
            // value is active. Restore the configured value after the probe.
            self.send(
                vm.hypervisor_port,
                &format!("vm set_idle_pc_online {} 0 0x0", quote(&vm.name)),
            )
            .await?;
            self.send_with_timeout(
                vm.hypervisor_port,
                &format!("vm get_idle_pc_prop {} 0", quote(&vm.name)),
                IDLE_PC_PROPOSAL_TIMEOUT,
            )
            .await
        }
        .await;
        let stopped = if was_auto_started {
            match self.stop_locked(vm).await {
                Ok(_) => self
                    .send(
                        vm.hypervisor_port,
                        &format!(
                            "vm set_idle_pc {} {}",
                            quote(&vm.name),
                            vm.idle_pc.as_deref().unwrap_or("0x0")
                        ),
                    )
                    .await
                    .map(|_| ()),
                Err(error) => Err(error),
            }
        } else {
            self.send(
                vm.hypervisor_port,
                &format!(
                    "vm set_idle_pc_online {} 0 {}",
                    quote(&vm.name),
                    vm.idle_pc.as_deref().unwrap_or("0x0")
                ),
            )
            .await
            .map(|_| ())
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
                .send_with_timeout(
                    vm.hypervisor_port,
                    &format!("vm get_idle_pc_prop {} 0", quote(&vm.name)),
                    IDLE_PC_PROPOSAL_TIMEOUT,
                )
                .await?;
            let process_id = self
                .routers
                .lock()
                .await
                .get(&vm.node_id)
                .and_then(|router| router.child.id());
            let ticks_per_second = process_ticks_per_second();
            let mut best: Option<(String, f64)> = None;
            let mut applied = None;
            for proposal in proposals {
                let Some(candidate) = idlepc_candidate(&proposal) else {
                    continue;
                };
                self.send(
                    vm.hypervisor_port,
                    &format!("vm set_idle_pc_online {} 0 {candidate}", quote(&vm.name)),
                )
                .await?;
                applied = Some(candidate.to_owned());
                let process_sample =
                    process_id
                        .zip(ticks_per_second)
                        .and_then(|(pid, ticks_per_second)| {
                            read_process_cpu_ticks(pid).map(|ticks| (pid, ticks_per_second, ticks))
                        });
                let cpu_seconds_before = self.cpu_usage(vm.hypervisor_port, &vm.name).await?;
                let started = tokio::time::Instant::now();
                sleep(IDLE_PC_CPU_SAMPLE_TIME).await;
                let process_cpu_percent =
                    process_sample.and_then(|(pid, ticks_per_second, before)| {
                        read_process_cpu_ticks(pid).map(|after| {
                            process_cpu_rate(before, after, started.elapsed(), ticks_per_second)
                        })
                    });
                let cpu_percent = if let Some(cpu_percent) = process_cpu_percent {
                    cpu_percent
                } else {
                    let after = self.cpu_usage(vm.hypervisor_port, &vm.name).await?;
                    idlepc_cpu_rate(cpu_seconds_before, after, started.elapsed())
                };
                tracing::debug!(
                    node_id = %vm.node_id,
                    idle_pc = candidate,
                    cpu_percent,
                    "Validated Dynamips Idle-PC candidate"
                );
                if cpu_percent < IDLE_PC_CPU_TARGET_PERCENT {
                    return Ok(candidate.to_owned());
                }
                if best
                    .as_ref()
                    .is_none_or(|(_, best_percent)| cpu_percent < *best_percent)
                {
                    best = Some((candidate.to_owned(), cpu_percent));
                }
            }
            // GNS3 accepts the first candidate below 70%. Prefer one that
            // reaches the idle target, otherwise keep GNS3's acceptance rule
            // but return the lowest measured candidate instead of the first.
            match best {
                Some((candidate, cpu_percent)) if cpu_percent < GNS3_IDLE_PC_CPU_LIMIT_PERCENT => {
                    tracing::warn!(
                        node_id = %vm.node_id,
                        idle_pc = %candidate,
                        cpu_percent,
                        "No Dynamips Idle-PC candidate reached the idle CPU target; using the lowest"
                    );
                    if applied.as_deref() != Some(candidate.as_str()) {
                        self.send(
                            vm.hypervisor_port,
                            &format!("vm set_idle_pc_online {} 0 {candidate}", quote(&vm.name)),
                        )
                        .await?;
                    }
                    Ok(candidate)
                }
                _ => Err(RuntimeError::Failed(
                    "Sorry, no idle-pc value was suitable".to_owned(),
                )),
            }
        }
        .await;
        let stopped = if was_auto_started {
            self.stop_locked(vm).await.map(|_| ())
        } else {
            if probe.is_err() {
                // Leave a running router on the Idle-PC it had before probing.
                let _ = self
                    .send(
                        vm.hypervisor_port,
                        &format!(
                            "vm set_idle_pc_online {} 0 {}",
                            quote(&vm.name),
                            vm.idle_pc.as_deref().unwrap_or("0x0")
                        ),
                    )
                    .await;
            }
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
                router.kill_group();
                let _ = timeout(Duration::from_secs(3), router.child.wait()).await;
            }
            let _ = fs::remove_file(&router.log_file);
        }
        remove_transient_runtime_files(&vm.working_dir)?;
        remove_runtime_manifest(&vm.working_dir)?;
        remove_generated_runtime_configs(&vm.working_dir)?;
        self.bindings.lock().await.remove(&vm.node_id);
        if vm.options.auto_delete_disks {
            remove_auto_delete_disk_files(&vm.working_dir, &vm.platform, vm.options.dynamips_id)?;
        }
        remove_empty_runtime_directories(&vm.working_dir)?;
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

    async fn detected_version(&self) -> Result<String, RuntimeError> {
        let mut command = Command::new(&self.binary);
        command
            .arg("-P")
            .arg("none")
            .stdin(Stdio::null())
            .kill_on_drop(true);
        // Dynamips names its log/VM files relative to the process cwd. Keep
        // this probe out of the controller's directory (the repository root in
        // development) even if a build writes a default log for it.
        if fs::create_dir_all(&self.state_dir).is_ok() {
            command.current_dir(&self.state_dir);
        }
        let output = timeout(COMMAND_TIMEOUT, command.output())
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
    }

    async fn version(&self) -> Result<String, RuntimeError> {
        let version = self.detected_version().await?;
        dynamips_version_error(&version).map_or_else(
            || Ok(version.clone()),
            |message| Err(RuntimeError::Unavailable(message)),
        )
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
        self.version().await?;
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
        if let Some(command) = router_start_idlepc_command(vm) {
            self.send(vm.hypervisor_port, &command).await?;
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
                // Dynamips writes the ghost RAM image while `vm start` loads
                // the IOS ELF, so GNS3 stops the ghost instance immediately.
                self.send(vm.hypervisor_port, &format!("vm start {}", quote(name)))
                    .await?;
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
                stale.kill_group();
                let _ = timeout(Duration::from_secs(2), stale.child.wait()).await;
            }
            let _ = fs::remove_file(&stale.log_file);
        }
        remove_transient_runtime_files(&vm.working_dir)?;
        self.bindings.lock().await.remove(&vm.node_id);
        self.initialize_router(vm).await
    }

    async fn spawn_hypervisor(&self, vm: &DynamipsVm) -> Result<ManagedRouter, RuntimeError> {
        let version = self.version().await?;
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
        let mut command = Command::new(&self.binary);
        command
            .arg("-N1")
            .arg("-l")
            .arg("dynamips_engine.log")
            .arg("-H")
            .arg(format!("127.0.0.1:{port}"));
        // GNS3 uses this flag with Dynamips 0.2.23 and newer. Without it,
        // console and AUX listeners may bind to every host interface even
        // though the hypervisor protocol itself listens only on loopback.
        if dynamips_version_tuple(&version)
            .is_some_and(|version| version >= CONSOLE_BINDING_MIN_VERSION)
        {
            command.args(["--console-binding-addr", "127.0.0.1"]);
        }
        let child = command
            .current_dir(&vm.working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .process_group(0)
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                RuntimeError::Unavailable(format!(
                    "Could not start Dynamips executable '{}': {error}",
                    self.binary.display()
                ))
            })?;
        // Keep the group-owning guard alive during the readiness probe too:
        // canceling a start future must not leave a hypervisor descendant.
        let mut router = ManagedRouter {
            process_group_id: child.id().expect("spawned Dynamips has a PID"),
            child,
            group_killed: false,
            hypervisor_port: port,
            log_file,
        };
        let deadline = tokio::time::Instant::now() + PROCESS_READY_TIMEOUT;
        loop {
            if TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))
                .await
                .is_ok()
            {
                return Ok(router);
            }
            match router.child.try_wait() {
                Ok(Some(status)) => {
                    router.kill_group();
                    let _ = router.child.wait().await;
                    let log = fs::read_to_string(&router.log_file).unwrap_or_default();
                    return Err(RuntimeError::Failed(format!(
                        "Dynamips exited during startup ({status}){}",
                        if log.trim().is_empty() {
                            String::new()
                        } else {
                            format!(": {}", log.trim())
                        }
                    )));
                }
                Ok(None) => {}
                Err(error) => {
                    router.kill_group();
                    let _ = timeout(Duration::from_secs(2), router.child.wait()).await;
                    return Err(RuntimeError::Unavailable(format!(
                        "Could not inspect Dynamips startup: {error}"
                    )));
                }
            }
            if tokio::time::Instant::now() >= deadline {
                router.kill_group();
                let _ = router.child.wait().await;
                let log = fs::read_to_string(&router.log_file).unwrap_or_default();
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
                router.kill_group();
                let _ = timeout(Duration::from_secs(2), router.child.wait()).await;
            }
            let _ = fs::remove_file(&router.log_file);
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

    async fn router_process_exited(&self, node_id: &str) -> bool {
        let mut routers = self.routers.lock().await;
        let Some(router) = routers.get_mut(node_id) else {
            return false;
        };
        match router.child.try_wait() {
            Ok(Some(_)) | Err(_) => true,
            Ok(None) => false,
        }
    }

    async fn reap_dead_hypervisor(&self, node_id: &str) -> bool {
        let dead_router = {
            let mut routers = self.routers.lock().await;
            let exited = routers
                .get_mut(node_id)
                .is_some_and(|router| !matches!(router.child.try_wait(), Ok(None)));
            exited.then(|| routers.remove(node_id)).flatten()
        };
        let Some(mut router) = dead_router else {
            return false;
        };
        router.kill_group();
        let _ = router.child.wait().await;
        let _ = fs::remove_file(&router.log_file);
        if let Some(working_dir) = router.log_file.parent() {
            let _ = remove_transient_runtime_files(working_dir);
        }
        self.bindings.lock().await.remove(node_id);
        true
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
        self.send_with_timeout(port, command, COMMAND_TIMEOUT).await
    }

    async fn send_with_timeout(
        &self,
        port: u16,
        command: &str,
        command_timeout: Duration,
    ) -> Result<Vec<String>, RuntimeError> {
        if command.chars().any(char::is_control) {
            return Err(RuntimeError::Failed(
                "Dynamips protocol commands cannot contain control characters".to_owned(),
            ));
        }
        timeout(command_timeout, async {
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

fn process_ticks_per_second() -> Option<u64> {
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    u64::try_from(ticks).ok().filter(|ticks| *ticks > 0)
}

fn read_process_cpu_ticks(pid: u32) -> Option<u64> {
    let tasks = fs::read_dir(format!("/proc/{pid}/task")).ok()?;
    let mut total_ticks = 0_u64;
    let mut found_thread = false;
    for task in tasks.flatten() {
        let Ok(stat) = fs::read_to_string(task.path().join("stat")) else {
            continue;
        };
        let Some(ticks) = process_thread_cpu_ticks(&stat) else {
            continue;
        };
        total_ticks = total_ticks.saturating_add(ticks);
        found_thread = true;
    }
    found_thread.then_some(total_ticks)
}

fn process_thread_cpu_ticks(stat: &str) -> Option<u64> {
    let fields = stat
        .rsplit_once(')')?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    let user_ticks = fields.get(11)?.parse::<u64>().ok()?;
    let system_ticks = fields.get(12)?.parse::<u64>().ok()?;
    Some(user_ticks.saturating_add(system_ticks))
}

fn process_cpu_rate(before: u64, after: u64, elapsed: Duration, ticks_per_second: u64) -> f64 {
    if ticks_per_second == 0 {
        return f64::INFINITY;
    }
    let elapsed_seconds = elapsed.as_secs_f64().max(0.001);
    after.saturating_sub(before) as f64 * 100.0 / ticks_per_second as f64 / elapsed_seconds
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

fn default_iomem(platform: &str) -> Option<u8> {
    match platform {
        "c1700" | "c2600" => Some(15),
        "c2691" | "c3600" | "c3725" | "c3745" => Some(5),
        _ => None,
    }
}

/// Return the adapter Dynamips installs as part of creating this platform.
/// GNS3's Dynamips node wrappers model these adapters in their slot state, but
/// the hypervisor creates them when the chassis/platform is initialized. Sending
/// `slot_add_binding` for them a second time fails on real Dynamips.
fn integrated_slot0(platform: &str, chassis: Option<&str>) -> Option<&'static str> {
    match platform {
        "c1700" => Some("C1700-MB-1FE"),
        "c2600" => Some(match chassis.unwrap_or("2610") {
            "2611" => "C2600-MB-2E",
            "2620" | "2610XM" | "2620XM" | "2650XM" => "C2600-MB-1FE",
            "2621" | "2611XM" | "2621XM" | "2651XM" => "C2600-MB-2FE",
            _ => "C2600-MB-1E",
        }),
        "c3600" if chassis == Some("3660") => Some("Leopard-2FE"),
        "c2691" | "c3725" | "c3745" => Some("GT96100-FE"),
        _ => None,
    }
}

fn integrated_slot_module(
    platform: &str,
    chassis: Option<&str>,
    slot: usize,
) -> Option<&'static str> {
    if slot == 0 {
        return integrated_slot0(platform, chassis);
    }
    match (platform, chassis, slot) {
        ("c1700", Some("1751" | "1760"), 1) => Some("C1700-MB-WIC1"),
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
    let wic_capacity = match platform {
        "c1700" => 2,
        "c2600" | "c2691" | "c3725" | "c3745" => 3,
        _ => 0,
    };
    if options.wics.iter().enumerate().any(|(slot, wic)| {
        slot >= wic_capacity && wic.as_deref().is_some_and(|wic| !wic.is_empty())
    }) {
        return Err(RuntimeError::Failed(format!(
            "Dynamips platform {platform} provides {wic_capacity} WIC slot(s)"
        )));
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
    } else if let Some(iomem) = vm.options.iomem.or_else(|| default_iomem(&vm.platform)) {
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
            if integrated_slot_module(&vm.platform, vm.chassis.as_deref(), slot) == Some(module) {
                continue;
            }
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

/// Idle tuning sent after `vm start`, when Dynamips has created the CPU.
///
/// GNS3 never sends its default `idlemax` 500 / `idlesleep` 30: its router
/// object starts with those values and only issues `vm set_idle_*` when an
/// edit changes them. Dynamips then keeps its own CPU defaults, `idle_max`
/// 500 and `idle_sleep_time` 30000. The sleep time is in microseconds
/// (`cpu_idle_loop`), so sending GNS3's `30` would shorten each idle sleep
/// from 30 ms to 30 µs and keep the host core busy even with a good Idle-PC.
/// Non-default values are passed through unchanged, as GNS3 does.
fn router_start_settings_commands(vm: &DynamipsVm) -> Vec<String> {
    let mut commands = Vec::new();
    if vm.options.idlemax != GNS3_DEFAULT_IDLE_MAX {
        commands.push(format!(
            "vm set_idle_max {} 0 {}",
            quote(&vm.name),
            vm.options.idlemax
        ));
    }
    if vm.options.idlesleep != GNS3_DEFAULT_IDLE_SLEEP {
        commands.push(format!(
            "vm set_idle_sleep_time {} 0 {}",
            quote(&vm.name),
            vm.options.idlesleep
        ));
    }
    commands
}

fn router_start_idlepc_command(vm: &DynamipsVm) -> Option<String> {
    vm.idle_pc
        .as_deref()
        .map(|idle_pc| format!("vm set_idle_pc {} {idle_pc}", quote(&vm.name)))
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

fn remove_transient_runtime_files(working_dir: &Path) -> Result<(), RuntimeError> {
    if !working_dir.exists() {
        return Ok(());
    }
    let entries = fs::read_dir(working_dir).map_err(|error| {
        RuntimeError::Failed(format!("Could not inspect Dynamips runtime files: {error}"))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            RuntimeError::Failed(format!("Could not inspect Dynamips runtime file: {error}"))
        })?;
        if !entry
            .file_type()
            .map_err(|error| {
                RuntimeError::Failed(format!("Could not inspect Dynamips runtime file: {error}"))
            })?
            .is_file()
        {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name == "dynamips_stdout.log"
            || name == "dynamips_engine.log"
            || name.starts_with("ilt_")
            || is_vm_transient_file(name)
        {
            fs::remove_file(entry.path()).map_err(|error| {
                RuntimeError::Failed(format!("Could not remove Dynamips runtime file: {error}"))
            })?;
        }
    }
    Ok(())
}

/// Per-instance files Dynamips recreates on every `vm create`/`vm start`
/// (`<platform>_i<id>_lock`, `_log.txt`, `_rommon_vars`). GNS3 deletes the same
/// set when a project closes; NVRAM, disks, and flash stay with the node.
fn is_vm_transient_file(name: &str) -> bool {
    let Some((instance, suffix)) = name
        .strip_prefix('c')
        .and_then(|rest| rest.split_once("_i"))
        .and_then(|(model, rest)| {
            (model.len() == 4 && model.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| rest.split_once('_'))
                .flatten()
        })
    else {
        return false;
    };
    !instance.is_empty()
        && instance.bytes().all(|byte| byte.is_ascii_digit())
        && matches!(suffix, "lock" | "log.txt" | "rommon_vars")
}

fn remove_runtime_manifest(working_dir: &Path) -> Result<(), RuntimeError> {
    match fs::remove_file(working_dir.join(RUNTIME_MANIFEST_FILE)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(RuntimeError::Failed(format!(
            "Could not remove Dynamips runtime ownership marker: {error}"
        ))),
    }
}

fn remove_empty_runtime_directories(working_dir: &Path) -> Result<(), RuntimeError> {
    for directory in [working_dir.join("configs"), working_dir.to_path_buf()] {
        match fs::remove_dir(&directory) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) => {
                return Err(RuntimeError::Failed(format!(
                    "Could not remove empty Dynamips runtime directory '{}': {error}",
                    directory.display()
                )));
            }
        }
    }
    if let Some(project_dir) = working_dir.parent() {
        match fs::remove_dir(project_dir) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) => {
                return Err(RuntimeError::Failed(format!(
                    "Could not remove empty Dynamips project directory '{}': {error}",
                    project_dir.display()
                )));
            }
        }
    }
    Ok(())
}

fn remove_generated_runtime_configs(working_dir: &Path) -> Result<(), RuntimeError> {
    let configs = working_dir.join("configs");
    match fs::remove_dir_all(&configs) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(RuntimeError::Failed(format!(
            "Could not remove generated Dynamips configs '{}': {error}",
            configs.display()
        ))),
    }
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
#[path = "dynamips/tests.rs"]
mod tests;
