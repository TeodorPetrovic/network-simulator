use super::*;

pub(super) async fn create_docker_node(
    state: AppState,
    project_id: String,
    mut node: Value,
) -> Result<Response, ApiError> {
    let object = as_object_mut(&mut node, "node body must be an object")?;
    let name = required_string(object, "name")?.to_owned();
    let node_type = required_string(object, "node_type")?;
    if node_type != "docker" {
        return Err(ApiError::bad_request(format!(
            "Unsupported Docker node type '{node_type}'"
        )));
    }
    let image = required_string(object, "image")?.to_owned();
    let start_command = object
        .get("start_command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .map(str::to_owned);
    if let Some(start_command) = start_command.as_deref() {
        parse_start_command(start_command)
            .map_err(|error| ApiError::bad_request(error.message()))?;
    }
    if object
        .get("compute_id")
        .and_then(Value::as_str)
        .is_some_and(|compute_id| compute_id != "local")
    {
        return Err(ApiError::bad_request(
            "This runtime build can execute nodes only on the local Docker compute",
        ));
    }
    let node_id = parse_or_generate_uuid(object.get("node_id"), "node_id")?;
    ensure_project_exists_pool(state.store.pool(), &project_id).await?;
    if let Some(row) = sqlx::query(
        "SELECT object_json FROM topology_objects
         WHERE project_id = ? AND kind = 'node' AND object_id = ?",
    )
    .bind(&project_id)
    .bind(&node_id)
    .fetch_optional(state.store.pool())
    .await?
    {
        return Ok(Json(decode_json(row.get::<String, _>("object_json"))?).into_response());
    }

    object.insert("name".into(), json!(name));
    object.insert("node_id".into(), json!(node_id));
    object.insert("project_id".into(), json!(project_id));
    object.insert("node_type".into(), json!("docker"));
    object.insert("image".into(), json!(image));
    set_default(object, "compute_id", json!("local"));
    set_default(object, "status", json!("stopped"));
    set_default(object, "x", json!(0));
    set_default(object, "y", json!(0));
    set_default(object, "z", json!(1));
    set_default(object, "locked", json!(false));
    set_default(object, "adapters", json!(1));
    set_default(object, "memory", json!(0));
    set_default(object, "cpus", json!(0));
    set_default(object, "properties", json!({}));
    set_default(object, "console_type", json!("telnet"));
    set_default(object, "console_resolution", json!("1024x768"));
    set_default(object, "console_http_port", json!(80));
    set_default(object, "console_http_path", json!("/"));
    set_default(object, "aux_type", json!("none"));
    set_default(object, "custom_adapters", json!([]));
    set_default(object, "extra_configs", json!([]));
    set_default(object, "startup_config_content", json!(""));

    let memory_mb = object
        .get("memory")
        .map(|value| {
            value
                .as_i64()
                .ok_or_else(|| ApiError::bad_request("memory must be an integer number of MB"))
        })
        .transpose()?
        .unwrap_or(0);
    if !(0..=1_048_576).contains(&memory_mb) {
        return Err(ApiError::bad_request(
            "memory must be between 0 and 1048576 MB",
        ));
    }
    let cpus = object
        .get("cpus")
        .map(|value| {
            value
                .as_f64()
                .ok_or_else(|| ApiError::bad_request("cpus must be a number"))
        })
        .transpose()?
        .unwrap_or(0.0);
    if !(0.0..=256.0).contains(&cpus) {
        return Err(ApiError::bad_request("cpus must be between 0 and 256"));
    }
    let adapters = object
        .get("adapters")
        .map(|value| {
            value
                .as_i64()
                .ok_or_else(|| ApiError::bad_request("adapters must be an integer"))
        })
        .transpose()?
        .unwrap_or(8);
    if !(1..=99).contains(&adapters) {
        return Err(ApiError::bad_request("adapters must be between 1 and 99"));
    }
    let docker_options = DockerTemplateOptions {
        environment: object
            .get("environment")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        extra_hosts: object
            .get("extra_hosts")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        extra_volumes: object
            .get("extra_volumes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        extra_configs: validate_docker_extra_configs_api(
            object.get("extra_configs"),
            object.get("startup_config_content").and_then(Value::as_str),
        )?,
        startup_config_content: object
            .get("startup_config_content")
            .and_then(Value::as_str)
            .map(str::to_owned),
        console_type: object
            .get("console_type")
            .and_then(Value::as_str)
            .unwrap_or("telnet")
            .to_owned(),
        console_resolution: object
            .get("console_resolution")
            .and_then(Value::as_str)
            .unwrap_or("1024x768")
            .to_owned(),
        console_http_port: object
            .get("console_http_port")
            .and_then(Value::as_u64)
            .and_then(|port| u16::try_from(port).ok()),
        console_http_path: object
            .get("console_http_path")
            .and_then(Value::as_str)
            .map(str::to_owned),
        aux_type: object
            .get("aux_type")
            .and_then(Value::as_str)
            .unwrap_or("none")
            .to_owned(),
        custom_adapters: serde_json::from_value(
            object
                .get("custom_adapters")
                .cloned()
                .unwrap_or_else(|| json!([])),
        )
        .map_err(|_| {
            ApiError::bad_request("custom_adapters must be an array of adapter definitions")
        })?,
        ..DockerTemplateOptions::default()
    };
    let (container_id, image_digest) = state
        .runtime
        .create_container_with_options(
            &project_id,
            &node_id,
            &image,
            &name,
            start_command.as_deref(),
            u64::try_from(memory_mb).ok(),
            Some(cpus),
            &docker_options,
        )
        .await
        .map_err(runtime_api_error)?;
    object.insert("container_id".into(), json!(container_id));
    object.insert("image_digest".into(), json!(image_digest));
    if let Some(console_port) = state.runtime.console_port(&container_id).await {
        object.insert("console".into(), json!(console_port));
    }

    match create_topology_object(state.clone(), &project_id, "node", "node_id", node).await {
        Ok(response) => Ok(response),
        Err(error) => {
            if let Err(cleanup_error) = state
                .runtime
                .remove_node_resources(&node_id, Some(&container_id))
                .await
            {
                tracing::error!(error = %cleanup_error.message(), %container_id, "could not roll back created container");
            }
            Err(error)
        }
    }
}
