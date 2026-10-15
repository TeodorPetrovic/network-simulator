use std::{collections::HashMap, path::PathBuf, process::Stdio, time::Duration};

use serde::Serialize;
use serde_json::Value;
use tokio::{process::Command, time::timeout};
use uuid::Uuid;

const DOCKER_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone)]
pub struct DockerRuntime {
    binary: PathBuf,
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
        }
    }

    pub async fn info(&self) -> RuntimeInfo {
        match self
            .command(["info", "--format", "{{.ServerVersion}}"], DOCKER_TIMEOUT)
            .await
        {
            Ok(version) => RuntimeInfo {
                available: true,
                version: Some(version.trim().to_owned()),
                error: None,
            },
            Err(error) => RuntimeInfo {
                available: false,
                version: None,
                error: Some(error.message().to_owned()),
            },
        }
    }

    pub async fn templates(&self) -> Result<Vec<Value>, RuntimeError> {
        let output = self
            .command(
                [
                    "image",
                    "ls",
                    "--digests",
                    "--no-trunc",
                    "--format",
                    "{{json .}}",
                ],
                DOCKER_TIMEOUT,
            )
            .await?;
        let mut templates = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for line in output.lines().filter(|line| !line.trim().is_empty()) {
            let image: ImageRow = serde_json::from_str(line).map_err(|error| {
                RuntimeError::Failed(format!("Docker returned an invalid image row: {error}"))
            })?;
            if image.repository == "<none>" || image.tag == "<none>" || image.id.is_empty() {
                continue;
            }
            let reference = format!("{}:{}", image.repository, image.tag);
            if !seen.insert(reference.clone()) {
                continue;
            }
            let template_id = Uuid::new_v5(&Uuid::NAMESPACE_URL, reference.as_bytes());
            templates.push(serde_json::json!({
                "template_id": template_id,
                "name": reference,
                "template_type": "docker",
                "compute_id": "local",
                "category": "guest",
                "symbol": "docker_guest",
                "builtin": false,
                "image": reference,
                "image_digest": image.id,
                "start_command": "",
                "adapters": 8,
                "memory": 0,
                "cpus": 0.0
            }));
        }
        templates.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
        Ok(templates)
    }

    pub async fn create_container(
        &self,
        project_id: &str,
        node_id: &str,
        image: &str,
        name: &str,
        start_command: Option<&str>,
        memory_mb: Option<u64>,
        cpus: Option<f64>,
    ) -> Result<(String, String), RuntimeError> {
        let image_id = self.image_id(image).await?;
        let node_network = node_network_name(node_id);
        self.command(
            [
                "network",
                "create",
                "--driver",
                "bridge",
                "--internal",
                "--label",
                &format!("network-simulator.project-id={project_id}"),
                "--label",
                &format!("network-simulator.node-id={node_id}"),
                &node_network,
            ],
            DOCKER_TIMEOUT,
        )
        .await?;
        let container_name = format!("network-simulator-{node_id}");
        let mut args = vec![
            "create".to_owned(),
            "--name".to_owned(),
            container_name,
            "--network".to_owned(),
            node_network.clone(),
            "--label".to_owned(),
            format!("network-simulator.project-id={project_id}"),
            "--label".to_owned(),
            format!("network-simulator.node-id={node_id}"),
            "--label".to_owned(),
            format!("network-simulator.node-name={name}"),
            "--init".to_owned(),
        ];
        if let Some(memory_mb) = memory_mb.filter(|memory| *memory > 0) {
            args.extend(["--memory".to_owned(), format!("{memory_mb}m")]);
        }
        if let Some(cpus) = cpus.filter(|cpus| *cpus > 0.0) {
            args.extend(["--cpus".to_owned(), cpus.to_string()]);
        }
        args.push(image_id.clone());
        if let Some(command) = start_command.filter(|command| !command.trim().is_empty()) {
            args.extend(parse_start_command(command)?);
        }
        let container_id = match self.command(args, DOCKER_TIMEOUT).await {
            Ok(container_id) => container_id.trim().to_owned(),
            Err(error) => {
                let _ = self.remove_network(&node_network).await;
                return Err(error);
            }
        };
        if container_id.len() < 12 || !container_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            let _ = self.remove_network(&node_network).await;
            return Err(RuntimeError::Failed(
                "Docker returned an invalid container ID".to_owned(),
            ));
        }
        Ok((container_id, image_id))
    }

    pub async fn remove_node_resources(
        &self,
        node_id: &str,
        container_id: Option<&str>,
    ) -> Result<(), RuntimeError> {
        if let Some(container_id) = container_id {
            self.remove_container(container_id).await?;
        }
        self.remove_network(&node_network_name(node_id)).await
    }

    pub async fn remove_container(&self, container_id: &str) -> Result<(), RuntimeError> {
        match self
            .command(["rm", "--force", container_id], DOCKER_TIMEOUT)
            .await
        {
            Ok(_) => Ok(()),
            Err(RuntimeError::Failed(message))
                if message.contains("No such container") || message.contains("not found") =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    pub async fn lifecycle(
        &self,
        container_id: &str,
        action: NodeAction,
    ) -> Result<String, RuntimeError> {
        let status = self.container_status(container_id).await?;
        match action {
            NodeAction::Start if status == "suspended" => {
                self.command(["unpause", container_id], DOCKER_TIMEOUT)
                    .await?;
            }
            NodeAction::Start if status != "started" => {
                self.command(["start", container_id], DOCKER_TIMEOUT)
                    .await?;
            }
            NodeAction::Start => {}
            NodeAction::Stop if status == "started" || status == "suspended" => {
                if status == "suspended" {
                    self.command(["unpause", container_id], DOCKER_TIMEOUT)
                        .await?;
                }
                self.command(["stop", "--time", "10", container_id], DOCKER_TIMEOUT)
                    .await?;
            }
            NodeAction::Stop => {}
            NodeAction::Suspend if status == "started" => {
                self.command(["pause", container_id], DOCKER_TIMEOUT)
                    .await?;
            }
            NodeAction::Suspend if status == "suspended" => {}
            NodeAction::Suspend => {
                return Err(RuntimeError::Failed(
                    "Only a running Docker node can be suspended".to_owned(),
                ));
            }
            NodeAction::Reload => {
                self.command(["restart", "--time", "10", container_id], DOCKER_TIMEOUT)
                    .await?;
            }
        }
        self.container_status(container_id).await
    }

    pub async fn container_status(&self, container_id: &str) -> Result<String, RuntimeError> {
        let status = self
            .command(
                ["inspect", "--format", "{{.State.Status}}", container_id],
                DOCKER_TIMEOUT,
            )
            .await?;
        Ok(normalize_docker_state(status.trim()))
    }

    pub async fn create_link(
        &self,
        project_id: &str,
        link_id: &str,
        endpoints: &[Value],
    ) -> Result<(), RuntimeError> {
        let network = network_name(link_id);
        let containers = endpoints
            .iter()
            .map(|endpoint| {
                endpoint["container_id"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        RuntimeError::Failed("A link endpoint has no Docker container".to_owned())
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        self.command(
            [
                "network",
                "create",
                "--driver",
                "bridge",
                "--internal",
                "--label",
                &format!("network-simulator.project-id={project_id}"),
                "--label",
                &format!("network-simulator.link-id={link_id}"),
                &network,
            ],
            DOCKER_TIMEOUT,
        )
        .await?;

        let mut connected: Vec<String> = Vec::new();
        for container_id in containers {
            if let Err(error) = self
                .command(
                    ["network", "connect", &network, &container_id],
                    DOCKER_TIMEOUT,
                )
                .await
            {
                for connected_container in connected {
                    let _ = self
                        .command(
                            [
                                "network",
                                "disconnect",
                                "--force",
                                &network,
                                connected_container.as_str(),
                            ],
                            DOCKER_TIMEOUT,
                        )
                        .await;
                }
                let _ = self.remove_network(&network).await;
                return Err(error);
            }
            connected.push(container_id);
        }
        Ok(())
    }

    pub async fn remove_link(
        &self,
        link_id: &str,
        endpoints: &[Value],
    ) -> Result<(), RuntimeError> {
        let network = network_name(link_id);
        for endpoint in endpoints {
            let Some(container_id) = endpoint["container_id"].as_str() else {
                continue;
            };
            match self
                .command(
                    ["network", "disconnect", "--force", &network, container_id],
                    DOCKER_TIMEOUT,
                )
                .await
            {
                Ok(_) => {}
                Err(RuntimeError::Failed(message))
                    if message.contains("No such network")
                        || message.contains("not connected")
                        || message.contains("No such container") =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
        self.remove_network(&network).await
    }

    pub async fn project_node_states(
        &self,
        project_id: &str,
    ) -> Result<HashMap<String, String>, RuntimeError> {
        let output = self
            .command(
                [
                    "ps",
                    "--all",
                    "--filter",
                    &format!("label=network-simulator.project-id={project_id}"),
                    "--format",
                    "{{.Label \"network-simulator.node-id\"}} {{.State}}",
                ],
                DOCKER_TIMEOUT,
            )
            .await?;
        Ok(output
            .lines()
            .filter_map(|line| {
                let (node_id, state) = line.trim().split_once(' ')?;
                if node_id.is_empty() {
                    return None;
                }
                Some((node_id.to_owned(), normalize_docker_state(state)))
            })
            .collect())
    }

    async fn image_id(&self, image: &str) -> Result<String, RuntimeError> {
        let output = self
            .command(
                ["image", "inspect", "--format", "{{.Id}}", image],
                DOCKER_TIMEOUT,
            )
            .await
            .map_err(|error| match error {
                RuntimeError::Failed(message) if message.contains("No such image") => {
                    RuntimeError::NotFound(format!(
                        "Docker image '{image}' is not installed locally. Pull it on this host first."
                    ))
                }
                other => other,
            })?;
        Ok(output.trim().to_owned())
    }

    async fn remove_network(&self, network: &str) -> Result<(), RuntimeError> {
        match self
            .command(["network", "rm", network], DOCKER_TIMEOUT)
            .await
        {
            Ok(_) => Ok(()),
            Err(RuntimeError::Failed(message))
                if message.contains("No such network") || message.contains("not found") =>
            {
                Ok(())
            }
            Err(error) => Err(error),
        }
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

#[derive(Serialize)]
pub struct RuntimeInfo {
    pub available: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Copy)]
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

fn network_name(link_id: &str) -> String {
    format!("ns-link-{}", link_id.replace('-', ""))
}

fn node_network_name(node_id: &str) -> String {
    format!("ns-node-{}", node_id.replace('-', ""))
}

fn normalize_docker_state(state: &str) -> String {
    match state {
        "running" => "started".to_owned(),
        "paused" => "suspended".to_owned(),
        _ => "stopped".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        NodeAction, network_name, node_network_name, normalize_docker_state, parse_start_command,
    };

    #[test]
    fn maps_docker_states_to_controller_states() {
        assert_eq!(normalize_docker_state("running"), "started");
        assert_eq!(normalize_docker_state("paused"), "suspended");
        assert_eq!(normalize_docker_state("exited"), "stopped");
        let _ = NodeAction::Start;
    }

    #[test]
    fn derives_stable_safe_names_for_link_networks() {
        assert_eq!(network_name("ab12-cd34"), "ns-link-ab12cd34");
        assert_eq!(node_network_name("ab12-cd34"), "ns-node-ab12cd34");
    }

    #[test]
    fn parses_docker_command_without_running_a_host_shell() {
        assert_eq!(
            parse_start_command("/bin/sh -c 'sleep 60'").unwrap(),
            ["/bin/sh", "-c", "sleep 60"]
        );
        assert!(parse_start_command("/bin/sh -c 'sleep 60").is_err());
    }
}
