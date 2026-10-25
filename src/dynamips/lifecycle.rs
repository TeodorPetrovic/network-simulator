use super::*;

impl DynamipsRuntime {
    pub async fn lifecycle(
        &self,
        vm: &DynamipsVm,
        action: NodeAction,
    ) -> Result<String, RuntimeError> {
        let _guard = self.operation_lock.lock().await;
        match action {
            NodeAction::Start => self.start_locked(vm).await,
            NodeAction::Stop => {
                if self.reap_dead_hypervisor(&vm.node_id).await {
                    self.bindings.lock().await.retain(|(node_id, _, _), _| node_id != &vm.node_id);
                    return Ok("stopped".to_owned());
                }
                self.stop_locked(vm).await
            }
            NodeAction::Suspend => {
                let status = self.hypervisor_status(vm.hypervisor_port, &vm.name).await?;
                if status == "started" {
                    self.send(
                        vm.hypervisor_port,
                        &format!("vm suspend {}", quote(&vm.name)),
                    )
                    .await?;
                }
                self.hypervisor_status(vm.hypervisor_port, &vm.name).await
            }
            NodeAction::Reload => {
                self.stop_locked(vm).await?;
                self.start_locked(vm).await
            }
        }
    }

    pub async fn status(&self, vm: &DynamipsVm) -> Result<String, RuntimeError> {
        if self.router_process_exited(&vm.node_id).await {
            return Ok("stopped".to_owned());
        }
        self.hypervisor_status(vm.hypervisor_port, &vm.name).await
    }

    pub async fn console(&self, vm: &DynamipsVm) -> Result<TcpStream, RuntimeError> {
        if vm.console_port == 0
            || vm.options.console_type == "none"
            || self.status(vm).await? != "started"
        {
            return Err(RuntimeError::Failed(
                "Start the Dynamips router before opening its console".to_owned(),
            ));
        }
        timeout(
            Duration::from_secs(3),
            TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, vm.console_port)),
        )
        .await
        .map_err(|_| RuntimeError::Unavailable("Dynamips console connection timed out".to_owned()))?
        .map_err(|error| {
            RuntimeError::Unavailable(format!(
                "Could not connect to the Dynamips console: {error}"
            ))
        })
    }

    pub async fn remove_router(&self, vm: &DynamipsVm) -> Result<(), RuntimeError> {
        let _guard = self.operation_lock.lock().await;
        let mut first_error = None;
        if self.router_process_alive(&vm.node_id).await {
            if let Err(error) = self
                .send(
                    vm.hypervisor_port,
                    &format!("vm clean_delete {}", quote(&vm.name)),
                )
                .await
            {
                first_error = Some(error);
            }
            let _ = self.send(vm.hypervisor_port, "hypervisor stop").await;
        }
        if let Some(mut router) = self.routers.lock().await.remove(&vm.node_id) {
            if router.child.try_wait().ok().flatten().is_none() {
                router.kill_group();
                let _ = timeout(Duration::from_secs(3), router.child.wait()).await;
            }
            let _ = fs::remove_file(&router.log_file);
        }
        remove_transient_runtime_files(&vm.working_dir)?;
        remove_runtime_manifest(&vm.working_dir)?;
        remove_generated_runtime_configs(&vm.working_dir)?;
        self.bindings.lock().await.retain(|(node_id, _, _), _| node_id != &vm.node_id);
        if vm.options.auto_delete_disks {
            remove_auto_delete_disk_files(&vm.working_dir, &vm.platform, vm.options.dynamips_id)?;
        }
        remove_empty_runtime_directories(&vm.working_dir)?;
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(())
    }

    pub(super) async fn initialize_router(&self, vm: &DynamipsVm) -> Result<(), RuntimeError> {
        let router = self.spawn_hypervisor(vm).await?;
        let port = router.hypervisor_port;
        self.routers.lock().await.insert(vm.node_id.clone(), router);
        let initialize = async {
            self.send(port, "hypervisor version").await?;
            self.send(
                port,
                &format!(
                    "hypervisor working_dir {}",
                    quote(&vm.working_dir.display().to_string())
                ),
            )
            .await?;
            for command in router_initialization_commands(vm) {
                self.send(port, &command).await?;
            }
            Ok::<(), RuntimeError>(())
        }
        .await;
        if let Err(error) = initialize {
            let _ = self.stop_process(vm).await;
            return Err(error);
        }
        Ok(())
    }

    pub(super) async fn start_locked(&self, vm: &DynamipsVm) -> Result<String, RuntimeError> {
        self.version().await?;
        self.ensure_hypervisor(vm).await?;
        match self.hypervisor_status(vm.hypervisor_port, &vm.name).await? {
            status if status == "started" => return Ok(status),
            status if status == "suspended" => {
                self.send(
                    vm.hypervisor_port,
                    &format!("vm resume {}", quote(&vm.name)),
                )
                .await?;
                return self.hypervisor_status(vm.hypervisor_port, &vm.name).await;
            }
            _ => {}
        }
        validate_ios_image(&vm.image)?;
        self.ensure_ghost_ios(vm).await;
        fs::write(
            vm.working_dir.join(format!(
                "configs/i{}_startup-config.cfg",
                vm.options.dynamips_id
            )),
            &vm.startup_config,
        )
        .map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not write Dynamips startup configuration: {error}"
            ))
        })?;
        let private_config_path = vm.working_dir.join(format!(
            "configs/i{}_private-config.cfg",
            vm.options.dynamips_id
        ));
        if !vm.options.private_config.is_empty() {
            fs::write(&private_config_path, &vm.options.private_config).map_err(|error| {
                RuntimeError::Failed(format!(
                    "Could not write Dynamips private configuration: {error}"
                ))
            })?;
        }
        self.send(
            vm.hypervisor_port,
            &format!(
                "vm set_config {} {} {}",
                quote(&vm.name),
                quote(&format!(
                    "configs/i{}_startup-config.cfg",
                    vm.options.dynamips_id
                )),
                if vm.options.private_config.is_empty() {
                    "\"\"".to_owned()
                } else {
                    quote(&format!(
                        "configs/i{}_private-config.cfg",
                        vm.options.dynamips_id
                    ))
                }
            ),
        )
        .await?;
        self.send(vm.hypervisor_port, &format!("vm start {}", quote(&vm.name)))
            .await?;
        // Dynamips 0.2.25 rejects these commands before the CPU exists
        // (`Bad CPU specified`); apply them as soon as the VM has started.
        for command in router_start_settings_commands(vm) {
            self.send(vm.hypervisor_port, &command).await?;
        }
        self.hypervisor_status(vm.hypervisor_port, &vm.name).await
    }

    pub(super) async fn ensure_ghost_ios(&self, vm: &DynamipsVm) {
        let (Some(ghost_file), true) = (
            vm.ghost_file.as_ref(),
            vm.options.ghost_ios && vm.options.mmap,
        ) else {
            return;
        };
        if vm.platform == "c7200" && vm.options.npe.as_deref() == Some("npe-g2") {
            tracing::warn!(node_id = %vm.node_id, "Dynamips ghost IOS is unsupported for c7200 with NPE-G2");
            return;
        }
        if let Some(parent) = ghost_file.parent()
            && let Err(error) = fs::create_dir_all(parent)
        {
            tracing::warn!(error = %error, "Could not create Dynamips ghost IOS cache directory");
            return;
        }
        if !ghost_file.is_file() {
            let name = "Ghost";
            let create = async {
                for command in ghost_router_initialization_commands(vm, ghost_file) {
                    self.send(vm.hypervisor_port, &command).await?;
                }
                // Dynamips writes the ghost RAM image while `vm start` loads
                // the IOS ELF, so GNS3 stops the ghost instance immediately.
                self.send(vm.hypervisor_port, &format!("vm start {}", quote(name)))
                    .await?;
                self.send(vm.hypervisor_port, &format!("vm stop {}", quote(name)))
                    .await?;
                Ok::<(), RuntimeError>(())
            }
            .await;
            let cleanup = self
                .send(
                    vm.hypervisor_port,
                    &format!("vm clean_delete {}", quote(name)),
                )
                .await;
            if let Err(error) = create {
                tracing::warn!(error = %error.message(), node_id = %vm.node_id, "Could not generate Dynamips ghost IOS cache");
                let _ = cleanup;
                return;
            }
            if let Err(error) = cleanup {
                tracing::warn!(error = %error.message(), "Could not clean up temporary Dynamips ghost router");
            }
        }
        if ghost_file.is_file() {
            for command in [
                format!("vm set_ghost_status {} 2", quote(&vm.name)),
                format!(
                    "vm set_ghost_file {} {}",
                    quote(&vm.name),
                    quote(&ghost_file.display().to_string())
                ),
            ] {
                if let Err(error) = self.send(vm.hypervisor_port, &command).await {
                    tracing::warn!(error = %error.message(), node_id = %vm.node_id, "Could not attach Dynamips ghost IOS cache");
                    break;
                }
            }
        }
    }

    pub(super) async fn cpu_usage(&self, port: u16, name: &str) -> Result<i64, RuntimeError> {
        self.send(port, &format!("vm cpu_usage {} 0", quote(name)))
            .await?
            .first()
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| RuntimeError::Failed("Dynamips returned invalid CPU usage".to_owned()))
    }

    pub(super) async fn stop_locked(&self, vm: &DynamipsVm) -> Result<String, RuntimeError> {
        let status = self.hypervisor_status(vm.hypervisor_port, &vm.name).await?;
        if status != "stopped" {
            self.send(vm.hypervisor_port, &format!("vm stop {}", quote(&vm.name)))
                .await?;
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let status = self.hypervisor_status(vm.hypervisor_port, &vm.name).await?;
            if status == "stopped" || tokio::time::Instant::now() >= deadline {
                return Ok(status);
            }
            sleep(Duration::from_millis(100)).await;
        }
    }

    pub(super) async fn hypervisor_status(
        &self,
        port: u16,
        name: &str,
    ) -> Result<String, RuntimeError> {
        let response = self
            .send(port, &format!("vm get_status {}", quote(name)))
            .await?;
        match response.first().map(String::as_str) {
            Some("0") => Ok("stopped".to_owned()),
            Some("1") => Ok("stopping".to_owned()),
            Some("2") => Ok("started".to_owned()),
            Some("3") => Ok("suspended".to_owned()),
            _ => Err(RuntimeError::Failed(format!(
                "Dynamips returned an invalid router status: {}",
                response.join(" ")
            ))),
        }
    }
}
