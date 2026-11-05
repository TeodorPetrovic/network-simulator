use super::*;
use axum::extract::ws::CloseFrame;

pub(super) async fn qemu_vnc(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let project_id = project_id.to_string();
    let node_id = node_id.to_string();
    let node = get_topology_object(state.store.pool(), &project_id, "node", &node_id)
        .await?
        .0;
    if node["node_type"] != "qemu" || node["console_type"] != "vnc" {
        return Err(ApiError::conflict(
            "A VNC console is available only for QEMU nodes configured with VNC",
        ));
    }
    let vm = state.qemu.vm_from_node(&node).map_err(runtime_api_error)?;
    if state.qemu.status(&vm).await.map_err(runtime_api_error)? != "started" {
        return Ok(stopped_console(websocket));
    }
    let port = vm
        .console_port
        .ok_or_else(|| ApiError::conflict("QEMU VNC port is not configured"))?;
    let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(|error| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("Could not connect to QEMU VNC console: {error}"),
            )
        })?;
    let activity = state.project_activity.clone();
    Ok(websocket.on_upgrade(move |socket| async move {
        let _activity = activity.open_websocket(project_id);
        vnc_console_session(socket, stream).await;
    }))
}

pub(super) async fn docker_vnc(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let project_id = project_id.to_string();
    let node_id = node_id.to_string();
    let node = get_topology_object(state.store.pool(), &project_id, "node", &node_id)
        .await?
        .0;
    if node["node_type"] != "docker" || node["console_type"] != "vnc" {
        return Err(ApiError::conflict(
            "A VNC console is available only for Docker nodes configured with VNC",
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
        return Ok(stopped_console(websocket));
    }
    let port = state
        .runtime
        .console_port(container_id)
        .await
        .ok_or_else(|| ApiError::conflict("Docker VNC port is not configured"))?;
    let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(|error| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("Could not connect to Docker VNC console: {error}"),
            )
        })?;
    let activity = state.project_activity.clone();
    Ok(websocket.on_upgrade(move |socket| async move {
        let _activity = activity.open_websocket(project_id);
        vnc_console_session(socket, stream).await;
    }))
}

async fn vnc_console_session(socket: WebSocket, stream: tokio::net::TcpStream) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (mut sink, mut source) = socket.split();
    let (mut reader, mut writer) = stream.into_split();
    let client_to_vnc = async {
        while let Some(Ok(message)) = source.next().await {
            match message {
                Message::Binary(bytes) => {
                    if writer.write_all(&bytes).await.is_err() {
                        break;
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }
    };
    let vnc_to_client = async {
        let mut buffer = vec![0u8; 16 * 1024];
        loop {
            let count = match reader.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(count) => count,
            };
            if sink
                .send(Message::Binary(buffer[..count].to_vec().into()))
                .await
                .is_err()
            {
                break;
            }
        }
    };
    tokio::select! { _ = client_to_vnc => {}, _ = vnc_to_client => {} }
}

#[cfg(test)]
mod vnc_proxy_tests {
    use super::*;

    async fn proxy_upgrade(
        websocket: WebSocketUpgrade,
        State(address): State<std::net::SocketAddr>,
    ) -> Result<Response, StatusCode> {
        let stream = tokio::net::TcpStream::connect(address)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
        Ok(websocket.on_upgrade(move |socket| vnc_console_session(socket, stream)))
    }

    #[tokio::test]
    async fn vnc_websocket_forwards_binary_frames_both_ways() {
        use futures_util::{SinkExt, StreamExt};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio_tungstenite::tungstenite::Message as ClientMessage;

        let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_address = target.local_addr().unwrap();
        let echo = tokio::spawn(async move {
            let (mut stream, _) = target.accept().await.unwrap();
            let mut bytes = [0; 4];
            stream.read_exact(&mut bytes).await.unwrap();
            assert_eq!(&bytes, &[0, 1, 2, 255]);
            stream.write_all(&[4, 3, 2, 1]).await.unwrap();
        });

        let proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_address = proxy.local_addr().unwrap();
        let app = Router::new()
            .route("/", get(proxy_upgrade))
            .with_state(target_address);
        tokio::spawn(async move {
            axum::serve(proxy, app).await.unwrap();
        });

        let (mut websocket, _) = tokio_tungstenite::connect_async(format!("ws://{proxy_address}/"))
            .await
            .unwrap();
        websocket
            .send(ClientMessage::Binary(vec![0, 1, 2, 255].into()))
            .await
            .unwrap();
        let response = tokio::time::timeout(std::time::Duration::from_secs(2), websocket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(response.into_data().as_ref(), &[4, 3, 2, 1]);
        echo.await.unwrap();
    }
}


enum ConsoleStream {
    Tcp(TcpStream),
    Unix(UnixStream),
}

pub(super) async fn controller_telnet_console(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let project_id = project_id.to_string();
    let node_id = node_id.to_string();
    let node = get_topology_object(state.store.pool(), &project_id, "node", &node_id)
        .await?
        .0;
    if node["console_type"].as_str().unwrap_or("telnet") != "telnet" {
        return Err(ApiError::conflict(
            "Controller console proxy supports telnet consoles only",
        ));
    }
    let console = match node["node_type"].as_str().unwrap_or_default() {
        "qemu" => {
            let vm = state.qemu.vm_from_node(&node).map_err(runtime_api_error)?;
            if state.qemu.status(&vm).await.map_err(runtime_api_error)? != "started" {
                return Ok(stopped_console(websocket));
            }
            let port = vm
                .console_port
                .ok_or_else(|| ApiError::conflict("QEMU telnet console has no port"))?;
            TcpStream::connect(("127.0.0.1", port))
                .await
                .map_err(|error| {
                    ApiError::new(
                        StatusCode::SERVICE_UNAVAILABLE,
                        format!("Could not connect to the QEMU telnet console: {error}"),
                    )
                })?
        }
        "vpcs" => {
            if state
                .vpcs
                .status(&node_id)
                .await
                .map_err(runtime_api_error)?
                != "started"
            {
                return Ok(stopped_console(websocket));
            }
            state
                .vpcs
                .connect_console(&node_id)
                .await
                .map_err(runtime_api_error)?
        }
        "dynamips" => {
            let vm = state
                .dynamips
                .vm_from_node(&node)
                .map_err(runtime_api_error)?;
            state
                .dynamips
                .console(&vm)
                .await
                .map_err(runtime_api_error)?
        }
        _ => {
            return Err(ApiError::conflict(
                "Telnet console proxy is unavailable for this node type",
            ));
        }
    };
    let activity = state.project_activity.clone();
    Ok(websocket.on_upgrade(move |socket| async move {
        let _activity = activity.open_websocket(project_id);
        vpcs_console_session(socket, console).await;
    }))
}

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
        return Ok(stopped_console(websocket));
    }
    let serial = if vm.console_type == "telnet" {
        let port = vm
            .console_port
            .ok_or_else(|| ApiError::conflict("QEMU telnet console has no port"))?;
        ConsoleStream::Tcp(
            TcpStream::connect(("127.0.0.1", port))
                .await
                .map_err(|error| {
                    ApiError::new(
                        StatusCode::SERVICE_UNAVAILABLE,
                        format!("Could not connect to the QEMU telnet console: {error}"),
                    )
                })?,
        )
    } else {
        ConsoleStream::Unix(
            UnixStream::connect(&vm.serial_socket)
                .await
                .map_err(|error| {
                    ApiError::new(
                        StatusCode::SERVICE_UNAVAILABLE,
                        format!("Could not connect to the QEMU serial console: {error}"),
                    )
                })?,
        )
    };
    let activity = state.project_activity.clone();
    Ok(websocket.on_upgrade(move |socket| async move {
        let _activity = activity.open_websocket(project_id);
        match serial {
            ConsoleStream::Tcp(stream) => vpcs_console_session(socket, stream).await,
            ConsoleStream::Unix(stream) => qemu_console_session(socket, stream).await,
        }
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
        return Ok(stopped_console(websocket));
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
        return Ok(stopped_console(websocket));
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
        // GNS3 wraps both types around VPCS's internal Telnet stream. This route
        // bridges the browser terminal directly to that negotiated stream.
        "telnet" | "ssh" => {}
        _ => {
            return Err(ApiError::bad_request(
                "Unsupported VPCS console_type; expected telnet, ssh, or none",
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
        return Ok(stopped_console(websocket));
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

fn stopped_console(websocket: WebSocketUpgrade) -> Response {
    websocket.on_upgrade(|mut socket| async move {
        let _ = socket
            .send(Message::Text("node is not started".into()))
            .await;
        let _ = socket
            .send(Message::Close(Some(CloseFrame {
                code: 1008,
                reason: "node is not started".into(),
            })))
            .await;
    })
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
