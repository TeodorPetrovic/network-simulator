use super::*;

impl DynamipsRuntime {
    pub(super) async fn ensure_hypervisor(&self, vm: &DynamipsVm) -> Result<(), RuntimeError> {
        if self.router_process_alive(&vm.node_id).await
            && self
                .hypervisor_status(vm.hypervisor_port, &vm.name)
                .await
                .is_ok()
        {
            return Ok(());
        }
        if let Some(mut stale) = self.routers.lock().await.remove(&vm.node_id) {
            if stale.child.try_wait().ok().flatten().is_none() {
                stale.kill_group();
                let _ = timeout(Duration::from_secs(2), stale.child.wait()).await;
            }
            let _ = fs::remove_file(&stale.log_file);
        }
        remove_transient_runtime_files(&vm.working_dir)?;
        self.bindings.lock().await.retain(|(node_id, _, _), _| node_id != &vm.node_id);
        self.initialize_router(vm).await
    }

    pub(super) async fn spawn_hypervisor(
        &self,
        vm: &DynamipsVm,
    ) -> Result<ManagedRouter, RuntimeError> {
        let version = self.version().await?;
        fs::create_dir_all(&vm.working_dir).map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not create Dynamips working directory: {error}"
            ))
        })?;
        let log_file = vm.log_file.clone();
        let stdout = fs::File::create(&log_file).map_err(|error| {
            RuntimeError::Failed(format!("Could not create Dynamips log file: {error}"))
        })?;
        let stderr = stdout.try_clone().map_err(|error| {
            RuntimeError::Failed(format!("Could not open Dynamips log file: {error}"))
        })?;
        let port = vm.hypervisor_port;
        let mut command = Command::new(&self.binary);
        command
            .arg("-N1")
            .arg("-l")
            .arg("dynamips_engine.log")
            .arg("-H")
            .arg(format!("127.0.0.1:{port}"));
        // GNS3 uses this flag with Dynamips 0.2.23 and newer. Without it,
        // console and AUX listeners may bind to every host interface even
        // though the hypervisor protocol itself listens only on loopback.
        if dynamips_version_tuple(&version)
            .is_some_and(|version| version >= CONSOLE_BINDING_MIN_VERSION)
        {
            command.args(["--console-binding-addr", "127.0.0.1"]);
        }
        let child = command
            .current_dir(&vm.working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .process_group(0)
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                RuntimeError::Unavailable(format!(
                    "Could not start Dynamips executable '{}': {error}",
                    self.binary.display()
                ))
            })?;
        // Keep the group-owning guard alive during the readiness probe too:
        // canceling a start future must not leave a hypervisor descendant.
        let mut router = ManagedRouter {
            process_group_id: child.id().expect("spawned Dynamips has a PID"),
            child,
            group_killed: false,
            hypervisor_port: port,
            log_file,
        };
        let deadline = tokio::time::Instant::now() + PROCESS_READY_TIMEOUT;
        loop {
            if TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))
                .await
                .is_ok()
            {
                return Ok(router);
            }
            match router.child.try_wait() {
                Ok(Some(status)) => {
                    router.kill_group();
                    let _ = router.child.wait().await;
                    let log = fs::read_to_string(&router.log_file).unwrap_or_default();
                    return Err(RuntimeError::Failed(format!(
                        "Dynamips exited during startup ({status}){}",
                        if log.trim().is_empty() {
                            String::new()
                        } else {
                            format!(": {}", log.trim())
                        }
                    )));
                }
                Ok(None) => {}
                Err(error) => {
                    router.kill_group();
                    let _ = timeout(Duration::from_secs(2), router.child.wait()).await;
                    return Err(RuntimeError::Unavailable(format!(
                        "Could not inspect Dynamips startup: {error}"
                    )));
                }
            }
            if tokio::time::Instant::now() >= deadline {
                router.kill_group();
                let _ = router.child.wait().await;
                let log = fs::read_to_string(&router.log_file).unwrap_or_default();
                return Err(RuntimeError::Unavailable(format!(
                    "Dynamips hypervisor did not listen on 127.0.0.1:{port}{}",
                    if log.trim().is_empty() {
                        String::new()
                    } else {
                        format!(": {}", log.trim())
                    }
                )));
            }
            sleep(Duration::from_millis(50)).await;
        }
    }

    pub(super) async fn stop_process(&self, vm: &DynamipsVm) -> Result<(), RuntimeError> {
        let _ = self.send(vm.hypervisor_port, "hypervisor stop").await;
        if let Some(mut router) = self.routers.lock().await.remove(&vm.node_id) {
            if timeout(Duration::from_secs(3), router.child.wait())
                .await
                .is_err()
            {
                router.kill_group();
                let _ = timeout(Duration::from_secs(2), router.child.wait()).await;
            }
            let _ = fs::remove_file(&router.log_file);
        }
        Ok(())
    }

    pub(super) async fn router_process_alive(&self, node_id: &str) -> bool {
        let mut routers = self.routers.lock().await;
        let Some(router) = routers.get_mut(node_id) else {
            return false;
        };
        match router.child.try_wait() {
            Ok(None) => true,
            Ok(Some(_)) | Err(_) => false,
        }
    }

    pub(super) async fn router_process_exited(&self, node_id: &str) -> bool {
        let mut routers = self.routers.lock().await;
        let Some(router) = routers.get_mut(node_id) else {
            return false;
        };
        match router.child.try_wait() {
            Ok(Some(_)) | Err(_) => true,
            Ok(None) => false,
        }
    }

    pub(super) async fn reap_dead_hypervisor(&self, node_id: &str) -> bool {
        let dead_router = {
            let mut routers = self.routers.lock().await;
            let exited = routers
                .get_mut(node_id)
                .is_some_and(|router| !matches!(router.child.try_wait(), Ok(None)));
            exited.then(|| routers.remove(node_id)).flatten()
        };
        let Some(mut router) = dead_router else {
            return false;
        };
        router.kill_group();
        let _ = router.child.wait().await;
        let _ = fs::remove_file(&router.log_file);
        if let Some(working_dir) = router.log_file.parent() {
            let _ = remove_transient_runtime_files(working_dir);
        }
        self.bindings.lock().await.retain(|(bound_node, _, _), _| bound_node != node_id);
        true
    }
}
