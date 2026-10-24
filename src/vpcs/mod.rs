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

mod process;
pub use self::process::reap_orphaned_processes;
use self::process::*;

#[derive(Serialize)]
pub struct VpcsRuntimeInfo {
    pub available: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

mod runtime_impl;

impl Default for VpcsRuntime {
    fn default() -> Self {
        Self::new()
    }
}

mod startup;
use self::startup::*;

#[cfg(test)]
mod tests;
