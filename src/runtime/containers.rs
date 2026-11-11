use super::*;

const INIT_SH: &str = include_str!("resources/init.sh");
const RUN_CMD_SH: &str = include_str!("resources/run-cmd.sh");

fn sample_interfaces(adapters: u32) -> String {
    let mut text = String::from(
        "#\n# Network configuration applied at boot by /gns3/init.sh (BusyBox ifup).\n# BusyBox only brings up \"auto\" stanzas and needs separate address/netmask lines.\n#\n",
    );
    for adapter in 0..adapters {
        text.push_str(&format!(
            "\n# Static config for eth{adapter}\n#auto eth{adapter}\n#iface eth{adapter} inet static\n#\taddress 192.168.{adapter}.2\n#\tnetmask 255.255.255.0\n#\tgateway 192.168.{adapter}.1\n\n# DHCP config for eth{adapter}\n#auto eth{adapter}\n#iface eth{adapter} inet dhcp\n"
        ));
    }
    text
}

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
        // GNS3 mounts a static BusyBox at /gns3/bin/busybox; the auxiliary
        // console and the HTTP/VNC console bridges execute it inside the guest.
        if let Some(busybox) = runtime_program("NETWORK_SIMULATOR_BUSYBOX", "busybox") {
            args.extend([
                "--volume".to_owned(),
                format!("{}:/gns3/bin/busybox:ro", busybox.display()),
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
        let shim = self.init_shim_args(&project_id, &node_id, &image_id, options)?;
        let use_shim = shim.is_some();
        if let Some((shim_args, _)) = &shim {
            args.extend(shim_args.clone());
        }
        args.push(image_id.clone());
        let user_command = match start_command.filter(|command| !command.trim().is_empty()) {
            Some(command) => Some(parse_start_command(command)?),
            None => None,
        };
        if use_shim {
            // GNS3 prepends its init script to the image entrypoint; the command
            // is the start command or, when none is given, the image's own CMD.
            let (_, image_command) = self.image_entrypoint_and_cmd(&image_id).await;
            args.extend(image_command.0);
            args.extend(user_command.unwrap_or(image_command.1));
        } else if let Some(command) = user_command {
            args.extend(command);
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
        // Images built or loaded locally have no registry to pull from; use them as they are.
        if let Err(error) = self.command(["pull", image], Duration::from_secs(30 * 60)).await {
            if self.image_id(image).await.is_err() {
                return Err(error);
            }
        }
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

    pub(super) async fn remove_container_unchecked(
        &self,
        container_id: &str,
    ) -> Result<(), RuntimeError> {
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

    /// Docker arguments that route the container through the GNS3-style init shim.
    /// Returns None when it cannot be used (no adapters, or no static BusyBox on the host).
    fn init_shim_args(
        &self,
        project_id: &str,
        node_id: &str,
        _image_id: &str,
        options: &DockerTemplateOptions,
    ) -> Result<Option<(Vec<String>, PathBuf)>, RuntimeError> {
        if options.adapters == 0 || runtime_program("NETWORK_SIMULATOR_BUSYBOX", "busybox").is_none() {
            return Ok(None);
        }
        let node_dir = self.state_dir.join(project_id).join(node_id);
        let shim_dir = node_dir.join("gns3");
        let network_dir = node_dir.join("etc").join("network");
        fs::create_dir_all(&shim_dir)
            .and_then(|()| fs::create_dir_all(&network_dir))
            .map_err(|error| RuntimeError::Failed(format!("Could not prepare Docker init shim: {error}")))?;
        for (name, content) in [("init.sh", INIT_SH), ("run-cmd.sh", RUN_CMD_SH)] {
            let path = shim_dir.join(name);
            fs::write(&path, content)
                .and_then(|()| {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                })
                .map_err(|error| RuntimeError::Failed(format!("Could not write Docker init shim: {error}")))?;
        }
        let shim_dir = shim_dir.canonicalize().unwrap_or(shim_dir);
        let network_dir = network_dir.canonicalize().unwrap_or(network_dir);
        let interfaces = network_dir.join("interfaces");
        if !interfaces.exists() {
            fs::write(&interfaces, sample_interfaces(options.adapters))
                .map_err(|error| RuntimeError::Failed(format!("Could not write network config: {error}")))?;
        }
        let expected = shim_dir.join("expected-eth");
        if !expected.exists() {
            fs::write(&expected, "").map_err(|error| RuntimeError::Failed(format!("Could not write adapter list: {error}")))?;
        }
        let max = options.adapters.saturating_sub(1);
        let args = vec![
            "--entrypoint".to_owned(),
            "/gns3/init.sh".to_owned(),
            "--volume".to_owned(),
            format!("{}:/gns3/init.sh:ro", shim_dir.join("init.sh").display()),
            "--volume".to_owned(),
            format!("{}:/gns3/run-cmd.sh:ro", shim_dir.join("run-cmd.sh").display()),
            "--volume".to_owned(),
            format!("{}:/gns3/expected-eth:ro", expected.display()),
            "--volume".to_owned(),
            format!("{}:/etc/network/interfaces", interfaces.display()),
            "--env".to_owned(),
            format!("GNS3_MAX_ETHERNET=eth{max}"),
        ];
        Ok(Some((args, interfaces)))
    }

    /// Tell the init shim which adapters have links, so it waits for exactly those NICs.
    pub fn write_expected_adapters(&self, project_id: &str, node_id: &str, adapters: &[u64]) -> Result<(), RuntimeError> {
        let project_id = validate_docker_topology_id(project_id, "project")?;
        let node_id = validate_docker_topology_id(node_id, "node")?;
        let dir = self.state_dir.join(project_id).join(node_id).join("gns3");
        if !dir.is_dir() {
            return Ok(());
        }
        let text: String = adapters.iter().map(|adapter| format!("eth{adapter}\n")).collect();
        fs::write(dir.join("expected-eth"), text).map_err(|error| RuntimeError::Failed(format!("Could not write adapter list: {error}")))
    }

    /// Persist an `/etc/network/interfaces` file for a node (GNS3's "Edit network configuration").
    pub fn write_network_config(&self, project_id: &str, node_id: &str, content: &[u8]) -> Result<(), RuntimeError> {
        let project_id = validate_docker_topology_id(project_id, "project")?;
        let node_id = validate_docker_topology_id(node_id, "node")?;
        let dir = self.state_dir.join(project_id).join(node_id).join("etc").join("network");
        fs::create_dir_all(&dir).map_err(|error| RuntimeError::Failed(format!("Could not create network config directory: {error}")))?;
        fs::write(dir.join("interfaces"), content).map_err(|error| RuntimeError::Failed(format!("Could not write network config: {error}")))
    }

    /// (image entrypoint, (entrypoint args to keep, image CMD)) -- both empty if inspection fails.
    async fn image_entrypoint_and_cmd(&self, image_id: &str) -> ((), (Vec<String>, Vec<String>)) {
        let output = self
            .command(
                ["image", "inspect", "--format", "{{json .Config.Entrypoint}}|{{json .Config.Cmd}}", image_id],
                DOCKER_TIMEOUT,
            )
            .await
            .unwrap_or_default();
        let (entrypoint, cmd) = output.trim().split_once('|').unwrap_or(("null", "null"));
        let parse = |text: &str| serde_json::from_str::<Option<Vec<String>>>(text).ok().flatten().unwrap_or_default();
        ((), (parse(entrypoint), parse(cmd)))
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
