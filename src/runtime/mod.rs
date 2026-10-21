use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UnixStream},
    process::{Child, Command},
    task::JoinHandle,
    time::timeout,
};
use uuid::Uuid;

const DOCKER_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct DockerRuntime {
    binary: PathBuf,
    state_dir: PathBuf,
    socket: PathBuf,
    console_hosts: std::sync::Arc<tokio::sync::Mutex<HashMap<String, DockerConsoleHost>>>,
}

struct DockerConsoleHost {
    console_type: String,
    console_port: u16,
    http_port: u16,
    http_path: String,
    resolution: String,
    display: Option<u16>,
    vnc_processes: Vec<Child>,
    http_task: Option<JoinHandle<()>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DockerTemplateOptions {
    #[serde(default)]
    pub environment: String,
    #[serde(default)]
    pub extra_hosts: String,
    #[serde(default)]
    pub extra_volumes: Vec<String>,
    #[serde(default)]
    pub extra_configs: Vec<DockerExtraConfig>,
    #[serde(default)]
    pub startup_config_content: Option<String>,
    #[serde(default = "default_docker_console_type")]
    pub console_type: String,
    #[serde(default = "default_docker_console_resolution")]
    pub console_resolution: String,
    #[serde(default)]
    pub console_http_port: Option<u16>,
    #[serde(default)]
    pub console_http_path: Option<String>,
    #[serde(default = "default_docker_aux_type")]
    pub aux_type: String,
    #[serde(default)]
    pub custom_adapters: Vec<DockerCustomAdapter>,
}

impl Default for DockerTemplateOptions {
    fn default() -> Self {
        Self {
            environment: String::new(),
            extra_hosts: String::new(),
            extra_volumes: Vec::new(),
            extra_configs: Vec::new(),
            startup_config_content: None,
            console_type: default_docker_console_type(),
            console_resolution: default_docker_console_resolution(),
            console_http_port: Some(80),
            console_http_path: Some("/".to_owned()),
            aux_type: default_docker_aux_type(),
            custom_adapters: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct DockerExtraConfig {
    pub target: String,
    #[serde(default)]
    pub content: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct DockerCustomAdapter {
    pub adapter_number: u32,
    #[serde(default)]
    pub port_name: Option<String>,
    #[serde(default)]
    pub adapter_type: Option<String>,
    #[serde(default)]
    pub mac_address: Option<String>,
}

#[derive(Clone, Debug)]
pub enum RuntimeError {
    Unavailable(String),
    NotFound(String),
    Failed(String),
}

impl RuntimeError {
    pub fn message(&self) -> &str {
        match self {
            Self::Unavailable(message) | Self::NotFound(message) | Self::Failed(message) => message,
        }
    }
}

mod containers;

mod console;

mod status;

mod management;

pub fn parse_start_command(command: &str) -> Result<Vec<String>, RuntimeError> {
    if command.len() > 4_096 {
        return Err(RuntimeError::Failed(
            "Docker start_command cannot exceed 4096 bytes".to_owned(),
        ));
    }
    let words = shlex::split(command).ok_or_else(|| {
        RuntimeError::Failed("Docker start_command has unmatched quote characters".to_owned())
    })?;
    if words.is_empty() {
        return Err(RuntimeError::Failed(
            "Docker start_command must contain a command".to_owned(),
        ));
    }
    Ok(words)
}

impl Default for DockerRuntime {
    fn default() -> Self {
        Self::new()
    }
}

mod helpers;
use self::helpers::*;
pub use self::helpers::validate_docker_extra_configs;

#[derive(Serialize)]
pub struct RuntimeInfo {
    pub available: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum NodeAction {
    Start,
    Stop,
    Suspend,
    Reload,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ImageRow {
    repository: String,
    tag: String,
    #[serde(rename = "ID")]
    id: String,
}

fn node_network_name(node_id: &str) -> String {
    format!("ns-node-{}", node_id.replace('-', ""))
}

fn docker_hostname(name: &str, node_id: &str) -> String {
    let mut hostname = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .chars()
        .take(63)
        .collect::<String>();
    if hostname.is_empty() {
        hostname = format!("node-{}", node_id.chars().take(8).collect::<String>());
    }
    hostname
}

fn normalize_docker_state(state: &str) -> String {
    match state {
        "running" => "started".to_owned(),
        "paused" => "suspended".to_owned(),
        _ => "stopped".to_owned(),
    }
}

#[cfg(test)]
mod tests;
