use super::*;

pub(super) async fn create_link(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Json(mut link): Json<Value>,
) -> Result<impl IntoResponse, ApiError> {
    let project_id = project_id.to_string();
    let _project_lock = state.lock_project_operations(&project_id).await;
    let (link_id, link_type) = {
        let object = as_object_mut(&mut link, "link body must be an object")?;
        let link_id = parse_or_generate_uuid(object.get("link_id"), "link_id")?;
        let endpoints = object.get("nodes").and_then(Value::as_array)
            .ok_or_else(|| ApiError::bad_request("link must contain a nodes array"))?;
        if endpoints.len() != 2 {
        return Err(ApiError::bad_request(
            "GNS3 links connect exactly two endpoints; use a switch or hub for shared media",
        ));
        }
        (link_id, object.get("link_type").and_then(Value::as_str).unwrap_or("ethernet").to_owned())
    };
    resolve_link_port_names(&state, &project_id, &mut link).await?;
    validate_link_shape(link["nodes"].as_array().expect("validated node list"))?;
    if link_type != "ethernet" {
        return Err(ApiError::bad_request(
            "Only Ethernet links are supported by the active runtime",
        ));
    }
    let object = as_object_mut(&mut link, "link body must be an object")?;
    object.insert("link_id".into(), json!(link_id));
    object.insert("project_id".into(), json!(project_id));
    set_default(object, "link_type", json!("ethernet"));
    set_default(object, "suspend", json!(false));
    set_default(object, "filters", json!({}));
    set_default(object, "link_style", json!({}));
    object.insert("capturing".into(), json!(false));
    let filters = super::validate_link_filters(object.get("filters").unwrap_or(&Value::Null))?;
    object.insert("filters".into(), filters);
    if !object.get("suspend").is_some_and(Value::is_boolean) {
        return Err(ApiError::bad_request("suspend must be a boolean"));
    }
    if !object.get("link_style").is_some_and(Value::is_object) {
        return Err(ApiError::bad_request("link_style must be an object"));
    }
    object.insert("capture_file_name".into(), Value::Null);
    object.insert("capture_file_path".into(), Value::Null);
    object.insert("capture_endpoint_id".into(), Value::Null);
    object.insert("wireshark".into(), json!(false));
    if let Some(row) = sqlx::query(
        "SELECT object_json FROM topology_objects
         WHERE project_id = ? AND kind = 'link' AND object_id = ?",
    )
    .bind(&project_id)
    .bind(&link_id)
    .fetch_optional(state.store.pool())
    .await?
    {
        return Ok(Json(decode_json(row.get::<String, _>("object_json"))?).into_response());
    }
    let mut validation = state.store.pool().begin().await?;
    ensure_project_exists(&mut validation, &project_id).await?;
    validate_link_in_tx(&mut validation, &project_id, &link).await?;
    validation.rollback().await?;
    let runtime_endpoints =
        super::link_runtime::link_runtime_endpoints(state.store.pool(), &project_id, &link).await?;
    let endpoints = link["nodes"].as_array_mut().expect("validated nodes array");
    for (endpoint, runtime_endpoint) in endpoints.iter_mut().zip(&runtime_endpoints) {
        if let Some(object) = endpoint.as_object_mut() {
            object.retain(|key, _| !key.starts_with("runtime_"));
        }
        endpoint["node_type"] = runtime_endpoint["node_type"].clone();
    }
    let mut should_attach = Vec::with_capacity(runtime_endpoints.len());
    for runtime_endpoint in &runtime_endpoints {
        should_attach.push(
            super::link_runtime::runtime_endpoint_status(&state, &project_id, runtime_endpoint)
                .await?
                != "stopped",
        );
    }
    for endpoint in link["nodes"].as_array_mut().expect("validated nodes array") {
        if endpoint["node_type"].as_str() == Some("vpcs")
            && let Some(node_id) = endpoint["node_id"].as_str().map(str::to_owned)
            && let (Some(adapter_port), Some(runtime_port)) = (
                state.vpcs.adapter_udp_port(&node_id).await,
                state.vpcs.active_udp_port(&node_id).await,
            )
        {
            endpoint["runtime_adapter_port"] = json!(adapter_port);
            endpoint["runtime_udp_port"] = json!(runtime_port);
        }
    }
    let engine = LinkEngine::new(state.ubridge.clone());
    engine
        .reserve_pair(&mut link)
        .await
        .map_err(runtime_api_error)?;
    link["filter_endpoint_id"] = link["nodes"][0]["runtime_endpoint_id"].clone();
    for endpoint in link["nodes"].as_array_mut().expect("validated nodes array") {
        if let Some(object) = endpoint.as_object_mut() {
            object.remove("node_type");
        }
    }
    let mut attached = Vec::new();
    for ((endpoint, runtime_endpoint), should_attach) in link["nodes"]
        .as_array()
        .expect("validated nodes array")
        .iter()
        .zip(&runtime_endpoints)
        .zip(should_attach)
    {
        if should_attach {
            if let Err(error) = super::link_runtime::attach_runtime_link_endpoint(
                &state,
                &project_id,
                &link,
                endpoint,
                runtime_endpoint,
            )
            .await
            {
                for (attached_endpoint, attached_runtime) in attached.iter().rev() {
                    let _ = super::link_runtime::detach_runtime_link_endpoint(
                        &state,
                        &project_id,
                        attached_endpoint,
                        attached_runtime,
                    )
                    .await;
                }
                engine.release_link_ports(&link).await;
                return Err(error);
            }
            attached.push((endpoint.clone(), runtime_endpoint.clone()));
        }
    }
    match create_topology_object(state.clone(), &project_id, "link", "link_id", link.clone()).await
    {
        Ok(response) => Ok(response),
        Err(error) => {
            for endpoint in link["nodes"].as_array().expect("validated nodes array") {
                let node_id = endpoint["node_id"].as_str().unwrap_or_default();
                if let Some(runtime_endpoint) = runtime_endpoints
                    .iter()
                    .find(|candidate| candidate["node_id"].as_str() == Some(node_id))
                {
                    let _ = super::link_runtime::detach_runtime_link_endpoint(
                        &state,
                        &project_id,
                        endpoint,
                        runtime_endpoint,
                    )
                    .await;
                }
            }
            engine.release_link_ports(&link).await;
            Err(error)
        }
    }
}

async fn resolve_link_port_names(
    state: &AppState,
    project_id: &str,
    link: &mut Value,
) -> Result<(), ApiError> {
    let endpoints = link["nodes"].as_array_mut().ok_or_else(|| ApiError::bad_request("link must contain a nodes array"))?;
    for endpoint in endpoints {
        let Some(port) = endpoint.get("port_number").cloned() else { continue };
        let Some(name) = port.as_str() else { continue };
        let node_id = endpoint.get("node_id").and_then(Value::as_str).ok_or_else(|| ApiError::bad_request("each link endpoint requires node_id"))?;
        let row = sqlx::query("SELECT object_json FROM topology_objects WHERE project_id=? AND kind='node' AND object_id=?")
            .bind(project_id).bind(node_id).fetch_optional(state.store.pool()).await?
            .ok_or_else(|| ApiError::bad_request(format!("Node {node_id} does not exist in this project")))?;
        let node = decode_json(row.get("object_json"))?;
        let ports = node["ports_mapping"].as_array().cloned().unwrap_or_default();
        let adapter = endpoint.get("adapter_number").and_then(Value::as_u64).unwrap_or(0);
        let mut available = Vec::new();
        for (index, mapping) in ports.iter().enumerate() {
            let port_adapter = mapping.get("adapter_number").and_then(Value::as_u64).unwrap_or(0);
            let port_number = mapping.get("port_number").and_then(Value::as_u64).unwrap_or(index as u64);
            if port_adapter != adapter { continue; }
            let used: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM link_endpoints WHERE project_id=? AND node_id=? AND adapter_number=? AND port_number=?)")
                .bind(project_id).bind(node_id).bind(port_adapter as i64).bind(port_number as i64).fetch_one(state.store.pool()).await?;
            if used == 0 { available.push((port_number, mapping.get("name").and_then(Value::as_str).unwrap_or_default())); }
        }
        let selected = if name == "auto" { available.first().map(|entry| entry.0) }
            else { available.iter().find(|entry| entry.1 == name).map(|entry| entry.0) };
        let selected = selected.ok_or_else(|| ApiError::conflict(format!("No free port named '{name}' on node {node_id}")))?;
        endpoint["port_number"] = json!(selected);
    }
    Ok(())
}
