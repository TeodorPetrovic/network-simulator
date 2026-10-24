use super::*;

pub(super) async fn qemu_console(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let project_id = project_id.to_string();
    let node_id = node_id.to_string();
    let node = get_topology_object(state.store.pool(), &project_id, "node", &node_id)
        .await?
        .0;
    if node["node_type"].as_str() != Some("qemu") {
        return Err(ApiError::conflict(
            "Serial consoles are available for QEMU nodes only",
        ));
    }
    let vm = state.qemu.vm_from_node(&node).map_err(runtime_api_error)?;
    if state.qemu.status(&vm).await.map_err(runtime_api_error)? != "started" {
        return Err(ApiError::conflict(
            "Start the QEMU node before opening its console",
        ));
    }
    let serial = UnixStream::connect(&vm.serial_socket)
        .await
        .map_err(|error| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("Could not connect to the QEMU serial console: {error}"),
            )
        })?;
    let activity = state.project_activity.clone();
    Ok(websocket.on_upgrade(move |socket| async move {
        let _activity = activity.open_websocket(project_id);
        qemu_console_session(socket, serial).await;
    }))
}

pub(super) async fn docker_console(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let project_id = project_id.to_string();
    let node_id = node_id.to_string();
    let node = get_topology_object(state.store.pool(), &project_id, "node", &node_id)
        .await?
        .0;
    if node["node_type"].as_str() != Some("docker") {
        return Err(ApiError::conflict(
            "Attached Docker consoles are available for Docker nodes only",
        ));
    }
    let console_type = node["console_type"].as_str().unwrap_or("telnet");
    if console_type != "telnet" {
        let detail = if matches!(console_type, "http" | "https" | "vnc") {
            "use the Docker console TCP port"
        } else {
            "the Docker WebSocket route supports telnet consoles"
        };
        return Err(ApiError::conflict(format!(
            "Docker {console_type} console is configured; {detail}"
        )));
    }
    let container_id = node["container_id"]
        .as_str()
        .ok_or_else(|| ApiError::conflict("Docker node has no runtime container"))?;
    if state
        .runtime
        .container_status(container_id)
        .await
        .map_err(runtime_api_error)?
        != "started"
    {
        return Err(ApiError::conflict(
            "Start the Docker node before opening its console",
        ));
    }
    let process = state
        .runtime
        .attach_process(container_id)
        .map_err(runtime_api_error)?;
    let runtime = state.runtime.clone();
    let container_id = container_id.to_owned();
    let activity = state.project_activity.clone();
    Ok(websocket.on_upgrade(move |socket| async move {
        let _activity = activity.open_websocket(project_id);
        docker_console_session(socket, process, runtime, container_id, true).await;
    }))
}

pub(super) async fn docker_aux_console(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let project_id = project_id.to_string();
    let node_id = node_id.to_string();
    let node = get_topology_object(state.store.pool(), &project_id, "node", &node_id)
        .await?
        .0;
    if node["node_type"].as_str() != Some("docker") {
        return Err(ApiError::conflict(
            "Auxiliary Docker consoles are available for Docker nodes only",
        ));
    }
    if node["aux_type"].as_str().unwrap_or("none") == "none" {
        return Err(ApiError::conflict(
            "Configure a Docker auxiliary console before opening it",
        ));
    }
    let container_id = node["container_id"]
        .as_str()
        .ok_or_else(|| ApiError::conflict("Docker node has no runtime container"))?;
    if state
        .runtime
        .container_status(container_id)
        .await
        .map_err(runtime_api_error)?
        != "started"
    {
        return Err(ApiError::conflict(
            "Start the Docker node before opening its auxiliary console",
        ));
    }
    let process = state
        .runtime
        .attach_aux_process(container_id)
        .map_err(runtime_api_error)?;
    let runtime = state.runtime.clone();
    let container_id = container_id.to_owned();
    let activity = state.project_activity.clone();
    Ok(websocket.on_upgrade(move |socket| async move {
        let _activity = activity.open_websocket(project_id);
        docker_console_session(socket, process, runtime, container_id, false).await;
    }))
}

pub(super) async fn vpcs_console(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let project_id = project_id.to_string();
    let node_id = node_id.to_string();
    let node = get_topology_object(state.store.pool(), &project_id, "node", &node_id)
        .await?
        .0;
    if node["node_type"].as_str() != Some("vpcs") {
        return Err(ApiError::conflict(
            "VPCS consoles are available for VPCS nodes only",
        ));
    }
    match node["console_type"].as_str().unwrap_or("telnet") {
        "none" => {
            return Err(ApiError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                "VPCS console is disabled for this node",
            ));
        }
        "telnet" | "ssh" => {}
        _ => {
            return Err(ApiError::bad_request(
                "VPCS console_type must be telnet, ssh, or none",
            ));
        }
    }
    if state
        .vpcs
        .status(&node_id)
        .await
        .map_err(runtime_api_error)?
        != "started"
    {
        return Err(ApiError::conflict(
            "Start the VPCS node before opening its console",
        ));
    }
    let console = state
        .vpcs
        .connect_console(&node_id)
        .await
        .map_err(runtime_api_error)?;
    let activity = state.project_activity.clone();
    Ok(websocket.on_upgrade(move |socket| async move {
        let _activity = activity.open_websocket(project_id);
        vpcs_console_session(socket, console).await;
    }))
}

pub(super) async fn dynamips_console(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let project_id = project_id.to_string();
    let node_id = node_id.to_string();
    let node = get_topology_object(state.store.pool(), &project_id, "node", &node_id)
        .await?
        .0;
    if node["node_type"].as_str() != Some("dynamips") {
        return Err(ApiError::conflict(
            "Console access is available for Dynamips IOS routers only",
        ));
    }
    let vm = state
        .dynamips
        .vm_from_node(&node)
        .map_err(runtime_api_error)?;
    let console = state
        .dynamips
        .console(&vm)
        .await
        .map_err(runtime_api_error)?;
    let activity = state.project_activity.clone();
    Ok(websocket.on_upgrade(move |socket| async move {
        let _activity = activity.open_websocket(project_id);
        vpcs_console_session(socket, console).await;
    }))
}

pub(super) async fn qemu_console_session(socket: WebSocket, serial: UnixStream) {
    let (mut websocket_sink, mut websocket_stream) = socket.split();
    let (mut serial_reader, mut serial_writer) = tokio::io::split(serial);
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        tokio::select! {
            incoming = websocket_stream.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    if serial_writer.write_all(text.as_bytes()).await.is_err() { break; }
                }
                Some(Ok(Message::Binary(bytes))) => {
                    if serial_writer.write_all(&bytes).await.is_err() { break; }
                }
                Some(Ok(Message::Ping(payload))) => {
                    if websocket_sink.send(Message::Pong(payload)).await.is_err() { break; }
                }
                Some(Ok(Message::Pong(_))) => {}
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
            },
            received = serial_reader.read(&mut buffer) => match received {
                Ok(0) | Err(_) => break,
                Ok(length) => {
                    if websocket_sink.send(Message::Binary(buffer[..length].to_vec().into())).await.is_err() { break; }
                }
            }
        }
    }
    let _ = websocket_sink.close().await;
}

pub(super) async fn vpcs_console_session(socket: WebSocket, console: TcpStream) {
    let (mut websocket_sink, mut websocket_stream) = socket.split();
    let (mut console_reader, mut console_writer) = tokio::io::split(console);
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        tokio::select! {
            incoming = websocket_stream.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    if console_writer.write_all(text.as_bytes()).await.is_err() { break; }
                }
                Some(Ok(Message::Binary(bytes))) => {
                    if console_writer.write_all(&bytes).await.is_err() { break; }
                }
                Some(Ok(Message::Ping(payload))) => {
                    if websocket_sink.send(Message::Pong(payload)).await.is_err() { break; }
                }
                Some(Ok(Message::Pong(_))) => {}
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
            },
            received = console_reader.read(&mut buffer) => match received {
                Ok(0) | Err(_) => break,
                Ok(length) => {
                    if websocket_sink.send(Message::Binary(buffer[..length].to_vec().into())).await.is_err() { break; }
                }
            }
        }
    }
    let _ = websocket_sink.close().await;
}

pub(super) async fn docker_console_session(
    socket: WebSocket,
    mut process: tokio::process::Child,
    runtime: DockerRuntime,
    container_id: String,
    resize_container_tty: bool,
) {
    let (Some(mut stdin), Some(mut stdout), Some(mut stderr)) = (
        process.stdin.take(),
        process.stdout.take(),
        process.stderr.take(),
    ) else {
        return;
    };
    let (mut websocket_sink, mut websocket_stream) = socket.split();
    let mut stdout_open = true;
    let mut stderr_open = true;
    let mut stdout_buffer = [0_u8; 16 * 1024];
    let mut stderr_buffer = [0_u8; 16 * 1024];

    loop {
        tokio::select! {
            incoming = websocket_stream.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    if resize_container_tty
                        && let Some((rows, columns)) = parse_docker_resize_control(text.as_str())
                    {
                        if let Err(error) = runtime.resize_tty(&container_id, rows, columns).await {
                            tracing::warn!(container_id, error = %error.message(), "Could not resize Docker console TTY");
                        }
                        continue;
                    }
                    if stdin.write_all(text.as_bytes()).await.is_err() { break; }
                }
                Some(Ok(Message::Binary(bytes))) => {
                    if stdin.write_all(&bytes).await.is_err() { break; }
                }
                Some(Ok(Message::Ping(payload))) => {
                    if websocket_sink.send(Message::Pong(payload)).await.is_err() { break; }
                }
                Some(Ok(Message::Pong(_))) => {}
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
            },
            received = stdout.read(&mut stdout_buffer), if stdout_open => match received {
                Ok(0) | Err(_) => stdout_open = false,
                Ok(length) => {
                    if websocket_sink.send(Message::Binary(stdout_buffer[..length].to_vec().into())).await.is_err() { break; }
                }
            },
            received = stderr.read(&mut stderr_buffer), if stderr_open => match received {
                Ok(0) | Err(_) => stderr_open = false,
                Ok(length) => {
                    if websocket_sink.send(Message::Binary(stderr_buffer[..length].to_vec().into())).await.is_err() { break; }
                }
            },
            _ = process.wait(), if !stdout_open && !stderr_open => break,
        }
    }
    let _ = stdin.write_all(&[0x10, 0x11]).await;
    let _ = stdin.flush().await;
    if tokio::time::timeout(std::time::Duration::from_millis(500), process.wait())
        .await
        .is_err()
    {
        drop(stdin);
        let _ = process.kill().await;
        let _ = process.wait().await;
    }
    let _ = websocket_sink.close().await;
}

pub(super) fn parse_docker_resize_control(text: &str) -> Option<(u16, u16)> {
    let message: Value = serde_json::from_str(text).ok()?;
    if message.get("type")?.as_str()? != "resize" {
        return None;
    }
    let rows = u16::try_from(message.get("rows")?.as_u64()?).ok()?;
    let columns = u16::try_from(message.get("columns")?.as_u64()?).ok()?;
    (rows > 0 && columns > 0).then_some((rows, columns))
}
