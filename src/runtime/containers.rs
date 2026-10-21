use super::*;

impl DockerRuntime {
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

    pub(super) async fn create_container_named(
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

    pub(super) async fn remove_container_unchecked(&self, container_id: &str) -> Result<(), RuntimeError> {
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

    pub(super) async fn image_id(&self, image: &str) -> Result<String, RuntimeError> {
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

    pub(super) async fn remove_network(&self, network: &str) -> Result<(), RuntimeError> {
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
}
