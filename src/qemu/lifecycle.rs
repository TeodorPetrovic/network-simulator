use super::*;

impl QemuRuntime {
    pub(super) async fn start(&self, vm: &QemuVm, links: &[QemuLink]) -> Result<String, RuntimeError> {
        if !vm.overlay.is_file() {
            return Err(RuntimeError::NotFound(format!(
                "QEMU node disk overlay '{}' is missing",
                vm.overlay.display()
            )));
        }
        if process_matches(&vm.pid_file, &vm.qmp_socket) {
            if !vm.qmp_socket.exists() {
                return Err(RuntimeError::Unavailable(
                    "QEMU is running but its QMP monitor socket is missing".to_owned(),
                ));
            }
            match self.status(vm).await {
                Ok(status) if status == "started" => {
                    self.start_cpu_limiter(vm).await?;
                    return Ok("started".to_owned());
                }
                Ok(status) if status == "suspended" => {
                    self.qmp_execute(&vm.qmp_socket, "cont", None).await?;
                    self.start_cpu_limiter(vm).await?;
                    return self.status(vm).await;
                }
                Ok(status) if status == "stopped" => {
                    self.qmp_execute(&vm.qmp_socket, "system_reset", None)
                        .await?;
                    return self.status(vm).await;
                }
                Ok(_) => {
                    return Err(RuntimeError::Failed(
                        "QEMU is running in a state that cannot be resumed".to_owned(),
                    ));
                }
                Err(error) => {
                    return Err(RuntimeError::Unavailable(format!(
                        "QEMU is running but its QMP monitor is not responding: {}",
                        error.message()
                    )));
                }
            }
        }
        let _ = fs::remove_file(&vm.qmp_socket);
        let _ = fs::remove_file(&vm.serial_socket);
        let _ = fs::remove_file(&vm.pid_file);
        let _ = fs::remove_file(&vm.log_file);
        let runtime_arguments = qemu_runtime_arguments(vm)?;
        let mut command = Command::new(&self.binary);
        command
            .arg("-daemonize")
            .arg("-pidfile")
            .arg(&vm.pid_file)
            .arg("-D")
            .arg(&vm.log_file)
            .arg("-qmp")
            .arg(format!(
                "unix:{},server=on,wait=off",
                vm.qmp_socket.display()
            ));
        command.arg("-monitor").arg("none");
        if matches!(vm.console_type.as_str(), "telnet" | "ssh") {
            command
                .arg("-chardev")
                .arg(format!(
                    "socket,id=serial0,path={},server=on,wait=off",
                    qemu_keyval_escape(&vm.serial_socket.display().to_string())
                ))
                .arg("-serial")
                .arg("chardev:serial0");
        }
        // QEMU treats QMP system_reset as a shutdown when -no-reboot is set.
        command.args(runtime_arguments);
        // TODO(RUNTIME-NET QEMU custom adapter hook): materialize adapter
        // counts without links, custom_adapters, configured MAC addresses, and
        // carrier replication through NET's NIC work.
        if links.is_empty() {
            command.arg("-nic").arg("none");
        } else {
            let mut links = links.to_vec();
            links.sort_by_key(|link| link.adapter_number);
            for adapter_number in 0..vm.adapters {
                if let Some(link) = links
                    .iter()
                    .find(|link| link.adapter_number == adapter_number)
                {
                    let id = qemu_nio_id(&link.link_id);
                    let net_id = format!("ns_{id}");
                    command
                        .arg("-netdev")
                        .arg(qemu_netdev_argument(link, &net_id))
                        .arg("-device")
                        .arg(format!(
                            "{},netdev={net_id},mac={}",
                            vm.adapter_type,
                            mac_address(&vm.node_id, adapter_number)
                        ));
                } else {
                    let net_id = format!("ns_unused_{adapter_number}");
                    command
                        .arg("-netdev")
                        .arg(format!("null,id={net_id}"))
                        .arg("-device")
                        .arg(format!(
                            "{},netdev={net_id},mac={}",
                            vm.adapter_type,
                            mac_address(&vm.node_id, adapter_number)
                        ));
                }
            }
        }
        if vm.tpm {
            self.start_tpm(vm).await?;
        }
        set_child_process_priority(&mut command, &vm.process_priority)?;
        let output = timeout(COMMAND_TIMEOUT, command.stdin(Stdio::null()).output())
            .await
            .map_err(|_| RuntimeError::Unavailable("QEMU startup timed out".to_owned()))
            .and_then(|output| {
                output.map_err(|error| {
                    RuntimeError::Unavailable(format!(
                        "Could not run QEMU binary '{}': {error}",
                        self.binary.display()
                    ))
                })
            });
        let output = match output {
            Ok(output) => output,
            Err(error) => {
                self.stop_tpm(vm).await;
                return Err(error);
            }
        };
        if !output.status.success() {
            self.stop_tpm(vm).await;
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(RuntimeError::Failed(if stderr.is_empty() {
                format!("QEMU exited with {}", output.status)
            } else {
                stderr
            }));
        }
        let deadline = tokio::time::Instant::now() + START_TIMEOUT;
        loop {
            if vm.qmp_socket.exists() {
                match self.status(vm).await {
                    Ok(status) if status == "started" || status == "suspended" => {
                        if status == "started"
                            && let Err(error) = self.start_cpu_limiter(vm).await
                        {
                            let _ = self.stop(vm).await;
                            return Err(error);
                        }
                        return Ok(status);
                    }
                    Ok(_) => {}
                    Err(error) => {
                        if tokio::time::Instant::now() >= deadline {
                            self.stop_tpm(vm).await;
                            let log = fs::read_to_string(&vm.log_file).unwrap_or_default();
                            return Err(RuntimeError::Failed(format!(
                                "QEMU started without a usable QMP monitor: {}{}",
                                error.message(),
                                if log.is_empty() {
                                    String::new()
                                } else {
                                    format!("; {log}")
                                }
                            )));
                        }
                    }
                }
            }
            if tokio::time::Instant::now() >= deadline {
                self.stop_tpm(vm).await;
                let log = fs::read_to_string(&vm.log_file).unwrap_or_default();
                return Err(RuntimeError::Failed(format!(
                    "QEMU did not create its QMP monitor{}",
                    if log.is_empty() {
                        String::new()
                    } else {
                        format!(": {log}")
                    }
                )));
            }
            sleep(Duration::from_millis(100)).await;
        }
    }

    pub(super) async fn stop(&self, vm: &QemuVm) -> Result<String, RuntimeError> {
        let pid_matches = process_matches(&vm.pid_file, &vm.qmp_socket);
        if !vm.qmp_socket.exists() && pid_matches {
            return Err(RuntimeError::Unavailable(
                "QEMU is running but its QMP monitor socket is missing".to_owned(),
            ));
        }
        if !vm.qmp_socket.exists() {
            let _ = fs::remove_file(&vm.qmp_socket);
            let _ = fs::remove_file(&vm.serial_socket);
            let _ = fs::remove_file(&vm.pid_file);
            self.stop_tpm(vm).await;
            self.stop_cpu_limiter(vm).await;
            return Ok("stopped".to_owned());
        }
        if vm.on_close == "shutdown_signal" {
            let _ = self
                .qmp_execute(&vm.qmp_socket, "system_powerdown", None)
                .await;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
            while process_matches(&vm.pid_file, &vm.qmp_socket)
                && tokio::time::Instant::now() < deadline
            {
                sleep(Duration::from_millis(200)).await;
            }
        } else if vm.on_close == "save_vm_state" {
            self.qmp_execute(&vm.qmp_socket, "stop", None).await?;
            self.qmp_execute(
                &vm.qmp_socket,
                "human-monitor-command",
                Some(json!({ "command-line": "savevm GNS3_SAVED_STATE" })),
            )
            .await?;
        }
        if process_matches(&vm.pid_file, &vm.qmp_socket)
            && let Err(error) = self.qmp_execute(&vm.qmp_socket, "quit", None).await
        {
            if pid_matches {
                return Err(RuntimeError::Unavailable(format!(
                    "Could not stop QEMU because its QMP monitor is unavailable: {}",
                    error.message()
                )));
            }
            let _ = fs::remove_file(&vm.qmp_socket);
            let _ = fs::remove_file(&vm.serial_socket);
            let _ = fs::remove_file(&vm.pid_file);
            self.stop_tpm(vm).await;
            self.stop_cpu_limiter(vm).await;
            return Ok("stopped".to_owned());
        }
        let deadline = tokio::time::Instant::now() + START_TIMEOUT;
        while process_matches(&vm.pid_file, &vm.qmp_socket) {
            if tokio::time::Instant::now() >= deadline {
                return Err(RuntimeError::Failed(
                    "QEMU did not exit after the QMP quit request".to_owned(),
                ));
            }
            sleep(Duration::from_millis(100)).await;
        }
        let _ = fs::remove_file(&vm.qmp_socket);
        let _ = fs::remove_file(&vm.serial_socket);
        let _ = fs::remove_file(&vm.pid_file);
        self.stop_tpm(vm).await;
        self.stop_cpu_limiter(vm).await;
        Ok("stopped".to_owned())
    }

    pub(super) async fn start_cpu_limiter(&self, vm: &QemuVm) -> Result<(), RuntimeError> {
        if vm.cpu_throttling == 0
            || self
                .cpu_limit_processes
                .lock()
                .await
                .contains_key(&vm.node_id)
        {
            return Ok(());
        }
        let pid = fs::read_to_string(&vm.pid_file)
            .map_err(|error| {
                RuntimeError::Failed(format!(
                    "Could not read QEMU pid file for CPU throttling: {error}"
                ))
            })?
            .trim()
            .parse::<u32>()
            .map_err(|_| {
                RuntimeError::Failed("QEMU pid file contains an invalid process ID".to_owned())
            })?;
        let binary = std::env::var_os("NETWORK_SIMULATOR_CPULIMIT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("cpulimit"));
        let mut child = Command::new(&binary)
            .args(cpu_limiter_arguments(pid, vm.cpu_throttling))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| RuntimeError::Unavailable(format!(
                "QEMU CPU throttling is enabled but cpulimit could not be started with '{}': {error}",
                binary.display()
            )))?;
        if let Some(status) = child.try_wait().map_err(|error| {
            RuntimeError::Unavailable(format!("Could not inspect cpulimit startup: {error}"))
        })? {
            return Err(RuntimeError::Failed(format!(
                "cpulimit exited during startup ({status})"
            )));
        }
        self.cpu_limit_processes
            .lock()
            .await
            .insert(vm.node_id.clone(), child);
        Ok(())
    }

    pub(super) async fn stop_cpu_limiter(&self, vm: &QemuVm) {
        let Some(mut child) = self.cpu_limit_processes.lock().await.remove(&vm.node_id) else {
            return;
        };
        if child.try_wait().ok().flatten().is_none() {
            let _ = child.start_kill();
            let _ = timeout(Duration::from_secs(2), child.wait()).await;
        }
    }

    pub(super) async fn start_tpm(&self, vm: &QemuVm) -> Result<(), RuntimeError> {
        if self.tpm_processes.lock().await.contains_key(&vm.node_id) {
            return Ok(());
        }
        let binary = std::env::var_os("NETWORK_SIMULATOR_SWTPM")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("swtpm"));
        let working_dir = vm
            .pid_file
            .parent()
            .ok_or_else(|| RuntimeError::Failed("QEMU working directory is missing".to_owned()))?;
        let state_dir = working_dir.join("tpm");
        fs::create_dir_all(&state_dir).map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not create QEMU TPM state directory: {error}"
            ))
        })?;
        let socket = tpm_socket_path(vm);
        let _ = fs::remove_file(&socket);
        let state = format!("dir={}", state_dir.display());
        let server = format!("type=unixio,path={},mode=0600", socket.display());
        let mut child = Command::new(&binary)
            .args([
                "socket",
                "--tpm2",
                "--tpmstate",
                &state,
                "--server",
                &server,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                RuntimeError::Unavailable(format!(
                    "TPM is enabled but swtpm could not be started with '{}': {error}",
                    binary.display()
                ))
            })?;
        let deadline = tokio::time::Instant::now() + START_TIMEOUT;
        while !socket.exists() {
            if let Some(status) = child.try_wait().map_err(|error| {
                RuntimeError::Unavailable(format!("Could not inspect swtpm startup: {error}"))
            })? {
                return Err(RuntimeError::Failed(format!(
                    "swtpm exited during startup ({status})"
                )));
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(RuntimeError::Unavailable(
                    "swtpm did not create its Unix socket".to_owned(),
                ));
            }
            sleep(Duration::from_millis(50)).await;
        }
        self.tpm_processes
            .lock()
            .await
            .insert(vm.node_id.clone(), child);
        Ok(())
    }

    pub(super) async fn stop_tpm(&self, vm: &QemuVm) {
        let Some(mut process) = self.tpm_processes.lock().await.remove(&vm.node_id) else {
            return;
        };
        if process.try_wait().ok().flatten().is_none() {
            let _ = process.start_kill();
            let _ = timeout(Duration::from_secs(2), process.wait()).await;
        }
        let _ = fs::remove_file(tpm_socket_path(vm));
    }

}
