use super::*;

pub(crate) async fn restart_web_wireshark(
    State(state): State<AppState>,
    Path((project_id, link_id)): Path<(Uuid, Uuid)>,
    _principal: axum::Extension<crate::api::auth::Principal>,
) -> Result<Json<Value>, ApiError> {
    let project_id = project_id.to_string();
    let link_id = link_id.to_string();
    let link = get_topology_object(state.store.pool(), &project_id, "link", &link_id)
        .await?
        .0;
    if !link["capturing"].as_bool().unwrap_or(false) {
        return Err(ApiError::conflict("This link has no active packet capture"));
    }
    configure_web_wireshark(&state).await?;
    let file = link["capture_file_name"]
        .as_str()
        .ok_or_else(|| ApiError::not_found("Link has no capture file name"))?;
    let path = link["capture_file_path"]
        .as_str()
        .ok_or_else(|| ApiError::not_found("Link has no capture file"))?;
    let capture_path = canonical_capture_file_path(&project_id, file, Some(path), false)?;
    state
        .web_wireshark
        .restart_feeder(
            &project_id,
            &link_id,
            capture_path,
            state.store.pool().clone(),
        )
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let ws_path = format!("/v3/projects/{project_id}/links/{link_id}/capture/web-wireshark");
    Ok(Json(json!({"ws_path":ws_path})))
}

pub(crate) async fn web_wireshark_websocket(
    State(state): State<AppState>,
    Path((project_id, link_id)): Path<(Uuid, Uuid)>,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let project_id = project_id.to_string();
    let link_id = link_id.to_string();
    let link = get_topology_object(state.store.pool(), &project_id, "link", &link_id)
        .await?
        .0;
    if !link["wireshark"].as_bool().unwrap_or(false) {
        return Err(ApiError::not_found(
            "Web Wireshark is not active for this link",
        ));
    }
    let session = state
        .web_wireshark
        .session(&project_id, &link_id)
        .ok_or_else(|| ApiError::not_found("Web Wireshark session does not exist"))?;
    let ip = state
        .web_wireshark
        .container_ip(&project_id)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e))?;
    let url = format!("ws://{ip}:{}", session.port);
    Ok(websocket
        .protocols(["binary"])
        .on_upgrade(move |client| async move { proxy_websocket(client, url).await }))
}

pub(crate) async fn proxy_websocket(client: WebSocket, url: String) {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let Ok(mut request) = url.into_client_request() else {
        return;
    };
    request
        .headers_mut()
        .insert("sec-websocket-protocol", "binary".parse().unwrap());
    let Ok((target, _)) = tokio_tungstenite::connect_async(request).await else {
        return;
    };
    let (mut tx, mut rx) = client.split();
    let (mut target_tx, mut target_rx) = target.split();
    let a = async {
        while let Some(Ok(message)) = rx.next().await {
            match message {
                Message::Binary(bytes) => {
                    if target_tx
                        .send(tokio_tungstenite::tungstenite::Message::Binary(bytes))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Message::Close(_) => {
                    let _ = target_tx
                        .send(tokio_tungstenite::tungstenite::Message::Close(None))
                        .await;
                    break;
                }
                _ => {}
            }
        }
    };
    let b = async {
        while let Some(Ok(message)) = target_rx.next().await {
            match message {
                tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                    if tx.send(Message::Binary(bytes)).await.is_err() {
                        break;
                    }
                }
                tokio_tungstenite::tungstenite::Message::Close(_) => {
                    let _ = tx.send(Message::Close(None)).await;
                    break;
                }
                _ => {}
            }
        }
    };
    tokio::select! { _ = a => {}, _ = b => {} }
}

pub(crate) async fn configure_web_wireshark(state: &AppState) -> Result<String, ApiError> {
    let stored: String =
        sqlx::query_scalar("SELECT settings_json FROM controller_settings WHERE singleton = 1")
            .fetch_one(state.store.pool())
            .await?;
    let settings = decode_json(stored)?;
    let section = settings
        .get("WebWireshark")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if section.get("enabled").and_then(Value::as_bool) == Some(false) {
        return Err(ApiError::new(
            StatusCode::NOT_IMPLEMENTED,
            "Web Wireshark is disabled in controller settings",
        ));
    }
    let image = section
        .get("image")
        .and_then(Value::as_str)
        .unwrap_or("gns3/web-wireshark:latest")
        .to_owned();
    let subnet = section
        .get("network_subnet")
        .and_then(Value::as_str)
        .unwrap_or("172.31.0.0/22")
        .to_owned();
    let memory = section
        .get("memory")
        .and_then(Value::as_str)
        .unwrap_or("2g")
        .to_owned();
    let cpus = section
        .get("cpus")
        .map(|v| v.to_string())
        .unwrap_or_else(|| "1.0".to_owned());
    let pids_limit = section
        .get("pids_limit")
        .map(|v| v.to_string())
        .unwrap_or_else(|| "1000".to_owned());
    state
        .web_wireshark
        .configure(image, subnet.clone(), memory, cpus, pids_limit);
    Ok(subnet)
}

