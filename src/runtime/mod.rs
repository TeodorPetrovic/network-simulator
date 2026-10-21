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
                "environment": "",
                "extra_hosts": "",
                "extra_volumes": [],
                "console_type": "telnet",
                "console_resolution": "1024x768",
                "console_http_port": 80,
                "console_http_path": "/",
                "aux_type": "none",
                "custom_adapters": [],
                "adapters": 1,
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
        self.create_container_with_options(
            project_id,
            node_id,
            image,
            name,
            start_command,
            memory_mb,
            cpus,
            &DockerTemplateOptions::default(),
        )
        .await
    }

    pub async fn create_container_with_options(
        &self,
        project_id: &str,
        node_id: &str,
        image: &str,
        name: &str,
        start_command: Option<&str>,
        memory_mb: Option<u64>,
        cpus: Option<f64>,
        options: &DockerTemplateOptions,
    ) -> Result<(String, String), RuntimeError> {
        validate_docker_runtime_options(options)?;
        let container_name = format!("network-simulator-{node_id}");
        self.create_container_named(
            project_id,
            node_id,
            image,
            name,
            start_command,
            memory_mb,
            cpus,
            options,
            &container_name,
        )
        .await
    }

    async fn create_container_named(
        &self,
        project_id: &str,
        node_id: &str,
        image: &str,
        name: &str,
        start_command: Option<&str>,
        memory_mb: Option<u64>,
        cpus: Option<f64>,
        options: &DockerTemplateOptions,
        container_name: &str,
    ) -> Result<(String, String), RuntimeError> {
        validate_docker_runtime_options(options)?;
        let project_id = validate_docker_topology_id(project_id, "project")?;
        let node_id = validate_docker_topology_id(node_id, "node")?;
        let mut console_host = self.prepare_console_host(options).await?;
        let image_id = self.image_id(image).await?;
        let mut args = vec![
            "create".to_owned(),
            "--name".to_owned(),
            container_name.to_owned(),
            "--interactive".to_owned(),
            "--tty".to_owned(),
            "--network".to_owned(),
            "none".to_owned(),
            "--label".to_owned(),
            format!("network-simulator.project-id={project_id}"),
            "--label".to_owned(),
            format!("network-simulator.node-id={node_id}"),
            "--label".to_owned(),
            format!("network-simulator.node-name={name}"),
            "--init".to_owned(),
            "--hostname".to_owned(),
            docker_hostname(name, &node_id),
            "--env".to_owned(),
            "container=docker".to_owned(),
            // GNS3 runs appliance containers with the capabilities needed to
            // create interfaces, VLANs, routes, and packet captures.
            "--privileged".to_owned(),
        ];
        if let Some(host) = console_host.as_ref()
            && let Some(display) = host.display
        {
            args.extend([
                "--env".to_owned(),
                "QT_GRAPHICSSYSTEM=native".to_owned(),
                "--env".to_owned(),
                format!("DISPLAY=:{display}"),
                "--volume".to_owned(),
                "/tmp/.X11-unix:/tmp/.X11-unix:ro".to_owned(),
            ]);
        }
        // TODO(RUNTIME-NET Docker custom adapter hook): apply custom interface
        // names and MAC addresses while the NET workstream creates container NICs.
        if let Some(memory_mb) = memory_mb.filter(|memory| *memory > 0) {
            args.extend(["--memory".to_owned(), format!("{memory_mb}m")]);
        }
        if let Some(cpus) = cpus.filter(|cpus| *cpus > 0.0) {
            args.extend(["--cpus".to_owned(), cpus.to_string()]);
        }
        for variable in parse_environment(&options.environment)? {
            args.extend(["--env".to_owned(), variable]);
        }
        for host in parse_extra_hosts(&options.extra_hosts)? {
            args.extend(["--add-host".to_owned(), host]);
        }
        let volume_root = self
            .state_dir
            .join(&project_id)
            .join(&node_id)
            .join("volumes");
        if !options.extra_volumes.is_empty() {
            fs::create_dir_all(&volume_root).map_err(|error| {
                RuntimeError::Failed(format!("Could not create Docker volume directory: {error}"))
            })?;
        }
        for container_path in &options.extra_volumes {
            let relative = validate_container_volume(container_path)?;
            let host_path = volume_root.join(relative);
            fs::create_dir_all(&host_path).map_err(|error| {
                RuntimeError::Failed(format!(
                    "Could not create persistent Docker volume: {error}"
                ))
            })?;
            let host_path = host_path.canonicalize().map_err(|error| {
                RuntimeError::Failed(format!(
                    "Could not resolve persistent Docker volume: {error}"
                ))
            })?;
            if !host_path.starts_with(
                &volume_root
                    .canonicalize()
                    .unwrap_or_else(|_| volume_root.clone()),
            ) {
                return Err(RuntimeError::Failed(
                    "Docker volume path escaped its project directory".to_owned(),
                ));
            }
            args.extend([
                "--volume".to_owned(),
                format!("{}:{container_path}", host_path.display()),
            ]);
        }
        let config_root = self
            .state_dir
            .join(&project_id)
            .join(&node_id)
            .join("configs");
        let mut injected_configs = options.extra_configs.clone();
        if let Some(content) = options
            .startup_config_content
            .as_deref()
            .filter(|content| !content.is_empty())
        {
            injected_configs.push(DockerExtraConfig {
                target: "/startup-config".to_owned(),
                content: content.to_owned(),
            });
        }
        if !injected_configs.is_empty() {
            if config_root.exists() {
                fs::remove_dir_all(&config_root).map_err(|error| {
                    RuntimeError::Failed(format!(
                        "Could not reset Docker injected config directory: {error}"
                    ))
                })?;
            }
            fs::create_dir_all(&config_root).map_err(|error| {
                RuntimeError::Failed(format!(
                    "Could not create Docker injected config directory: {error}"
                ))
            })?;
            let canonical_root = config_root.canonicalize().map_err(|error| {
                RuntimeError::Failed(format!(
                    "Could not resolve Docker injected config directory: {error}"
                ))
            })?;
            for config in &injected_configs {
                let relative = validate_extra_config_target(&config.target)?;
                let host_path = canonical_root.join(relative);
                if let Some(parent) = host_path.parent() {
                    fs::create_dir_all(parent).map_err(|error| {
                        RuntimeError::Failed(format!(
                            "Could not create Docker injected config parent: {error}"
                        ))
                    })?;
                    let canonical_parent = parent.canonicalize().map_err(|error| {
                        RuntimeError::Failed(format!(
                            "Could not resolve Docker injected config parent: {error}"
                        ))
                    })?;
                    if !canonical_parent.starts_with(&canonical_root) {
                        return Err(RuntimeError::Failed(
                            "Docker injected config path escaped its node directory".to_owned(),
                        ));
                    }
                }
                let mut file = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&host_path)
                    .map_err(|error| {
                        RuntimeError::Failed(format!(
                            "Could not write Docker injected config: {error}"
                        ))
                    })?;
                use std::io::Write as _;
                file.write_all(config.content.as_bytes()).map_err(|error| {
                    RuntimeError::Failed(format!("Could not write Docker injected config: {error}"))
                })?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&host_path, fs::Permissions::from_mode(0o400)).map_err(
                        |error| {
                            RuntimeError::Failed(format!(
                                "Could not make Docker injected config read-only: {error}"
                            ))
                        },
                    )?;
                }
                args.extend([
                    "--volume".to_owned(),
                    format!("{}:{}:ro", host_path.display(), config.target),
                ]);
            }
        }
        args.push(image_id.clone());
        if let Some(command) = start_command.filter(|command| !command.trim().is_empty()) {
            args.extend(parse_start_command(command)?);
        }
        let container_id = self.command(args, DOCKER_TIMEOUT).await?.trim().to_owned();
        if !valid_container_id(&container_id) {
            return Err(RuntimeError::Failed(
                "Docker returned an invalid container ID".to_owned(),
            ));
        }
        if let Err(error) = self.mark_container_managed(&project_id, &node_id, &container_id) {
            let _ = self
                .command(["rm", "--force", &container_id], DOCKER_TIMEOUT)
                .await;
            if let Some(host) = console_host.take() {
                self.console_hosts
                    .lock()
                    .await
                    .insert(container_id.clone(), host);
                self.stop_console_host(&container_id).await;
            }
            return Err(error);
        }
        if let Some(host) = console_host.take() {
            self.console_hosts
                .lock()
                .await
                .insert(container_id.clone(), host);
        }
        Ok((container_id, image_id))
    }

    async fn prepare_console_host(
        &self,
        options: &DockerTemplateOptions,
    ) -> Result<Option<DockerConsoleHost>, RuntimeError> {
        let console_type = options.console_type.as_str();
        if console_type == "vnc" {
            return self
                .start_vnc_console(&options.console_resolution)
                .await
                .map(Some);
        }
        if console_type == "http" || console_type == "https" {
            let port = available_tcp_port().await?;
            return Ok(Some(DockerConsoleHost {
                console_type: console_type.to_owned(),
                console_port: port,
                http_port: options.console_http_port.unwrap_or(80),
                http_path: options
                    .console_http_path
                    .clone()
                    .unwrap_or_else(|| "/".to_owned()),
                resolution: options.console_resolution.clone(),
                display: None,
                vnc_processes: Vec::new(),
                http_task: None,
            }));
        }
        Ok(None)
    }

    async fn start_vnc_console(&self, resolution: &str) -> Result<DockerConsoleHost, RuntimeError> {
        let xvfb = runtime_program("NETWORK_SIMULATOR_XVFB", "Xvfb");
        let x11vnc = runtime_program("NETWORK_SIMULATOR_X11VNC", "x11vnc");
        if let Some(error) = docker_vnc_tools_error(xvfb.is_some(), x11vnc.is_some()) {
            return Err(error);
        }
        let (Some(xvfb), Some(x11vnc)) = (xvfb, x11vnc) else {
            unreachable!("Docker VNC tools were validated above")
        };
        let display = (10..=999)
            .find(|display| !Path::new(&format!("/tmp/.X11-unix/X{display}")).exists())
            .ok_or_else(|| {
                RuntimeError::Unavailable(
                    "No free X11 display is available for Docker VNC".to_owned(),
                )
            })?;
        let console_port = available_tcp_port().await?;
        let screen = format!("{resolution}x16");
        let mut xvfb_process = Command::new(&xvfb)
            .args(["-screen", "0", &screen, "-nolisten", "tcp", "-ac"])
            .arg(format!(":{display}"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                RuntimeError::Unavailable(format!(
                    "Docker VNC console is unsupported: could not start Xvfb '{}': {error}",
                    xvfb.display()
                ))
            })?;
        let socket_path = PathBuf::from(format!("/tmp/.X11-unix/X{display}"));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !socket_path.exists() {
            if let Some(status) = xvfb_process.try_wait().map_err(|error| {
                RuntimeError::Unavailable(format!("Could not inspect Xvfb startup: {error}"))
            })? {
                return Err(RuntimeError::Failed(format!(
                    "Xvfb exited during Docker VNC startup ({status})"
                )));
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = xvfb_process.start_kill();
                return Err(RuntimeError::Unavailable(
                    "Docker VNC console is unsupported: Xvfb did not create its display socket"
                        .to_owned(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let display_name = format!(":{display}");
        let console_port_text = console_port.to_string();
        let mut x11vnc_process = Command::new(&x11vnc)
            .args([
                "-display",
                display_name.as_str(),
                "-rfbport",
                console_port_text.as_str(),
                "-localhost",
                "-forever",
                "-shared",
                "-nopw",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                RuntimeError::Unavailable(format!(
                    "Docker VNC console is unsupported: could not start x11vnc '{}': {error}",
                    x11vnc.display()
                ))
            })?;
        let address = std::net::SocketAddr::from(([127, 0, 0, 1], console_port));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if TcpStream::connect(address).await.is_ok() {
                break;
            }
            if let Some(status) = x11vnc_process.try_wait().map_err(|error| {
                RuntimeError::Unavailable(format!("Could not inspect x11vnc startup: {error}"))
            })? {
                let _ = xvfb_process.start_kill();
                return Err(RuntimeError::Failed(format!(
                    "x11vnc exited during Docker VNC startup ({status})"
                )));
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = x11vnc_process.start_kill();
                let _ = xvfb_process.start_kill();
                return Err(RuntimeError::Unavailable(
                    "Docker VNC console is unsupported: x11vnc did not open its TCP port"
                        .to_owned(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Ok(DockerConsoleHost {
            console_type: "vnc".to_owned(),
            console_port,
            http_port: 0,
            http_path: "/".to_owned(),
            resolution: resolution.to_owned(),
            display: Some(display),
            vnc_processes: vec![xvfb_process, x11vnc_process],
            http_task: None,
        })
    }

    pub async fn reconfigure_container(
        &self,
        project_id: &str,
        node_id: &str,
        old_container_id: &str,
        image: &str,
        name: &str,
        start_command: Option<&str>,
        memory_mb: Option<u64>,
        cpus: Option<f64>,
        options: &DockerTemplateOptions,
    ) -> Result<(String, String), RuntimeError> {
        self.ensure_managed_container(old_container_id)?;
        let nonce = uuid::Uuid::new_v4().simple().to_string();
        let replacement_name = format!("network-simulator-replacement-{nonce}");
        let backup_name = format!("network-simulator-backup-{nonce}");
        let canonical_name = format!("network-simulator-{node_id}");
        let (replacement_id, image_id) = self
            .create_container_named(
                project_id,
                node_id,
                image,
                name,
                start_command,
                memory_mb,
                cpus,
                options,
                &replacement_name,
            )
            .await?;

        if let Err(error) = self
            .command(["rename", old_container_id, &backup_name], DOCKER_TIMEOUT)
            .await
        {
            let _ = self.remove_container(&replacement_id).await;
            return Err(error);
        }
        if let Err(error) = self
            .command(["rename", &replacement_id, &canonical_name], DOCKER_TIMEOUT)
            .await
        {
            let rollback = self
                .command(["rename", &backup_name, &canonical_name], DOCKER_TIMEOUT)
                .await;
            let _ = self.remove_container(&replacement_id).await;
            return Err(match rollback {
                Ok(_) => error,
                Err(rollback_error) => RuntimeError::Failed(format!(
                    "Docker could not install the replacement container ({}) or restore the previous name ({}); the previous container remains under '{}'",
                    error.message(),
                    rollback_error.message(),
                    backup_name
                )),
            });
        }
        self.stop_console_host(old_container_id).await;
        if let Err(error) = self.remove_container_unchecked(&backup_name).await {
            tracing::warn!(error = %error.message(), %backup_name, %node_id, "old Docker container remained after successful reconfiguration");
        } else {
            self.forget_managed_container(old_container_id);
        }
        Ok((replacement_id, image_id))
    }

    pub async fn pull_image(&self, image: &str) -> Result<String, RuntimeError> {
        validate_docker_image_reference(image)?;
        self.command(["pull", image], Duration::from_secs(30 * 60))
            .await?;
        self.image_id(image).await
    }

    pub async fn remove_node_resources(
        &self,
        node_id: &str,
        container_id: Option<&str>,
    ) -> Result<(), RuntimeError> {
        if let Some(container_id) = container_id {
            self.remove_container(container_id).await?;
        } else {
            self.stop_console_host(node_id).await;
        }
        self.remove_network(&node_network_name(node_id)).await
    }

    pub async fn remove_container(&self, container_id: &str) -> Result<(), RuntimeError> {
        self.ensure_managed_container(container_id)?;
        let result = self.remove_container_unchecked(container_id).await;
        if result.is_ok() {
            self.forget_managed_container(container_id);
        }
        result
    }

    async fn remove_container_unchecked(&self, container_id: &str) -> Result<(), RuntimeError> {
        let result = match self
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
        };
        if result.is_ok() {
            self.stop_console_host(container_id).await;
        }
        result
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
        let status = self.container_status(container_id).await?;
        if status == "started" {
            self.start_http_console(container_id).await?;
        } else {
            self.stop_http_console(container_id).await;
        }
        Ok(status)
    }

    pub async fn console_port(&self, container_id: &str) -> Option<u16> {
        self.console_hosts
            .lock()
            .await
            .get(container_id)
            .map(|host| host.console_port)
    }

    pub fn attach_aux_process(&self, container_id: &str) -> Result<Child, RuntimeError> {
        self.ensure_managed_container(container_id)?;
        let command = docker_aux_exec_command(&self.binary, container_id)?;
        let mut process = Command::new("script");
        process
            .args(["-qfc", command.as_str(), "/dev/null"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        process.spawn().map_err(|error| {
            RuntimeError::Unavailable(format!(
                "Docker auxiliary console is unsupported: could not start its PTY with script: {error}"
            ))
        })
    }

    async fn start_http_console(&self, container_id: &str) -> Result<(), RuntimeError> {
        let mut hosts = self.console_hosts.lock().await;
        let Some(host) = hosts.get_mut(container_id) else {
            return Ok(());
        };
        if !matches!(host.console_type.as_str(), "http" | "https") || host.http_task.is_some() {
            return Ok(());
        }
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, host.console_port))
            .await
            .map_err(|error| {
                RuntimeError::Unavailable(format!(
                    "Could not start Docker {} console on port {}: {error}",
                    host.console_type, host.console_port
                ))
            })?;
        let docker = self.binary.clone();
        let container_id = container_id.to_owned();
        let target_port = host.http_port;
        let console_port = host.console_port;
        let web_path = host.http_path.clone();
        let resolution = host.resolution.clone();
        host.http_task = Some(tokio::spawn(async move {
            tracing::debug!(%container_id, %web_path, %resolution, "starting Docker web console proxy");
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let docker = docker.clone();
                let container_id = container_id.clone();
                tokio::spawn(async move {
                    if let Err(error) =
                        proxy_docker_http(stream, &docker, &container_id, target_port, console_port)
                            .await
                    {
                        tracing::debug!(error = %error.message(), %container_id, "Docker web console proxy disconnected");
                    }
                });
            }
        }));
        Ok(())
    }

    async fn stop_http_console(&self, container_id: &str) {
        if let Some(host) = self.console_hosts.lock().await.get_mut(container_id)
            && let Some(task) = host.http_task.take()
        {
            task.abort();
        }
    }

    async fn stop_console_host(&self, container_id: &str) {
        let Some(mut host) = self.console_hosts.lock().await.remove(container_id) else {
            return;
        };
        if let Some(task) = host.http_task.take() {
            task.abort();
        }
        for mut process in host.vnc_processes.drain(..).rev() {
            if process.try_wait().ok().flatten().is_none() {
                let _ = process.start_kill();
                let _ = timeout(Duration::from_secs(2), process.wait()).await;
            }
        }
    }

    pub async fn container_status(&self, container_id: &str) -> Result<String, RuntimeError> {
        self.ensure_managed_container(container_id)?;
        let status = self
            .command(
                ["inspect", "--format", "{{.State.Status}}", container_id],
                DOCKER_TIMEOUT,
            )
            .await?;
        Ok(normalize_docker_state(status.trim()))
    }

    pub async fn container_pid(&self, container_id: &str) -> Result<u32, RuntimeError> {
        self.ensure_managed_container(container_id)?;
        let output = self
            .command(
                ["inspect", "--format", "{{.State.Pid}}", container_id],
                DOCKER_TIMEOUT,
            )
            .await?;
        output.trim().parse::<u32>().map_err(|_| {
            RuntimeError::Failed(format!(
                "Docker returned an invalid network namespace PID for container {container_id}"
            ))
        })
    }

    /// Resize the TTY attached to a running Docker container (height, width).
    /// The console WebSocket route should call this when it receives a terminal
    /// resize control frame.
    pub async fn resize_tty(
        &self,
        container_id: &str,
        rows: u16,
        columns: u16,
    ) -> Result<(), RuntimeError> {
        if rows == 0 || columns == 0 {
            return Err(RuntimeError::Failed(
                "Docker console rows and columns must be greater than zero".to_owned(),
            ));
        }
        self.ensure_managed_container(container_id)?;
        let mut socket = UnixStream::connect(&self.socket).await.map_err(|error| {
            RuntimeError::Unavailable(format!(
                "Could not connect to Docker Engine socket '{}': {error}",
                self.socket.display()
            ))
        })?;
        let request = format!(
            "POST /containers/{container_id}/resize?h={rows}&w={columns} HTTP/1.1\r\nHost: docker\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        socket
            .write_all(request.as_bytes())
            .await
            .map_err(|error| {
                RuntimeError::Unavailable(format!(
                    "Could not send Docker TTY resize request: {error}"
                ))
            })?;
        let mut response = Vec::new();
        timeout(DOCKER_TIMEOUT, socket.read_to_end(&mut response))
            .await
            .map_err(|_| {
                RuntimeError::Unavailable("Docker TTY resize request timed out".to_owned())
            })?
            .map_err(|error| {
                RuntimeError::Unavailable(format!(
                    "Could not read Docker TTY resize response: {error}"
                ))
            })?;
        let status_line = String::from_utf8_lossy(&response)
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned();
        if !status_line
            .split_whitespace()
            .nth(1)
            .is_some_and(|status| matches!(status, "200" | "201" | "204"))
        {
            return Err(RuntimeError::Failed(format!(
                "Docker TTY resize failed: {status_line}"
            )));
        }
        Ok(())
    }

    pub fn attach_process(
        &self,
        container_id: &str,
    ) -> Result<tokio::process::Child, RuntimeError> {
        self.ensure_managed_container(container_id)?;
        let docker_binary = self.binary.to_string_lossy();
        let docker_binary = shlex::try_quote(&docker_binary).map_err(|_| {
            RuntimeError::Failed("Docker CLI path cannot be quoted safely".to_owned())
        })?;
        let attach_command = format!(
            "{docker_binary} attach --sig-proxy=false --detach-keys=ctrl-p,ctrl-q {container_id}"
        );
        let mut command = Command::new("script");
        command
            .args(["-qfc", attach_command.as_str(), "/dev/null"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command.spawn().map_err(|error| {
            RuntimeError::Unavailable(format!(
                "Could not start the Docker console terminal for container '{container_id}': {error}"
            ))
        })
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

fn parse_environment(environment: &str) -> Result<Vec<String>, RuntimeError> {
    let mut parsed = Vec::new();
    for line in environment
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let (key, value) = line.split_once('=').ok_or_else(|| {
            RuntimeError::Failed(format!(
                "Docker environment entry '{line}' must use KEY=VALUE"
            ))
        })?;
        if key.is_empty()
            || !key.bytes().enumerate().all(|(index, byte)| {
                if index == 0 {
                    byte.is_ascii_alphabetic() || byte == b'_'
                } else {
                    byte.is_ascii_alphanumeric() || byte == b'_'
                }
            })
            || line.contains('\0')
        {
            return Err(RuntimeError::Failed(format!(
                "Docker environment variable name '{key}' is invalid"
            )));
        }
        parsed.push(format!("{key}={value}"));
    }
    Ok(parsed)
}

fn parse_extra_hosts(extra_hosts: &str) -> Result<Vec<String>, RuntimeError> {
    let mut parsed = Vec::new();
    for line in extra_hosts
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let (host, address) = line.split_once(':').ok_or_else(|| {
            RuntimeError::Failed(format!("Docker extra host '{line}' must use host:address"))
        })?;
        let host_valid = !host.is_empty()
            && host.len() <= 253
            && host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'));
        let address_valid = !address.is_empty()
            && address.len() <= 128
            && address.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b':' | b'-' | b'_')
            });
        if !host_valid || !address_valid {
            return Err(RuntimeError::Failed(format!(
                "Docker extra host '{line}' contains an invalid host or address"
            )));
        }
        parsed.push(line.to_owned());
    }
    Ok(parsed)
}

fn validate_container_volume(path: &str) -> Result<PathBuf, RuntimeError> {
    let path = Path::new(path);
    if !path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
        || path.to_string_lossy().len() > 512
    {
        return Err(RuntimeError::Failed(
            "Docker persistent volumes must be safe absolute container paths".to_owned(),
        ));
    }
    Ok(path
        .strip_prefix("/")
        .map_err(|_| RuntimeError::Failed("Invalid Docker container path".to_owned()))?
        .to_path_buf())
}

fn default_docker_console_type() -> String {
    "telnet".to_owned()
}

fn docker_socket_path() -> PathBuf {
    std::env::var_os("DOCKER_HOST")
        .and_then(|host| host.into_string().ok())
        .and_then(|host| host.strip_prefix("unix://").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/var/run/docker.sock"))
}

fn default_docker_console_resolution() -> String {
    "1024x768".to_owned()
}

fn default_docker_aux_type() -> String {
    "none".to_owned()
}

async fn available_tcp_port() -> Result<u16, RuntimeError> {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not allocate a TCP port: {error}"))
        })?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not inspect an allocated TCP port: {error}"))
        })
}

fn runtime_program(variable: &str, name: &str) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(variable).map(PathBuf::from) {
        return path.is_file().then_some(path);
    }
    if name.contains('/') {
        let path = PathBuf::from(name);
        return path.is_file().then_some(path);
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|directory| directory.join(name))
        .find(|path| path.is_file())
}

fn docker_aux_exec_command(binary: &Path, container_id: &str) -> Result<String, RuntimeError> {
    let binary = binary.to_string_lossy();
    let docker = shlex::try_quote(&binary)
        .map_err(|_| RuntimeError::Failed("Docker CLI path cannot be quoted safely".to_owned()))?;
    Ok(format!(
        "{docker} exec -i -t {container_id} /gns3/bin/busybox sh -c 'while true; do TERM=vt100 /gns3/bin/busybox sh; done'"
    ))
}

async fn proxy_docker_http(
    client: TcpStream,
    docker: &Path,
    container_id: &str,
    target_port: u16,
    console_port: u16,
) -> Result<(), RuntimeError> {
    let target_port_text = target_port.to_string();
    let mut child = Command::new(docker)
        .args([
            "exec",
            "-i",
            container_id,
            "/gns3/bin/busybox",
            "nc",
            "127.0.0.1",
            target_port_text.as_str(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| {
            RuntimeError::Unavailable(format!(
                "Could not start the Docker web-console bridge: {error}"
            ))
        })?;
    let (Some(mut stdin), Some(mut stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return Err(RuntimeError::Failed(
            "Docker web-console bridge did not provide input and output streams".to_owned(),
        ));
    };
    let client_host = client
        .peer_addr()
        .map(|address| address.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_owned());
    let (mut client_reader, mut client_writer) = tokio::io::split(client);
    let input = tokio::io::copy(&mut client_reader, &mut stdin);
    let output = async {
        let host_replace = format!("://{client_host}").into_bytes();
        let port_replace = format!(":{target_port}").into_bytes();
        let public_port = format!(":{console_port}").into_bytes();
        let mut buffer = [0u8; 4096];
        loop {
            let length = stdout.read(&mut buffer).await?;
            if length == 0 {
                return Ok::<(), std::io::Error>(());
            }
            let mut chunk = buffer[..length].to_vec();
            replace_bytes(&mut chunk, b"://127.0.0.1", &host_replace);
            replace_bytes(&mut chunk, &port_replace, &public_port);
            client_writer.write_all(&chunk).await?;
        }
    };
    tokio::pin!(input, output);
    let bridge_result = tokio::select! {
        result = &mut output => result,
        input_result = &mut input => match input_result {
            Ok(_) => output.await,
            Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => output.await,
            Err(error) => return Err(RuntimeError::Unavailable(format!("Docker web-console request stream failed: {error}"))),
        },
    };
    bridge_result.map_err(|error| {
        RuntimeError::Unavailable(format!(
            "Docker web-console response stream failed: {error}"
        ))
    })?;
    let _ = child.kill().await;
    Ok(())
}

fn replace_bytes(contents: &mut Vec<u8>, from: &[u8], to: &[u8]) {
    if from.is_empty() {
        return;
    }
    let mut output = Vec::with_capacity(contents.len());
    let mut index = 0;
    while index < contents.len() {
        if contents[index..].starts_with(from) {
            output.extend_from_slice(to);
            index += from.len();
        } else {
            output.push(contents[index]);
            index += 1;
        }
    }
    *contents = output;
}

fn docker_vnc_tools_error(has_xvfb: bool, has_x11vnc: bool) -> Option<RuntimeError> {
    (!has_xvfb || !has_x11vnc).then(|| {
        RuntimeError::Unavailable(
            "Docker VNC console is unsupported: Xvfb and x11vnc must be installed".to_owned(),
        )
    })
}

fn validate_docker_runtime_options(options: &DockerTemplateOptions) -> Result<(), RuntimeError> {
    if !matches!(
        options.console_type.as_str(),
        "none" | "telnet" | "vnc" | "http" | "https"
    ) {
        return Err(RuntimeError::Failed(
            "Docker console_type must be none, telnet, vnc, http, or https".to_owned(),
        ));
    }
    if !matches!(options.aux_type.as_str(), "none" | "telnet" | "ssh") {
        return Err(RuntimeError::Failed(
            "Docker aux_type must be none, telnet, or ssh".to_owned(),
        ));
    }
    let Some((width, height)) = options.console_resolution.split_once('x') else {
        return Err(RuntimeError::Failed(
            "Docker console_resolution must use WIDTHxHEIGHT".to_owned(),
        ));
    };
    if width.is_empty()
        || height.is_empty()
        || !width.bytes().all(|byte| byte.is_ascii_digit())
        || !height.bytes().all(|byte| byte.is_ascii_digit())
        || width
            .parse::<u16>()
            .ok()
            .filter(|value| *value > 0)
            .is_none()
        || height
            .parse::<u16>()
            .ok()
            .filter(|value| *value > 0)
            .is_none()
    {
        return Err(RuntimeError::Failed(
            "Docker console_resolution must use positive numeric WIDTHxHEIGHT values".to_owned(),
        ));
    }
    if options.console_http_port == Some(0) {
        return Err(RuntimeError::Failed(
            "Docker console_http_port must be between 1 and 65535".to_owned(),
        ));
    }
    if options.console_http_path.as_deref().is_some_and(|path| {
        !path.starts_with('/') || path.bytes().any(|byte| byte.is_ascii_control())
    }) {
        return Err(RuntimeError::Failed(
            "Docker console_http_path must be an absolute URL path".to_owned(),
        ));
    }
    validate_docker_extra_configs(
        &options.extra_configs,
        options.startup_config_content.as_deref(),
    )?;
    let mut adapter_numbers = std::collections::HashSet::new();
    for adapter in &options.custom_adapters {
        if adapter.adapter_number > 99 || !adapter_numbers.insert(adapter.adapter_number) {
            return Err(RuntimeError::Failed(
                "Docker custom adapter numbers must be unique values from 0 to 99".to_owned(),
            ));
        }
        if adapter
            .mac_address
            .as_deref()
            .is_some_and(|address| !valid_mac_address(address))
        {
            return Err(RuntimeError::Failed(format!(
                "Docker custom adapter {} has an invalid MAC address",
                adapter.adapter_number
            )));
        }
    }
    Ok(())
}

pub fn validate_docker_extra_configs(
    configs: &[DockerExtraConfig],
    startup_config_content: Option<&str>,
) -> Result<(), RuntimeError> {
    if configs.len() > 64 {
        return Err(RuntimeError::Failed(
            "Docker extra_configs may contain at most 64 files".to_owned(),
        ));
    }
    let mut targets = std::collections::HashSet::<PathBuf>::new();
    let mut total_bytes = 0_usize;
    for config in configs {
        let target = validate_extra_config_target(&config.target)?;
        if targets
            .iter()
            .any(|existing| existing.starts_with(&target) || target.starts_with(existing))
        {
            return Err(RuntimeError::Failed(format!(
                "Docker extra_configs contains overlapping target '{}'",
                config.target
            )));
        }
        targets.insert(target);
        if config.content.len() > 1024 * 1024 {
            return Err(RuntimeError::Failed(
                "Docker extra config content must be 1 MiB or smaller".to_owned(),
            ));
        }
        total_bytes = total_bytes.saturating_add(config.content.len());
    }
    if let Some(content) = startup_config_content {
        if content.len() > 1024 * 1024 {
            return Err(RuntimeError::Failed(
                "Docker startup_config_content must be 1 MiB or smaller".to_owned(),
            ));
        }
        if !content.is_empty() {
            let startup_target = PathBuf::from("startup-config");
            if targets.iter().any(|target| {
                target.starts_with(&startup_target) || startup_target.starts_with(target)
            }) {
                return Err(RuntimeError::Failed(
                    "Docker startup_config_content conflicts with extra_configs target /startup-config"
                        .to_owned(),
                ));
            }
            targets.insert(startup_target);
        }
        total_bytes = total_bytes.saturating_add(content.len());
    }
    if total_bytes > 4 * 1024 * 1024 {
        return Err(RuntimeError::Failed(
            "Docker injected config content must total 4 MiB or less".to_owned(),
        ));
    }
    Ok(())
}

fn validate_extra_config_target(target: &str) -> Result<PathBuf, RuntimeError> {
    if !target.starts_with('/')
        || target.ends_with('/')
        || target.contains('\\')
        || target.bytes().any(|byte| byte.is_ascii_control())
        || target.len() > 1024
    {
        return Err(RuntimeError::Failed(format!(
            "Docker extra config target '{}' must be an absolute file path",
            target
        )));
    }
    let mut relative = PathBuf::new();
    for component in Path::new(target).components() {
        match component {
            std::path::Component::RootDir => {}
            std::path::Component::Normal(part) => relative.push(part),
            _ => {
                return Err(RuntimeError::Failed(format!(
                    "Docker extra config target '{}' contains an unsafe path component",
                    target
                )));
            }
        }
    }
    if relative.as_os_str().is_empty() {
        return Err(RuntimeError::Failed(
            "Docker extra config target must name a file".to_owned(),
        ));
    }
    Ok(relative)
}

fn valid_container_id(container_id: &str) -> bool {
    (12..=64).contains(&container_id.len())
        && container_id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_docker_topology_id(value: &str, kind: &str) -> Result<String, RuntimeError> {
    Uuid::parse_str(value)
        .map(|id| id.to_string())
        .map_err(|_| RuntimeError::Failed(format!("Docker {kind} ID must be a valid UUID")))
}

fn valid_mac_address(value: &str) -> bool {
    let octets = value.split(':').collect::<Vec<_>>();
    octets.len() == 6
        && octets
            .iter()
            .all(|octet| octet.len() == 2 && octet.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn validate_docker_image_reference(image: &str) -> Result<(), RuntimeError> {
    if image.trim().is_empty()
        || image.len() > 512
        || image.bytes().any(|byte| {
            byte.is_ascii_whitespace()
                || byte.is_ascii_control()
                || matches!(byte, b'\\' | b'`' | b'\'' | b'"')
        })
    {
        return Err(RuntimeError::Failed(
            "Docker image reference is invalid".to_owned(),
        ));
    }
    Ok(())
}

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
mod tests {
    use super::{
        DockerCustomAdapter, DockerExtraConfig, DockerRuntime, DockerTemplateOptions,
        docker_aux_exec_command, docker_hostname, docker_vnc_tools_error, node_network_name,
        normalize_docker_state, parse_environment, parse_extra_hosts, parse_start_command,
        proxy_docker_http, validate_container_volume, validate_docker_extra_configs,
        validate_docker_runtime_options,
    };
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
        time::Duration,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpStream, UnixListener},
    };

    fn docker_test_double(directory: &std::path::Path) -> (PathBuf, PathBuf) {
        let binary = directory.join("docker-test-double");
        let log = directory.join("docker-args.log");
        fs::write(
            &binary,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\ncase \"$1\" in image) echo sha256:0123456789abcdef;; create) echo 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef;; esac\n",
                log.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
        (binary, log)
    }

    #[test]
    fn maps_docker_states_to_controller_states() {
        assert_eq!(normalize_docker_state("running"), "started");
        assert_eq!(normalize_docker_state("paused"), "suspended");
        assert_eq!(normalize_docker_state("exited"), "stopped");
    }

    #[test]
    fn derives_stable_safe_names_for_node_networks() {
        assert_eq!(node_network_name("ab12-cd34"), "ns-node-ab12cd34");
    }

    #[test]
    fn docker_hostnames_are_valid_and_stable_for_node_names() {
        assert_eq!(
            docker_hostname("Alpine Router 1", "01234567"),
            "alpine-router-1"
        );
        assert_eq!(docker_hostname("---", "01234567"), "node-01234567");
    }

    #[test]
    fn parses_docker_command_without_running_a_host_shell() {
        assert_eq!(
            parse_start_command("/bin/sh -c 'sleep 60'").unwrap(),
            ["/bin/sh", "-c", "sleep 60"]
        );
        assert!(parse_start_command("/bin/sh -c 'sleep 60").is_err());
    }

    #[test]
    fn validates_docker_template_environment_hosts_and_persistent_paths() {
        assert_eq!(
            parse_environment("_LAB=router\nMODE=ospf").unwrap(),
            ["_LAB=router", "MODE=ospf"]
        );
        assert!(parse_environment("1INVALID=value").is_err());
        assert_eq!(
            parse_extra_hosts("gateway:192.0.2.1").unwrap(),
            ["gateway:192.0.2.1"]
        );
        assert!(parse_extra_hosts("bad host:192.0.2.1").is_err());
        assert!(validate_container_volume("/var/lib/router").is_ok());
        assert!(validate_container_volume("/var/../etc").is_err());
    }

    #[test]
    fn validates_docker_console_and_custom_adapter_schema() {
        let mut options = DockerTemplateOptions::default();
        options.console_type = "https".to_owned();
        options.console_resolution = "1600x900".to_owned();
        options.aux_type = "ssh".to_owned();
        options.custom_adapters.push(DockerCustomAdapter {
            adapter_number: 2,
            mac_address: Some("02:00:00:00:00:02".to_owned()),
            ..DockerCustomAdapter::default()
        });
        assert!(validate_docker_runtime_options(&options).is_ok());
        options.console_resolution = "wide".to_owned();
        assert!(validate_docker_runtime_options(&options).is_err());
    }

    #[test]
    fn validates_docker_injected_config_paths_content_and_conflicts() {
        let config = DockerExtraConfig {
            target: "/etc/router/router.conf".to_owned(),
            content: "hostname lab".to_owned(),
        };
        assert!(validate_docker_extra_configs(&[config], None).is_ok());
        assert!(
            validate_docker_extra_configs(
                &[DockerExtraConfig {
                    target: "/etc/../outside".to_owned(),
                    content: "bad".to_owned(),
                }],
                None
            )
            .is_err()
        );
        assert!(
            validate_docker_extra_configs(
                &[DockerExtraConfig {
                    target: "/startup-config".to_owned(),
                    content: "file".to_owned(),
                }],
                Some("inline")
            )
            .is_err()
        );
        assert!(
            validate_docker_extra_configs(
                &[
                    DockerExtraConfig {
                        target: "/etc/router".to_owned(),
                        content: "directory mount".to_owned(),
                    },
                    DockerExtraConfig {
                        target: "/etc/router/router.conf".to_owned(),
                        content: "nested file".to_owned(),
                    },
                ],
                None
            )
            .is_err()
        );
        assert!(
            validate_docker_extra_configs(
                &[DockerExtraConfig {
                    target: "/startup-config/nested.conf".to_owned(),
                    content: "nested file".to_owned(),
                }],
                Some("inline")
            )
            .is_err()
        );
    }

    #[test]
    fn imported_docker_container_ids_require_a_runtime_state_ownership_marker() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = DockerRuntime::with_paths("docker", directory.path().join("state"));
        let container_id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(runtime.ensure_managed_container(container_id).is_err());
        runtime
            .mark_container_managed(
                "00000000-0000-4000-8000-000000000001",
                "00000000-0000-4000-8000-000000000002",
                container_id,
            )
            .unwrap();
        runtime.ensure_managed_container(container_id).unwrap();
        runtime.forget_managed_container(container_id);
        runtime.ensure_managed_container(container_id).unwrap();
    }

    #[tokio::test]
    async fn docker_none_console_type_is_valid_and_allocates_no_console_host() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = DockerRuntime::with_paths("docker", directory.path().join("state"));
        let options = DockerTemplateOptions {
            console_type: "none".to_owned(),
            ..DockerTemplateOptions::default()
        };
        validate_docker_runtime_options(&options).unwrap();
        assert!(
            runtime
                .prepare_console_host(&options)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn prepares_http_and_https_console_endpoints_with_configured_web_port_and_path() {
        let directory = tempfile::tempdir().unwrap();
        let (binary, _) = docker_test_double(directory.path());
        let runtime = DockerRuntime::with_binary(binary);
        for console_type in ["http", "https"] {
            let options = DockerTemplateOptions {
                console_type: console_type.to_owned(),
                console_resolution: "1600x900".to_owned(),
                console_http_port: Some(8443),
                console_http_path: Some("/admin/".to_owned()),
                ..DockerTemplateOptions::default()
            };
            let host = runtime
                .prepare_console_host(&options)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(host.console_type, console_type);
            assert_eq!(host.http_port, 8443);
            assert_eq!(host.http_path, "/admin/");
            assert_eq!(host.resolution, "1600x900");
            assert_ne!(host.console_port, 0);
        }
    }

    #[test]
    fn docker_vnc_reports_missing_host_tools_as_unsupported() {
        let error = docker_vnc_tools_error(false, false).unwrap();
        assert!(error.message().contains("unsupported"));
        assert!(error.message().contains("Xvfb and x11vnc"));
        assert!(docker_vnc_tools_error(true, true).is_none());
    }

    #[test]
    fn builds_auxiliary_console_pty_command_with_docker_exec() {
        let command = docker_aux_exec_command(
            Path::new("/usr/bin/docker"),
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        assert!(command.contains("docker exec -i -t 0123456789abcdef"));
        assert!(command.contains("/gns3/bin/busybox sh"));
    }

    #[tokio::test]
    async fn docker_http_console_rewrites_container_host_port_in_responses() {
        let directory = tempfile::tempdir().unwrap();
        let docker = directory.path().join("docker-http-test-double");
        fs::write(
            &docker,
            "#!/bin/sh\ncr=$(printf '\\r')\nwhile IFS= read -r line; do [ \"$line\" = \"$cr\" ] && break; done\nprintf 'Location: http://127.0.0.1:8080/ui\\r\\n\\r\\n'\n",
        )
        .unwrap();
        fs::set_permissions(&docker, fs::Permissions::from_mode(0o755)).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            proxy_docker_http(stream, &docker, "0123456789abcdef", 8080, 9999)
                .await
                .unwrap();
        });
        let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
        client.write_all(b"GET /ui HTTP/1.1\r\n\r\n").await.unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        server.await.unwrap();
        let response = String::from_utf8(response).unwrap();
        assert!(response.contains("http://127.0.0.1:9999/ui"));
    }

    #[tokio::test]
    #[ignore = "requires a running Docker Engine and alpine:latest"]
    async fn docker_live_http_console_bridges_to_container_web_port() {
        let image = std::env::var("NETWORK_SIMULATOR_TEST_DOCKER_IMAGE")
            .unwrap_or_else(|_| "alpine:latest".to_owned());
        let binary = std::env::var_os("NETWORK_SIMULATOR_TEST_DOCKER")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("docker"));
        let directory = tempfile::tempdir().unwrap();
        let runtime = DockerRuntime::with_paths_and_socket(
            binary,
            directory.path().join("state"),
            std::env::var_os("DOCKER_HOST")
                .and_then(|host| host.into_string().ok())
                .and_then(|host| host.strip_prefix("unix://").map(PathBuf::from))
                .unwrap_or_else(|| PathBuf::from("/var/run/docker.sock")),
        );
        let node_id = uuid::Uuid::new_v4().to_string();
        let project_id = uuid::Uuid::new_v4().to_string();
        let command = "/bin/sh -c \"mkdir -p /gns3/bin; ln -sf /bin/busybox /gns3/bin/busybox; while true; do printf 'HTTP/1.1 200 OK\\r\\nConnection: close\\r\\nContent-Length: 26\\r\\n\\r\\nhttp://127.0.0.1:8080/path' | /bin/busybox nc -l -p 8080; done\"";
        let options = DockerTemplateOptions {
            console_type: "http".to_owned(),
            console_http_port: Some(8080),
            console_http_path: Some("/path".to_owned()),
            ..DockerTemplateOptions::default()
        };
        let (container_id, _) = runtime
            .create_container_with_options(
                &project_id,
                &node_id,
                &image,
                "Runtime HTTP console test",
                Some(command),
                None,
                None,
                &options,
            )
            .await
            .expect("Docker creates the live console container");
        let result = async {
            runtime
                .lifecycle(&container_id, crate::runtime::NodeAction::Start)
                .await
                .map_err(|error| error.message().to_owned())?;
            let port = runtime
                .console_port(&container_id)
                .await
                .ok_or_else(|| "HTTP console did not allocate a host port".to_owned())?;
            let mut client = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
                .await
                .map_err(|error| error.to_string())?;
            client
                .write_all(b"GET /path HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .await
                .map_err(|error| error.to_string())?;
            let response = tokio::time::timeout(Duration::from_secs(5), async {
                let mut response = Vec::new();
                let mut buffer = [0u8; 1024];
                loop {
                    let length = client
                        .read(&mut buffer)
                        .await
                        .map_err(|error| error.to_string())?;
                    if length == 0 {
                        return Err(format!(
                            "HTTP console closed before returning a response body: {}",
                            String::from_utf8_lossy(&response)
                        ));
                    }
                    response.extend_from_slice(&buffer[..length]);
                    if response
                        .windows(b"/path".len())
                        .any(|window| window == b"/path")
                    {
                        return Ok::<_, String>(response);
                    }
                }
            })
            .await
            .map_err(|_| "Docker HTTP console response timed out".to_owned())??;
            let response = String::from_utf8_lossy(&response);
            if !response.contains(&format!("http://127.0.0.1:{port}/path"))
                || response.contains("http://127.0.0.1:8080/path")
            {
                return Err(format!(
                    "HTTP console returned an untranslated response: {response}"
                ));
            }
            let mut auxiliary = runtime
                .attach_aux_process(&container_id)
                .map_err(|error| error.message().to_owned())?;
            let mut auxiliary_input = auxiliary
                .stdin
                .take()
                .ok_or_else(|| "Docker aux process has no input stream".to_owned())?;
            let mut auxiliary_output = auxiliary
                .stdout
                .take()
                .ok_or_else(|| "Docker aux process has no output stream".to_owned())?;
            auxiliary_input
                .write_all(b"echo AUX_RUNTIME_OK\n")
                .await
                .map_err(|error| error.to_string())?;
            let mut output = Vec::new();
            let read = async {
                let mut buffer = [0u8; 1024];
                loop {
                    let length = auxiliary_output
                        .read(&mut buffer)
                        .await
                        .map_err(|error| error.to_string())?;
                    if length == 0 {
                        return Err("Docker aux process closed before responding".to_owned());
                    }
                    output.extend_from_slice(&buffer[..length]);
                    if output
                        .windows(b"AUX_RUNTIME_OK".len())
                        .any(|window| window == b"AUX_RUNTIME_OK")
                    {
                        return Ok(());
                    }
                }
            };
            tokio::time::timeout(Duration::from_secs(5), read)
                .await
                .map_err(|_| "Docker auxiliary console shell timed out".to_owned())??;
            let _ = auxiliary.kill().await;
            let _ = auxiliary.wait().await;
            Ok::<(), String>(())
        }
        .await;
        let _ = runtime
            .remove_node_resources(&node_id, Some(&container_id))
            .await;
        if let Err(error) = result {
            panic!("Docker live console check failed: {error}");
        }
    }

    #[tokio::test]
    async fn docker_create_and_tty_resize_reach_the_runtime_cli() {
        let directory = tempfile::tempdir().unwrap();
        let (binary, log) = docker_test_double(directory.path());
        let socket_path = directory.path().join("docker.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();
        let resize_server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut chunk = [0_u8; 512];
            loop {
                let length = stream.read(&mut chunk).await.unwrap();
                request.extend_from_slice(&chunk[..length]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            stream
                .write_all(
                    b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            String::from_utf8(request).unwrap()
        });
        let runtime = DockerRuntime::with_paths_and_socket(
            binary,
            directory.path().join("state"),
            socket_path,
        );
        let options = DockerTemplateOptions {
            environment: "MODE=ospf".to_owned(),
            extra_hosts: "gateway:192.0.2.1".to_owned(),
            extra_volumes: vec!["/var/lib/router".to_owned()],
            extra_configs: vec![DockerExtraConfig {
                target: "/etc/router/router.conf".to_owned(),
                content: "router-id 192.0.2.1\n".to_owned(),
            }],
            startup_config_content: Some("hostname %h\n".to_owned()),
            ..DockerTemplateOptions::default()
        };
        let (container_id, _) = runtime
            .create_container_with_options(
                "00000000-0000-0000-0000-000000000001",
                "00000000-0000-0000-0000-000000000002",
                "router:latest",
                "Router 1",
                Some("/bin/sh -c 'sleep 60'"),
                Some(512),
                Some(1.5),
                &options,
            )
            .await
            .unwrap();
        assert_eq!(container_id.len(), 64);
        runtime.resize_tty(&container_id, 40, 120).await.unwrap();
        let calls = fs::read_to_string(log).unwrap();
        assert!(calls.contains("--env MODE=ospf"));
        assert!(calls.contains("--add-host gateway:192.0.2.1"));
        assert!(calls.contains("/var/lib/router"));
        assert!(calls.contains(":/etc/router/router.conf:ro"));
        assert!(calls.contains(":/startup-config:ro"));
        assert!(calls.contains("/bin/sh -c sleep 60"));
        let config_root = directory
            .path()
            .join("state/00000000-0000-0000-0000-000000000001/00000000-0000-0000-0000-000000000002/configs");
        assert_eq!(
            fs::read_to_string(config_root.join("etc/router/router.conf")).unwrap(),
            "router-id 192.0.2.1\n"
        );
        assert_eq!(
            fs::read_to_string(config_root.join("startup-config")).unwrap(),
            "hostname %h\n"
        );
        let resize_request = resize_server.await.unwrap();
        assert!(resize_request.starts_with(&format!(
            "POST /containers/{container_id}/resize?h=40&w=120 HTTP/1.1"
        )));
    }
}
