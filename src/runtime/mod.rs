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
