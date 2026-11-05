use super::*;
use axum::extract::ws::CloseFrame;

pub(crate) async fn qemu_vnc(
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

pub(crate) async fn docker_vnc(
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

pub(crate) async fn vnc_console_session(socket: WebSocket, stream: tokio::net::TcpStream) {
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


