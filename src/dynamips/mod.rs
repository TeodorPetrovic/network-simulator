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

mod config;
mod config_files;
mod idlepc;
mod lifecycle;
mod lifecycle_files;
mod lifecycle_process;
mod nio;
mod platforms;
mod process;
mod protocol;

mod version;

pub use config::DynamipsOptions;
use config_files::*;
use idlepc::*;
use lifecycle_files::*;
use platforms::*;
use protocol::*;

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
    bindings: Arc<Mutex<HashMap<(String, u32, u32), NioBinding>>>,
    operation_lock: Arc<Mutex<()>>,
}

#[derive(Clone, Debug)]
struct NioBinding {
    local_port: u16,
    nio_name: String,
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

pub(crate) use config_files::validate_ios_image;

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
}

impl Default for DynamipsRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
