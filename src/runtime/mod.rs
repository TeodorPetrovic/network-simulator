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

impl DockerRuntime {
    pub fn new() -> Self {
        Self::with_binary(
            std::env::var_os("NETWORK_SIMULATOR_DOCKER")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("docker")),
        )
    }

    pub fn with_binary(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            state_dir: std::env::var_os("NETWORK_SIMULATOR_DOCKER_STATE")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("data/docker")),
            socket: docker_socket_path(),
            console_hosts: std::sync::Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    #[cfg(test)]
    pub fn with_paths(binary: impl Into<PathBuf>, state_dir: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            state_dir: state_dir.into(),
            socket: PathBuf::from("/var/run/docker.sock"),
            console_hosts: std::sync::Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    #[cfg(test)]
    pub fn with_paths_and_socket(
        binary: impl Into<PathBuf>,
        state_dir: impl Into<PathBuf>,
        socket: impl Into<PathBuf>,
    ) -> Self {
        Self {
            binary: binary.into(),
            state_dir: state_dir.into(),
            socket: socket.into(),
            console_hosts: std::sync::Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    fn managed_container_marker(&self, container_id: &str) -> Result<PathBuf, RuntimeError> {
        if !valid_container_id(container_id) {
            return Err(RuntimeError::Failed(
                "Docker returned an invalid container ID".to_owned(),
            ));
        }
        Ok(self
            .state_dir
            .join("managed-containers")
            .join(container_id.to_ascii_lowercase()))
    }

    fn mark_container_managed(
        &self,
        project_id: &str,
        node_id: &str,
        container_id: &str,
    ) -> Result<(), RuntimeError> {
        let marker = self.managed_container_marker(container_id)?;
        if marker.exists() {
            return self.ensure_managed_container(container_id);
        }
        let directory = marker.parent().expect("managed marker has a parent");
        fs::create_dir_all(directory).map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not create Docker runtime state directory: {error}"
            ))
        })?;
        let contents = format!("{project_id}\n{node_id}\n");
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)
            .and_then(|mut file| std::io::Write::write_all(&mut file, contents.as_bytes()))
        {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                self.ensure_managed_container(container_id)
            }
            Err(error) => Err(RuntimeError::Failed(format!(
                "Could not record Docker runtime ownership: {error}"
            ))),
        }
    }

    fn ensure_managed_container(&self, container_id: &str) -> Result<(), RuntimeError> {
        let marker = self.managed_container_marker(container_id)?;
        let metadata = fs::symlink_metadata(&marker).map_err(|_| {
            RuntimeError::Failed(
                "Docker container ID is not owned by this controller runtime".to_owned(),
            )
        })?;
        if !metadata.file_type().is_file() {
            return Err(RuntimeError::Failed(
                "Docker runtime ownership marker is invalid".to_owned(),
            ));
        }
        let contents = fs::read_to_string(marker).map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not read Docker runtime ownership marker: {error}"
            ))
        })?;
        let mut lines = contents.lines();
        let project = lines.next().unwrap_or_default();
        let node = lines.next().unwrap_or_default();
        if Uuid::parse_str(project).is_err()
            || Uuid::parse_str(node).is_err()
            || lines.next().is_some()
        {
            return Err(RuntimeError::Failed(
                "Docker runtime ownership marker is invalid".to_owned(),
            ));
        }
        Ok(())
    }

    fn forget_managed_container(&self, container_id: &str) {
        // Docker IDs are immutable and never recycled. Keeping this small
        // record allows the controller to recognize stale IDs after a restart.
        let _ = container_id;
    }

    async fn command<I, S>(&self, args: I, duration: Duration) -> Result<String, RuntimeError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let mut command = Command::new(&self.binary);
        command.args(args).stdin(Stdio::null()).kill_on_drop(true);
        let output = timeout(duration, command.output())
            .await
            .map_err(|_| RuntimeError::Unavailable("Docker command timed out".to_owned()))?
            .map_err(|error| {
                RuntimeError::Unavailable(format!(
                    "Could not run Docker CLI '{}': {error}",
                    self.binary.display()
                ))
            })?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if stderr.contains("Cannot connect to the Docker daemon")
            || stderr.contains("Is the docker daemon running")
            || stderr.contains("permission denied while trying to connect")
        {
            return Err(RuntimeError::Unavailable(stderr));
        }
        Err(RuntimeError::Failed(if stderr.is_empty() {
            format!("Docker command exited with {}", output.status)
        } else {
            stderr
        }))
    }



}

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
