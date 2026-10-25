use super::*;

pub(super) async fn create_dynamips_node(
    state: AppState,
    project_id: String,
    mut node: Value,
) -> Result<Response, ApiError> {
    let object = as_object_mut(&mut node, "node body must be an object")?;
    let name = required_string(object, "name")?.to_owned();
    if required_string(object, "node_type")? != "dynamips" {
        return Err(ApiError::bad_request(
            "Dynamips nodes require node_type 'dynamips'",
        ));
    }
    if object
        .get("compute_id")
        .and_then(Value::as_str)
        .is_some_and(|compute_id| compute_id != "local")
    {
        return Err(ApiError::bad_request(
            "This runtime build can execute nodes only on the local Dynamips compute",
        ));
    }
    let image = required_string(object, "image")?.to_owned();
    let platform = object
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or("c7200")
        .to_owned();
    let chassis = object
        .get("chassis")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| match platform.as_str() {
            "c3600" => Some("3640".to_owned()),
            "c2600" => Some("2610".to_owned()),
            "c1700" => Some("1760".to_owned()),
            _ => None,
        });
    let slot0 = object
        .get("slot0")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let ram_mb = object
        .get("ram")
        .and_then(Value::as_u64)
        .unwrap_or(match platform.as_str() {
            "c1700" | "c2600" => 160,
            "c2691" | "c3600" => 192,
            "c3725" => 128,
            "c3745" => 256,
            _ => 512,
        });
    let nvram = object
        .get("nvram")
        .and_then(Value::as_u64)
        .unwrap_or(match platform.as_str() {
            "c2691" | "c3725" | "c3745" => 256,
            "c7200" => 512,
            _ => 128,
        });
    let idle_pc = object
        .get("idlepc")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let startup_config = object
        .get("startup_config")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
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

    let existing_nodes = sqlx::query(
        "SELECT object_json FROM topology_objects WHERE project_id = ? AND kind = 'node'",
    )
    .bind(&project_id)
    .fetch_all(state.store.pool())
    .await?;
    let used_ids = existing_nodes
        .iter()
        .filter_map(|row| decode_json(row.get::<String, _>("object_json")).ok())
        .filter(|node| node["node_type"].as_str() == Some("dynamips"))
        .filter_map(|node| node["dynamips_id"].as_u64())
        .collect::<HashSet<_>>();
    let dynamips_id = object
        .get("dynamips_id")
        .and_then(Value::as_u64)
        .or_else(|| (1..=4096).find(|identifier| !used_ids.contains(identifier)))
        .filter(|identifier| (1..=4096).contains(identifier))
        .ok_or_else(|| ApiError::conflict("No free Dynamips internal identifier is available"))?
        as u32;
    let dynamips_options = dynamips_options_from_template(object, dynamips_id)?;

    object.insert("name".into(), json!(name));
    object.insert("node_id".into(), json!(node_id));
    object.insert("project_id".into(), json!(project_id.clone()));
    object.insert("node_type".into(), json!("dynamips"));
    object.insert("platform".into(), json!(platform));
    object.insert("chassis".into(), json!(chassis));
    object.insert("image".into(), json!(image));
    object.insert("ram".into(), json!(ram_mb));
    object.insert("nvram".into(), json!(nvram));
    object.insert("idlepc".into(), json!(idle_pc));
    object.insert("startup_config".into(), json!(startup_config));
    object.insert("dynamips_id".into(), json!(dynamips_id));
    object.insert(
        "private_config".into(),
        json!(dynamips_options.private_config),
    );
    object.insert("mmap".into(), json!(dynamips_options.mmap));
    object.insert("sparsemem".into(), json!(dynamips_options.sparsemem));
    object.insert(
        "clock_divisor".into(),
        json!(dynamips_options.clock_divisor),
    );
    object.insert("idlemax".into(), json!(dynamips_options.idlemax));
    object.insert("idlesleep".into(), json!(dynamips_options.idlesleep));
    object.insert("exec_area".into(), json!(dynamips_options.exec_area));
    object.insert("disk0".into(), json!(dynamips_options.disk0));
    object.insert("disk1".into(), json!(dynamips_options.disk1));
    object.insert(
        "auto_delete_disks".into(),
        json!(dynamips_options.auto_delete_disks),
    );
    object.insert("ghost_ios".into(), json!(dynamips_options.ghost_ios));
    object.insert("console_type".into(), json!(dynamips_options.console_type));
    object.insert("aux_type".into(), json!(dynamips_options.aux_type));
    for (index, slot) in dynamips_options.slots.iter().enumerate() {
        object.insert(format!("slot{index}"), json!(slot));
    }
    for (index, wic) in dynamips_options.wics.iter().enumerate() {
        object.insert(format!("wic{index}"), json!(wic));
    }
    set_default(object, "compute_id", json!("local"));
    set_default(object, "status", json!("stopped"));
    set_default(object, "x", json!(0));
    set_default(object, "y", json!(0));
    set_default(object, "z", json!(1));
    set_default(object, "locked", json!(false));
    set_default(object, "adapters", json!(1));
    set_default(object, "memory", json!(ram_mb));
    set_default(object, "nvram", json!(128));
    set_default(object, "properties", json!({}));

    let vm = state
        .dynamips
        .create_router_with_options(
            &project_id,
            &node_id.to_string(),
            &name,
            &image,
            &platform,
            chassis.as_deref(),
            slot0.as_deref(),
            ram_mb,
            nvram,
            idle_pc.as_deref(),
            &startup_config,
            &dynamips_options,
        )
        .await
        .map_err(runtime_api_error)?;
    for (key, value) in state
        .dynamips
        .runtime_fields(&vm)
        .as_object()
        .expect("runtime fields are an object")
    {
        object.insert(key.clone(), value.clone());
    }

    match create_topology_object(state.clone(), &project_id, "node", "node_id", node).await {
        Ok(response) => Ok(response),
        Err(error) => {
            if let Err(cleanup_error) = state.dynamips.remove_router(&vm).await {
                tracing::error!(error = %cleanup_error.message(), %node_id, "could not roll back created Dynamips router");
            }
            Err(error)
        }
    }
}

pub(super) fn dynamips_options_from_template(
    object: &mut Map<String, Value>,
    dynamips_id: u32,
) -> Result<DynamipsOptions, ApiError> {
    // Appliance templates do not carry a runtime ID. `json!` in
    // `create_node_from_template` turns that absent field into `null`, which
    // fails to deserialize as `DynamipsOptions::dynamips_id: u32` before the
    // allocator's value can be applied. Normalize it before deserializing.
    object.insert("dynamips_id".into(), json!(dynamips_id));
    // Template-to-node construction includes absent fields as explicit nulls.
    // Serde's container-level `default` handles absent fields, but not nulls,
    // so drop null option values before deserializing to use platform defaults.
    object.retain(|_, value| !value.is_null());
    if object.get("sparsemem").is_none() {
        let platform = object
            .get("platform")
            .and_then(Value::as_str)
            .unwrap_or("c7200");
        object.insert(
            "sparsemem".into(),
            json!(!matches!(platform, "c1700" | "c2600")),
        );
    }
    let mut options: DynamipsOptions = serde_json::from_value(Value::Object(object.clone()))
        .map_err(|error| {
            ApiError::bad_request(format!("Dynamips runtime settings are invalid: {error}"))
        })?;
    options.dynamips_id = dynamips_id;
    options.mac_addr = options
        .mac_addr
        .take()
        .filter(|value| !value.trim().is_empty());
    options.system_id = options
        .system_id
        .take()
        .filter(|value| !value.trim().is_empty());
    options.slots = (0..7)
        .map(|index| {
            object
                .get(format!("slot{index}").as_str())
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    options.wics = (0..3)
        .map(|index| {
            object
                .get(format!("wic{index}").as_str())
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    Ok(options)
}
