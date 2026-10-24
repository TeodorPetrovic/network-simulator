use super::*;

impl VpcsRuntime {
    pub fn new() -> Self {
        Self {
            binary: crate::image_paths::runtime_binary("NETWORK_SIMULATOR_VPCS", "vpcs"),
            state_dir: std::env::var_os("NETWORK_SIMULATOR_VPCS_STATE")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("data/vpcs")),
            processes: Arc::new(Mutex::new(HashMap::new())),
            active_ports: Arc::new(RwLock::new(HashMap::new())),
            bridge_ports: Arc::new(RwLock::new(HashMap::new())),
            requested_runtime_ports: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Kill VPCS processes left behind by an earlier controller run.
    pub fn reap_orphans(&self) -> usize {
        reap_orphaned_processes(&self.binary, &self.state_dir)
    }

    /// Kill every VPCS process this runtime started (controller shutdown).
    pub async fn kill_all(&self) {
        let mut processes = self.processes.lock().await;
        for (_, process) in processes.drain() {
            process.kill_group();
        }
        self.active_ports.write().await.clear();
    }

    #[cfg(test)]
    pub fn with_paths(binary: impl Into<PathBuf>, state_dir: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            state_dir: state_dir.into(),
            processes: Arc::new(Mutex::new(HashMap::new())),
            active_ports: Arc::new(RwLock::new(HashMap::new())),
            bridge_ports: Arc::new(RwLock::new(HashMap::new())),
            requested_runtime_ports: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn info(&self) -> VpcsRuntimeInfo {
        match self.version().await {
            Ok(version) => VpcsRuntimeInfo {
                available: true,
                version: Some(version),
                error: None,
            },
            Err(error) => VpcsRuntimeInfo {
                available: false,
                version: None,
                error: Some(error.message().to_owned()),
            },
        }
    }

    pub async fn templates(&self) -> Result<Vec<Value>, RuntimeError> {
        self.version().await?;
        let template_id = Uuid::new_v5(&Uuid::NAMESPACE_URL, b"network-simulator/vpcs");
        Ok(vec![json!({
            "template_id": template_id,
            "name": "VPCS",
            "template_type": "vpcs",
            "compute_id": "local",
            "category": "guest",
            "symbol": "vpcs_guest",
            "builtin": true,
            "adapters": 1,
            "memory": 0,
            "cpus": 0,
            "startup_script": "",
            "base_script_file": "vpcs_base_config.txt",
            "console_type": "telnet",
            "console_auto_start": false
        })])
    }

    pub async fn ensure_available(&self) -> Result<String, RuntimeError> {
        self.version().await
    }

    pub async fn set_bridge_port(&self, node_id: &str, port: u16) {
        self.bridge_ports
            .write()
            .await
            .insert(node_id.to_owned(), port);
    }

    pub async fn adapter_udp_port(&self, node_id: &str) -> Option<u16> {
        self.bridge_ports.read().await.get(node_id).copied()
    }

    pub async fn set_nio_ports(&self, node_id: &str, adapter_port: u16, runtime_port: u16) {
        self.bridge_ports
            .write()
            .await
            .insert(node_id.to_owned(), adapter_port);
        self.requested_runtime_ports
            .write()
            .await
            .insert(node_id.to_owned(), runtime_port);
    }

    pub async fn active_udp_port(&self, node_id: &str) -> Option<u16> {
        self.active_ports.read().await.get(node_id).copied()
    }

    pub async fn lifecycle(
        &self,
        project_id: &str,
        node_id: &str,
        name: &str,
        startup_script: Option<&str>,
        action: NodeAction,
    ) -> Result<String, RuntimeError> {
        match action {
            NodeAction::Start => self.start(project_id, node_id, name, startup_script).await,
            NodeAction::Stop => self.stop(node_id).await,
            // TODO(RUNTIME-NET VPCS suspend hook): apply node suspension by
            // dropping the connected NIO carrier; the VPCS process itself stays running.
            NodeAction::Suspend => Err(RuntimeError::Failed(
                "VPCS does not support process suspension".to_owned(),
            )),
            NodeAction::Reload => {
                self.stop(node_id).await?;
                self.start(project_id, node_id, name, startup_script).await
            }
        }
    }

    pub async fn status(&self, node_id: &str) -> Result<String, RuntimeError> {
        let mut processes = self.processes.lock().await;
        let Some(process) = processes.get_mut(node_id) else {
            return Ok("stopped".to_owned());
        };
        let exited = process.child.try_wait().map_err(|error| {
            RuntimeError::Unavailable(format!("Could not inspect VPCS process: {error}"))
        })?;
        if exited.is_some() {
            processes
                .remove(node_id)
                .expect("the exited VPCS process remains present");
            self.active_ports.write().await.remove(node_id);
            return Ok("stopped".to_owned());
        }
        Ok("started".to_owned())
    }

    pub async fn console_address(&self, node_id: &str) -> Result<SocketAddr, RuntimeError> {
        let processes = self.processes.lock().await;
        let process = processes.get(node_id).ok_or_else(|| {
            RuntimeError::Failed("Start the VPCS node before opening its console".to_owned())
        })?;
        Ok(process.console)
    }

    pub async fn connect_console(&self, node_id: &str) -> Result<TcpStream, RuntimeError> {
        let address = self.console_address(node_id).await?;
        connect_vpcs_console(address).await
    }

    pub async fn stop(&self, node_id: &str) -> Result<String, RuntimeError> {
        let process = self.processes.lock().await.remove(node_id);
        let Some(mut process) = process else {
            self.active_ports.write().await.remove(node_id);
            return Ok("stopped".to_owned());
        };
        if let Err(error) = stop_vpcs_process(&mut process).await {
            self.processes
                .lock()
                .await
                .insert(node_id.to_owned(), process);
            return Err(error);
        }
        process.kill_group();
        self.active_ports.write().await.remove(node_id);
        Ok("stopped".to_owned())
    }

    pub async fn remove_node(&self, project_id: &str, node_id: &str) -> Result<(), RuntimeError> {
        self.stop(node_id).await?;
        self.bridge_ports.write().await.remove(node_id);
        self.requested_runtime_ports.write().await.remove(node_id);
        let project_id = parse_id(project_id)?;
        let node_id = parse_id(node_id)?;
        let node_dir = self
            .state_dir
            .join(project_id.to_string())
            .join(node_id.to_string());
        if node_dir.exists() {
            fs::remove_dir_all(&node_dir).map_err(|error| {
                RuntimeError::Failed(format!("Could not remove VPCS node files: {error}"))
            })?;
        }
        Ok(())
    }

    async fn start(
        &self,
        project_id: &str,
        node_id: &str,
        name: &str,
        startup_script: Option<&str>,
    ) -> Result<String, RuntimeError> {
        let project_id = parse_id(project_id)?;
        let node_id_uuid = parse_id(node_id)?;
        self.version().await?;

        if self.status(node_id).await? == "started" {
            return Ok("started".to_owned());
        }

        let mut processes = self.processes.lock().await;
        if processes.contains_key(node_id) {
            return Ok("started".to_owned());
        }

        let switch_port = if let Some(port) = self.adapter_udp_port(node_id).await {
            port
        } else {
            let port = available_udp_port().await?;
            self.set_bridge_port(node_id, port).await;
            port
        };

        let node_dir = self
            .state_dir
            .join(project_id.to_string())
            .join(node_id_uuid.to_string());
        fs::create_dir_all(&node_dir).map_err(|error| {
            RuntimeError::Failed(format!("Could not create VPCS node directory: {error}"))
        })?;

        let script_path = write_startup_script(&node_dir, startup_script, name)?;

        let console_port = available_tcp_port().await?;
        let local_udp_port = if let Some(port) = self
            .requested_runtime_ports
            .read()
            .await
            .get(node_id)
            .copied()
        {
            port
        } else {
            available_udp_port().await?
        };
        let log_path = node_dir.join("vpcs.log");
        let log_file = fs::File::create(&log_path).map_err(|error| {
            RuntimeError::Failed(format!("Could not create VPCS log file: {error}"))
        })?;
        let stderr = log_file.try_clone().map_err(|error| {
            RuntimeError::Failed(format!("Could not open VPCS log file: {error}"))
        })?;
        self.active_ports
            .write()
            .await
            .insert(node_id.to_owned(), local_udp_port);

        let mut command = Command::new(&self.binary);
        command
            .args([
                "-p".to_owned(),
                console_port.to_string(),
                "-m".to_owned(),
                mac_offset(&node_id_uuid).to_string(),
                "-i".to_owned(),
                "1".to_owned(),
                "-F".to_owned(),
                "-R".to_owned(),
                "-s".to_owned(),
                local_udp_port.to_string(),
                "-c".to_owned(),
                switch_port.to_string(),
                "-t".to_owned(),
                "127.0.0.1".to_owned(),
            ])
            .current_dir(&node_dir)
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(stderr))
            .stdin(Stdio::null())
            .process_group(0)
            .kill_on_drop(true);
        command.arg(script_path.file_name().unwrap_or_default());
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                self.active_ports.write().await.remove(node_id);
                return Err(RuntimeError::Unavailable(format!(
                    "Could not start VPCS executable '{}': {error}",
                    self.binary.display()
                )));
            }
        };

        let console = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, console_port));
        let deadline = tokio::time::Instant::now() + START_TIMEOUT;
        loop {
            if connect_vpcs_console(console).await.is_ok() {
                processes.insert(node_id.to_owned(), ManagedVpcs { child, console });
                return Ok("started".to_owned());
            }
            match child.try_wait() {
                Ok(Some(status)) => {
                    self.active_ports.write().await.remove(node_id);
                    let log = fs::read_to_string(&log_path).unwrap_or_default();
                    return Err(RuntimeError::Failed(format!(
                        "VPCS exited during startup ({status}){}",
                        if log.trim().is_empty() {
                            String::new()
                        } else {
                            format!(": {}", log.trim())
                        }
                    )));
                }
                Ok(None) => {}
                Err(error) => {
                    kill_vpcs_startup_tree(&mut child).await;
                    self.active_ports.write().await.remove(node_id);
                    return Err(RuntimeError::Unavailable(format!(
                        "Could not inspect VPCS startup: {error}"
                    )));
                }
            }
            if tokio::time::Instant::now() >= deadline {
                kill_vpcs_startup_tree(&mut child).await;
                self.active_ports.write().await.remove(node_id);
                let log = fs::read_to_string(&log_path).unwrap_or_default();
                return Err(RuntimeError::Unavailable(format!(
                    "VPCS console did not become ready{}",
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

    async fn version(&self) -> Result<String, RuntimeError> {
        let output = timeout(
            COMMAND_TIMEOUT,
            Command::new(&self.binary)
                .arg("-v")
                .stdin(Stdio::null())
                .output(),
        )
        .await
        .map_err(|_| RuntimeError::Unavailable("VPCS version check timed out".to_owned()))?
        .map_err(|error| {
            RuntimeError::Unavailable(format!(
                "Could not run VPCS executable '{}': {error}",
                self.binary.display()
            ))
        })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(RuntimeError::Failed(if stderr.is_empty() {
                format!("VPCS version check exited with {}", output.status)
            } else {
                stderr
            }));
        }
        let output = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let line = output.lines().find(|line| line.contains("version"));
        line.map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| RuntimeError::Failed("Could not determine the VPCS version".to_owned()))
    }
}

