use super::*;

#[derive(Deserialize)]
pub(super) struct LinkCaptureRequest {
    capture_file_name: Option<String>,
    data_link_type: Option<String>,
    #[serde(default)]
    wireshark: bool,
}

pub(super) async fn start_link_capture(
    State(state): State<AppState>,
    Path((project_id, link_id)): Path<(Uuid, Uuid)>,
    axum::Extension(principal): axum::Extension<crate::api::auth::Principal>,
    Json(request): Json<LinkCaptureRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if request.data_link_type.as_deref().unwrap_or("DLT_EN10MB") != "DLT_EN10MB" {
        return Err(ApiError::bad_request(
            "Ethernet link capture requires DLT_EN10MB",
        ));
    }
    let project_id = project_id.to_string();
    let link_id = link_id.to_string();
    let _project_lock = state.lock_project_operations(&project_id).await;
    let link = get_topology_object(state.store.pool(), &project_id, "link", &link_id)
        .await?
        .0;
    if link["capturing"].as_bool().unwrap_or(false) {
        return Err(ApiError::conflict("Link capture is already active"));
    }
    let file_name = safe_capture_file_name(
        request
            .capture_file_name
            .as_deref()
            .unwrap_or(&format!("{link_id}.pcap")),
    )?;
    let directory = capture_directory(&project_id);
    std::fs::create_dir_all(&directory).map_err(|error| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not create capture directory: {error}"),
        )
    })?;
    let path = canonical_capture_file_path(&project_id, &file_name, None, false)?;
    let path_text = path.to_string_lossy().into_owned();
    let runtimes = link_runtime_endpoints(state.store.pool(), &project_id, &link).await?;
    let mut selected = None;
    for endpoint in link["nodes"].as_array().into_iter().flatten() {
        let node_id = endpoint["node_id"].as_str().unwrap_or_default();
        let runtime = runtimes
            .iter()
            .find(|runtime| runtime["node_id"].as_str() == Some(node_id))
            .ok_or_else(|| ApiError::conflict("Link runtime endpoint is missing"))?;
        if runtime_endpoint_status(&state, &project_id, runtime).await? == "started" {
            selected = Some((endpoint.clone(), runtime.clone()));
            break;
        }
    }
    let (endpoint, runtime_endpoint) = selected
        .ok_or_else(|| ApiError::conflict("Start a node on this link before capturing packets"))?;
    attach_runtime_link_endpoint(&state, &project_id, &link, &endpoint, &runtime_endpoint).await?;
    LinkEngine::new(state.ubridge.clone())
        .start_capture(&endpoint, &path_text)
        .await
        .map_err(runtime_api_error)?;
    let mut patch = json!({
        "capturing": true,
        "capture_file_name": file_name,
        "capture_file_path": path_text,
        "capture_endpoint_id": endpoint["runtime_endpoint_id"],
        "wireshark": false
    });
    if request.wireshark {
        let path = format!("/v3/projects/{project_id}/links/{link_id}/capture/stream");
        let ticket = crate::api::auth::mint_capture_ticket(state.store.pool(), &principal, &path)
            .await
            .map_err(|error| ApiError::new(StatusCode::FORBIDDEN, error))?;
        let gateway = docker_gateway("172.31.0.0/22");
        let stream_url = format!(
            "http://{gateway}:{}/v3/projects/{project_id}/links/{link_id}/capture/stream?ticket={ticket}",
            state.bind_address.port()
        );
        let (_, _) = state
            .web_wireshark
            .start_session(&project_id, &link_id, &stream_url)
            .await
            .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error))?;
        let ws_url = format!(
            "ws://{}/v3/projects/{project_id}/links/{link_id}/capture/web-wireshark",
            state.bind_address
        );
        patch["wireshark"] = json!(true);
        let updated = update_topology_object(
            state.clone(),
            project_id.clone(),
            link_id.clone(),
            "link",
            "link_id",
            patch,
        )
        .await?;
        let mut tx = state.store.begin_write().await?;
        let event = crate::storage::Store::append_event(
            &mut tx,
            &project_id,
            "link.web_wireshark_started",
            &json!({"link_id":link_id,"ws_url":ws_url}),
        )
        .await?;
        tx.commit().await?;
        state.store.publish(event);
        return Ok((StatusCode::CREATED, updated).into_response());
    }
    let updated =
        update_topology_object(state, project_id, link_id, "link", "link_id", patch).await?;
    Ok((StatusCode::CREATED, updated).into_response())
}

pub(super) async fn stop_link_capture(
    State(state): State<AppState>,
    Path((project_id, link_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let project_id = project_id.to_string();
    let link_id = link_id.to_string();
    let _project_lock = state.lock_project_operations(&project_id).await;
    let link = get_topology_object(state.store.pool(), &project_id, "link", &link_id)
        .await?
        .0;
    if let Some(endpoint_id) = link["capture_endpoint_id"].as_str()
        && let Some(endpoint) = link["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|endpoint| endpoint["runtime_endpoint_id"].as_str() == Some(endpoint_id))
    {
        LinkEngine::new(state.ubridge.clone())
            .stop_capture(endpoint)
            .await
            .map_err(runtime_api_error)?;
    }
    if link["wireshark"].as_bool().unwrap_or(false) {
        let _ = state
            .web_wireshark
            .stop_session(&project_id, &link_id)
            .await;
    }
    let _ = update_topology_object(
        state,
        project_id,
        link_id,
        "link",
        "link_id",
        json!({ "capturing": false, "capture_endpoint_id": null, "wireshark": false }),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn stream_link_capture(
    State(state): State<AppState>,
    Path((project_id, link_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    link_capture_response(&state, &project_id.to_string(), &link_id.to_string(), true).await
}

pub(super) async fn download_link_capture(
    State(state): State<AppState>,
    Path((project_id, link_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    link_capture_response(&state, &project_id.to_string(), &link_id.to_string(), false).await
}

pub(super) async fn restart_web_wireshark(
    State(state): State<AppState>,
    Path((project_id, link_id)): Path<(Uuid, Uuid)>,
    axum::Extension(principal): axum::Extension<crate::api::auth::Principal>,
) -> Result<Json<Value>, ApiError> {
    let project_id = project_id.to_string();
    let link_id = link_id.to_string();
    let link = get_topology_object(state.store.pool(), &project_id, "link", &link_id)
        .await?
        .0;
    if !link["capturing"].as_bool().unwrap_or(false) {
        return Err(ApiError::conflict("This link has no active packet capture"));
    }
    let file = link["capture_file_name"]
        .as_str()
        .ok_or_else(|| ApiError::not_found("Link has no capture file name"))?;
    let path = link["capture_file_path"]
        .as_str()
        .ok_or_else(|| ApiError::not_found("Link has no capture file"))?;
    let capture_path = canonical_capture_file_path(&project_id, file, Some(path), false)?;
    let stream = format!("/v3/projects/{project_id}/links/{link_id}/capture/stream");
    let ticket = crate::api::auth::mint_capture_ticket(state.store.pool(), &principal, &stream)
        .await
        .map_err(|e| ApiError::new(StatusCode::FORBIDDEN, e))?;
    let gateway = docker_gateway("172.31.0.0/22");
    let stream_url = format!(
        "http://{gateway}:{}/v3/projects/{project_id}/links/{link_id}/capture/stream?ticket={ticket}",
        state.bind_address.port()
    );
    let (_, _) = state
        .web_wireshark
        .start_session(&project_id, &link_id, &stream_url)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let ws_url = format!(
        "ws://{}/v3/projects/{project_id}/links/{link_id}/capture/web-wireshark",
        state.bind_address
    );
    let _ = capture_path;
    Ok(Json(json!({"ws_url":ws_url})))
}

pub(super) async fn web_wireshark_websocket(
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

async fn proxy_websocket(client: WebSocket, url: String) {
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

fn docker_gateway(cidr: &str) -> String {
    let ip = cidr
        .split('/')
        .next()
        .unwrap_or("172.31.0.0")
        .parse::<std::net::Ipv4Addr>()
        .unwrap_or(std::net::Ipv4Addr::new(172, 31, 0, 0));
    let octets = ip.octets();
    format!(
        "{}.{}.{}.{}",
        octets[0],
        octets[1],
        octets[2],
        octets[3].saturating_add(1)
    )
}

pub(super) async fn link_capture_response(
    state: &AppState,
    project_id: &str,
    link_id: &str,
    require_active: bool,
) -> Result<Response, ApiError> {
    let link = get_topology_object(state.store.pool(), project_id, "link", link_id)
        .await?
        .0;
    if require_active && !link["capturing"].as_bool().unwrap_or(false) {
        return Err(ApiError::conflict("This link has no active packet capture"));
    }
    let file_name = safe_capture_file_name(
        link["capture_file_name"]
            .as_str()
            .ok_or_else(|| ApiError::not_found("Link has no capture file name"))?,
    )?;
    let stored_path = link["capture_file_path"]
        .as_str()
        .ok_or_else(|| ApiError::not_found("Link has no capture file"))?;
    let path =
        canonical_capture_file_path(project_id, &file_name, Some(stored_path), !require_active)?;
    let body = capture_file_body(
        path,
        require_active.then(|| {
            (
                state.store.pool().clone(),
                project_id.to_owned(),
                link_id.to_owned(),
            )
        }),
    );
    let mut response = body.into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        "application/vnd.tcpdump.pcap"
            .parse()
            .expect("valid media type"),
    );
    if !require_active && let Ok(value) = format!("attachment; filename=\"{file_name}\"").parse() {
        response
            .headers_mut()
            .insert(axum::http::header::CONTENT_DISPOSITION, value);
    }
    Ok(response)
}

pub(super) fn capture_file_body(
    path: PathBuf,
    follow: Option<(SqlitePool, String, String)>,
) -> Body {
    let stream = futures_util::stream::unfold(
        (None::<tokio::fs::File>, false),
        move |(mut file, finished)| {
            let path = path.clone();
            let follow = follow.clone();
            async move {
                if finished {
                    return None;
                }
                loop {
                    if file.is_none() {
                        match tokio::fs::File::open(&path).await {
                            Ok(opened) => file = Some(opened),
                            Err(error)
                                if error.kind() == std::io::ErrorKind::NotFound
                                    && follow.is_some() =>
                            {
                                match capture_is_active(
                                    follow.as_ref().expect("checked capture state"),
                                )
                                .await
                                {
                                    Ok(true) => sleep(Duration::from_millis(100)).await,
                                    Ok(false) => return None,
                                    Err(error) => return Some((Err(error), (None, true))),
                                }
                                continue;
                            }
                            Err(error) => return Some((Err(error), (None, true))),
                        }
                    }
                    let mut buffer = vec![0_u8; 16 * 1024];
                    match file
                        .as_mut()
                        .expect("capture file was opened")
                        .read(&mut buffer)
                        .await
                    {
                        Ok(0) => match follow.as_ref() {
                            Some(capture) => match capture_is_active(capture).await {
                                Ok(true) => sleep(Duration::from_millis(100)).await,
                                Ok(false) => return None,
                                Err(error) => return Some((Err(error), (file, true))),
                            },
                            None => return None,
                        },
                        Ok(read) => {
                            buffer.truncate(read);
                            return Some((Ok(Bytes::from(buffer)), (file, false)));
                        }
                        Err(error) => return Some((Err(error), (file, true))),
                    }
                }
            }
        },
    );
    Body::from_stream(stream)
}

pub(super) async fn capture_is_active(
    (pool, project_id, link_id): &(SqlitePool, String, String),
) -> Result<bool, std::io::Error> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(json_extract(object_json, '$.capturing'), 0)
         FROM topology_objects WHERE project_id = ? AND kind = 'link' AND object_id = ?",
    )
    .bind(project_id)
    .bind(link_id)
    .fetch_optional(pool)
    .await
    .map(|capturing| capturing == Some(1))
    .map_err(std::io::Error::other)
}

pub(super) fn capture_directory(project_id: &str) -> std::path::PathBuf {
    std::env::var_os("NETWORK_SIMULATOR_CAPTURE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("data/captures"))
        .join(project_id)
}

pub(super) fn canonical_capture_file_path(
    project_id: &str,
    file_name: &str,
    stored_path: Option<&str>,
    require_exists: bool,
) -> Result<PathBuf, ApiError> {
    canonical_capture_file_path_in(
        &capture_directory(project_id),
        file_name,
        stored_path,
        require_exists,
    )
}

pub(super) fn canonical_capture_file_path_in(
    directory: &FsPath,
    file_name: &str,
    stored_path: Option<&str>,
    require_exists: bool,
) -> Result<PathBuf, ApiError> {
    let file_name = safe_capture_file_name(file_name)?;
    let directory = directory
        .canonicalize()
        .map_err(|_| ApiError::not_found("Project capture directory is not available"))?;
    let expected = directory.join(&file_name);
    let candidate = stored_path
        .map(PathBuf::from)
        .unwrap_or_else(|| expected.clone());
    let canonical = match candidate.canonicalize() {
        Ok(path) => path,
        Err(error)
            if !require_exists
                && error.kind() == std::io::ErrorKind::NotFound
                && std::fs::symlink_metadata(&candidate).is_err() =>
        {
            let parent = candidate
                .parent()
                .and_then(|parent| parent.canonicalize().ok())
                .ok_or_else(|| ApiError::not_found("Capture file path is not available"))?;
            parent.join(
                candidate
                    .file_name()
                    .ok_or_else(|| ApiError::not_found("Capture file path is invalid"))?,
            )
        }
        Err(_) => return Err(ApiError::not_found("Capture file is not available")),
    };
    if canonical != expected || !canonical.starts_with(&directory) {
        return Err(ApiError::not_found(
            "Capture file path is outside the project capture directory",
        ));
    }
    if require_exists && !canonical.is_file() {
        return Err(ApiError::not_found("Capture file is not available"));
    }
    Ok(canonical)
}

pub(super) fn safe_capture_file_name(file_name: &str) -> Result<String, ApiError> {
    let path = FsPath::new(file_name);
    if file_name.is_empty()
        || file_name.len() > 255
        || path.file_name().and_then(|name| name.to_str()) != Some(file_name)
        || file_name == "."
        || file_name == ".."
        || file_name.chars().any(char::is_control)
    {
        return Err(ApiError::bad_request(
            "capture_file_name must be a simple file name",
        ));
    }
    Ok(file_name.to_owned())
}

pub(super) async fn delete_link(
    State(state): State<AppState>,
    Path((project_id, link_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let project_id = project_id.to_string();
    let link_id = link_id.to_string();
    let _project_lock = state.lock_project_operations(&project_id).await;
    let link = get_topology_object(state.store.pool(), &project_id, "link", &link_id)
        .await?
        .0;
    cleanup_link_runtime(&state, &project_id, &link).await?;
    let response = delete_topology_object(state.clone(), project_id, link_id, "link").await?;
    LinkEngine::new(state.ubridge.clone())
        .release_link_ports(&link)
        .await;
    Ok(response)
}
