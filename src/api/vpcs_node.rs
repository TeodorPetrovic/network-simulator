use super::*;

pub(super) async fn create_vpcs_node(
    state: AppState,
    project_id: String,
    mut node: Value,
) -> Result<Response, ApiError> {
    let object = as_object_mut(&mut node, "node body must be an object")?;
    let name = required_string(object, "name")?.to_owned();
    let node_type = required_string(object, "node_type")?;
    if node_type != "vpcs" {
        return Err(ApiError::bad_request(format!(
            "Unsupported VPCS node type '{node_type}'"
        )));
    }
    if object
        .get("compute_id")
        .and_then(Value::as_str)
        .is_some_and(|compute_id| compute_id != "local")
    {
        return Err(ApiError::bad_request(
            "This runtime build can execute nodes only on the local VPCS compute",
        ));
    }
    state
        .vpcs
        .ensure_available()
        .await
        .map_err(runtime_api_error)?;
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
    object.insert("project_id".into(), json!(project_id.clone()));
    object.insert("node_type".into(), json!("vpcs"));
    set_default(object, "compute_id", json!("local"));
    set_default(object, "status", json!("stopped"));
    set_default(object, "x", json!(0));
    set_default(object, "y", json!(0));
    set_default(object, "z", json!(1));
    set_default(object, "locked", json!(false));
    set_default(object, "adapters", json!(1));
    set_default(object, "memory", json!(0));
    set_default(object, "cpus", json!(0));
    set_default(object, "startup_script", json!(""));
    set_default(object, "base_script_file", json!("vpcs_base_config.txt"));
    set_default(object, "console_type", json!("telnet"));
    set_default(object, "console_auto_start", json!(false));
    set_default(object, "properties", json!({}));
    if object
        .get("console_type")
        .and_then(Value::as_str)
        .is_some_and(|value| !matches!(value, "telnet" | "ssh" | "none"))
    {
        return Err(ApiError::bad_request(
            "Unsupported VPCS console_type; expected telnet, ssh, or none",
        ));
    }
    if object.get("adapters").and_then(Value::as_u64) != Some(1) {
        return Err(ApiError::bad_request(
            "VPCS provides exactly one Ethernet adapter",
        ));
    }

    create_topology_object(state, &project_id, "node", "node_id", node).await
}
