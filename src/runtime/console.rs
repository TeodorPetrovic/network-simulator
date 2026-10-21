use super::*;

impl DockerRuntime {
    pub(super) async fn prepare_console_host(
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

    pub(super) async fn start_vnc_console(
        &self,
        resolution: &str,
    ) -> Result<DockerConsoleHost, RuntimeError> {
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

    pub(super) async fn start_http_console(&self, container_id: &str) -> Result<(), RuntimeError> {
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

    pub(super) async fn stop_http_console(&self, container_id: &str) {
        if let Some(host) = self.console_hosts.lock().await.get_mut(container_id)
            && let Some(task) = host.http_task.take()
        {
            task.abort();
        }
    }

    pub(super) async fn stop_console_host(&self, container_id: &str) {
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
}
