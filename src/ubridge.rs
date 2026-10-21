use serde::Serialize;
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tokio::{process::Child, sync::Mutex};

use crate::runtime::RuntimeError;

mod bridge_forwarding;
mod common;
mod endpoints;
mod protocol;
mod runtime_ops;

#[cfg(test)]
mod live_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub use common::guest_veth_name;
pub use common::host_veth_name;

pub(super) use common::*;

pub(super) const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const START_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const MAX_REPLY_BYTES: usize = 256 * 1024;

#[derive(Clone)]
pub struct UbridgeRuntime {
    pub(super) binary: PathBuf,
    pub(super) socket: PathBuf,
    pub(super) daemon: Arc<Mutex<Option<Child>>>,
    pub(super) link_bridges: Arc<Mutex<HashMap<String, String>>>,
    pub(super) host_bridge_taps: Arc<Mutex<HashMap<String, (String, String)>>>,
    pub(super) allocated_udp_ports: Arc<Mutex<HashMap<u16, String>>>,
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
}

impl Default for UbridgeRuntime {
    fn default() -> Self {
        Self::new()
    }
}
