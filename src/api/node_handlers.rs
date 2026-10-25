use super::*;

pub(super) async fn get_node(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    let project_id = project_id.to_string();
    let node = get_topology_object(
        state.store.pool(),
        &project_id,
        "node",
        &node_id.to_string(),
    )
    .await?
    .0;
    let mut nodes = refresh_node_statuses(&state, &project_id, vec![node]).await;
    Ok(etag_response(nodes.pop().expect("the requested node was provided")))
}

pub(super) async fn update_node(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
    headers: axum::http::HeaderMap,
    Json(patch): Json<Value>,
) -> Result<Response, ApiError> {
    let current = get_topology_object(state.store.pool(), &project_id.to_string(), "node", &node_id.to_string()).await?.0;
    let current = refresh_node_statuses(&state, &project_id.to_string(), vec![current]).await.pop().expect("one requested node was supplied");
    check_if_match(&headers, &current)?;
    if let Some(idle_pc) = patch.get("idlepc").and_then(Value::as_str) {
        let node = get_topology_object(
            state.store.pool(),
            &project_id.to_string(),
            "node",
            &node_id.to_string(),
        )
        .await?
        .0;
        if node["node_type"].as_str() == Some("dynamips") && !idle_pc.is_empty() {
            let vm = state
                .dynamips
                .vm_from_node(&node)
                .map_err(runtime_api_error)?;
            state
                .dynamips
                .set_idle_pc(&vm, idle_pc)
                .await
                .map_err(runtime_api_error)?;
        }
    }
    let updated = update_topology_object(
        state,
        project_id.to_string(),
        node_id.to_string(),
        "node",
        "node_id",
        patch,
    )
    .await?;
    Ok(etag_response(updated.0))
}

pub(super) async fn update_node_configuration(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
    Json(patch): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let project_id = project_id.to_string();
    let node_id = node_id.to_string();
    let _project_lock = state.lock_project_operations(&project_id).await;
    let node = get_topology_object(state.store.pool(), &project_id, "node", &node_id)
        .await?
        .0;
    let node_type = node["node_type"].as_str().unwrap_or_default().to_owned();
    if BuiltinRuntime::is_type(&node_type) {
        let patch = BuiltinRuntime::normalize_configuration_patch(&node, patch)
            .map_err(ApiError::bad_request)?;
        let mut updated = node.clone();
        for (key, value) in patch.as_object().expect("normalized patch is an object") {
            updated[key] = value.clone();
        }
        updated = BuiltinRuntime::normalize_node(updated).map_err(ApiError::bad_request)?;
        BuiltinRuntime::validate_host_interfaces(&updated).map_err(ApiError::bad_request)?;
        validate_builtin_configuration_links(
            state.store.pool(),
            &project_id,
            &node_id,
            &node,
            &updated,
        )
        .await?;

        let rewire_cloud =
            node_type == "cloud" && node["ports_mapping"] != updated["ports_mapping"];
        if rewire_cloud {
            detach_node_links(&state, &project_id, &node_id).await?;
        }
        let persisted = persist_node_configuration(
            state.clone(),
            &project_id,
            &node_id,
            patch.clone(),
            Map::new(),
        )
        .await?;
        let updated = persisted.0;
        if node_type == "ethernet_switch" {
            state.builtin.update_switch_configuration(&updated).await;
        }
        if rewire_cloud
            && let Err(error) = attach_node_links(&state, &project_id, &node_id, false).await
        {
            let _ = detach_node_links(&state, &project_id, &node_id).await;
            let mut restore = Map::new();
            for key in patch
                .as_object()
                .expect("normalized patch is an object")
                .keys()
            {
                if let Some(value) = node.get(key) {
                    restore.insert(key.clone(), value.clone());
                }
            }
            let _ = persist_node_configuration(
                state.clone(),
                &project_id,
                &node_id,
                Value::Object(restore),
                Map::new(),
            )
            .await;
            let _ = attach_node_links(&state, &project_id, &node_id, false).await;
            return Err(error);
        }
        return Ok(Json(updated));
    }
    let patch = validate_node_configuration_patch(&node_type, patch)?;
    let current_status = match node_type.as_str() {
        "qemu" => {
            let vm = state.qemu.vm_from_node(&node).map_err(runtime_api_error)?;
            state.qemu.status(&vm).await.map_err(runtime_api_error)?
        }
        "docker" => {
            let container_id = node["container_id"]
                .as_str()
                .ok_or_else(|| ApiError::conflict("Docker node has no runtime container"))?;
            state
                .runtime
                .container_status(container_id)
                .await
                .map_err(runtime_api_error)?
        }
        "vpcs" => {
            let node_id = node["node_id"]
                .as_str()
                .ok_or_else(|| ApiError::conflict("VPCS node has no node ID"))?;
            state
                .vpcs
                .status(node_id)
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
                .status(&vm)
                .await
                .map_err(runtime_api_error)?
        }
        _ => {
            return Err(ApiError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                "This runtime does not expose editable per-device settings",
            ));
        }
    };
    if current_status != "stopped" {
        return Err(ApiError::conflict(
            "Stop this device before changing runtime settings",
        ));
    }
    if let Some(adapters) = patch["adapters"].as_u64() {
        validate_adapter_count_against_links(
            state.store.pool(),
            &project_id,
            &node_id,
            &node_type,
            adapters,
        )
        .await?;
    }

    let mut updated = node;
    for (key, value) in patch.as_object().expect("validated patch is an object") {
        updated[key] = value.clone();
    }
    let mut extra_fields = Map::new();
    if node_type == "docker" {
        validate_docker_extra_configs_api(
            updated.get("extra_configs"),
            updated
                .get("startup_config_content")
                .and_then(Value::as_str),
        )?;
        detach_node_links(&state, &project_id, &node_id).await?;
        let options = DockerTemplateOptions {
            environment: updated["environment"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            extra_hosts: updated["extra_hosts"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            extra_volumes: updated["extra_volumes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            extra_configs: validate_docker_extra_configs_api(
                updated.get("extra_configs"),
                updated
                    .get("startup_config_content")
                    .and_then(Value::as_str),
            )?,
            startup_config_content: updated
                .get("startup_config_content")
                .and_then(Value::as_str)
                .map(str::to_owned),
            console_type: updated["console_type"]
                .as_str()
                .unwrap_or("telnet")
                .to_owned(),
            console_resolution: updated["console_resolution"]
                .as_str()
                .unwrap_or("1024x768")
                .to_owned(),
            console_http_port: updated["console_http_port"]
                .as_u64()
                .and_then(|port| u16::try_from(port).ok()),
            console_http_path: updated["console_http_path"].as_str().map(str::to_owned),
            aux_type: updated["aux_type"].as_str().unwrap_or("none").to_owned(),
            custom_adapters: serde_json::from_value(updated["custom_adapters"].clone()).map_err(
                |_| {
                    ApiError::bad_request("custom_adapters must be an array of adapter definitions")
                },
            )?,
            ..DockerTemplateOptions::default()
        };
        let old_container_id = updated["container_id"]
            .as_str()
            .ok_or_else(|| ApiError::conflict("Docker node has no runtime container"))?;
        let (container_id, image_digest) = state
            .runtime
            .reconfigure_container(
                &project_id,
                &node_id,
                old_container_id,
                updated["image"].as_str().unwrap_or_default(),
                updated["name"].as_str().unwrap_or("Docker node"),
                updated["start_command"].as_str(),
                updated["memory"].as_u64(),
                updated["cpus"].as_f64(),
                &options,
            )
            .await
            .map_err(runtime_api_error)?;
        extra_fields.insert("container_id".to_owned(), json!(container_id));
        extra_fields.insert("image_digest".to_owned(), json!(image_digest));
        extra_fields.insert(
            "console".to_owned(),
            json!(state.runtime.console_port(&container_id).await),
        );
    }
    persist_node_configuration(state, &project_id, &node_id, patch, extra_fields).await
}
