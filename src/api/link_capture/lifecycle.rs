use super::*;

#[derive(Deserialize)]
pub(crate) struct LinkCaptureRequest {
    capture_file_name: Option<String>,
    data_link_type: Option<String>,
    #[serde(default)]
    wireshark: bool,
}

pub(crate) async fn start_link_capture(
    State(state): State<AppState>,
    Path((project_id, link_id)): Path<(Uuid, Uuid)>,
    _principal: axum::Extension<crate::api::auth::Principal>,
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
    if request.wireshark {
        configure_web_wireshark(&state).await?;
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
        update_topology_object(
            state.clone(),
            project_id.clone(),
            link_id.clone(),
            "link",
            "link_id",
            patch.clone(),
        )
        .await?;
        let session_result = state
            .web_wireshark
            .start_session(
                &project_id,
                &link_id,
                path.clone(),
                state.store.pool().clone(),
            )
            .await;
        if let Err(error) = session_result {
            let _ = LinkEngine::new(state.ubridge.clone())
                .stop_capture(&endpoint)
                .await;
            let _ = update_topology_object(
                state.clone(),
                project_id,
                link_id,
                "link",
                "link_id",
                json!({"capturing":false,"capture_endpoint_id":null}),
            )
            .await;
            return Err(ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error));
        }
        let ws_path = format!("/v3/projects/{project_id}/links/{link_id}/capture/web-wireshark");
        patch["wireshark"] = json!(true);
        let updated = update_topology_object(
            state.clone(),
            project_id.clone(),
            link_id.clone(),
            "link",
            "link_id",
            json!({"wireshark":true,"ws_path":ws_path.clone()}),
        )
        .await?;
        let mut tx = state.store.begin_write().await?;
        let event = crate::storage::Store::append_event(
            &mut tx,
            &project_id,
            "link.web_wireshark_started",
            &json!({"link_id":link_id,"ws_path":ws_path}),
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

pub(crate) async fn stop_link_capture(
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
        json!({ "capturing": false, "capture_endpoint_id": null, "wireshark": false, "ws_path": null }),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn stream_link_capture(
    State(state): State<AppState>,
    Path((project_id, link_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    link_capture_response(&state, &project_id.to_string(), &link_id.to_string(), true).await
}

pub(crate) async fn download_link_capture(
    State(state): State<AppState>,
    Path((project_id, link_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    link_capture_response(&state, &project_id.to_string(), &link_id.to_string(), false).await
}

