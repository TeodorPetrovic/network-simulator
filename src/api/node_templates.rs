use super::*;

pub(super) async fn create_node(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Json(node): Json<Value>,
) -> Result<impl IntoResponse, ApiError> {
    match node.get("node_type").and_then(Value::as_str) {
        Some("docker") => {
            super::docker_node::create_docker_node(state, project_id.to_string(), node).await
        }
        Some("qemu") => {
            super::qemu_node::create_qemu_node(state, project_id.to_string(), node).await
        }
        Some("vpcs") => {
            super::vpcs_node::create_vpcs_node(state, project_id.to_string(), node).await
        }
        Some("dynamips") => {
            super::dynamips_node::create_dynamips_node(state, project_id.to_string(), node).await
        }
        Some(node_type) if BuiltinRuntime::is_type(node_type) => {
            create_builtin_node(state, project_id.to_string(), node).await
        }
        Some(node_type) => Err(ApiError::bad_request(format!(
            "Unsupported node type '{node_type}'. Executable node types are Docker, QEMU, VPCS, Dynamips, Ethernet switch, Ethernet hub, Cloud, and NAT."
        ))),
        None => Err(ApiError::bad_request("node_type is required")),
    }
}

pub(super) async fn create_builtin_node(
    state: AppState,
    project_id: String,
    mut node: Value,
) -> Result<Response, ApiError> {
    let object = as_object_mut(&mut node, "node body must be an object")?;
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
    object.insert("node_id".into(), json!(node_id));
    object.insert("project_id".into(), json!(project_id.clone()));
    let node = BuiltinRuntime::normalize_node(node).map_err(ApiError::bad_request)?;
    BuiltinRuntime::validate_host_interfaces(&node).map_err(ApiError::bad_request)?;
    create_topology_object(state, &project_id, "node", "node_id", node).await
}

pub(super) async fn create_node_from_template(
    State(state): State<AppState>,
    Path((project_id, template_id)): Path<(Uuid, Uuid)>,
    Json(usage): Json<Value>,
) -> Result<impl IntoResponse, ApiError> {
    let template_id_string = template_id.to_string();
    let template = if let Some(row) =
        sqlx::query("SELECT template_json FROM controller_templates WHERE template_id = ?")
            .bind(&template_id_string)
            .fetch_optional(state.store.pool())
            .await?
    {
        decode_json(row.get::<String, _>("template_json"))?
    } else {
        available_templates(&state)
            .await?
            .into_iter()
            .find(|template| {
                template.get("template_id").and_then(Value::as_str)
                    == Some(template_id_string.as_str())
            })
            .ok_or_else(|| ApiError::not_found(format!("Template {template_id} does not exist")))?
    };
    let name = usage
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .or_else(|| template.get("name").and_then(Value::as_str))
        .ok_or_else(|| ApiError::bad_request("template has no name"))?;
    let node_type = template
        .get("template_type")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad_request("template has no runtime type"))?;
    if BuiltinRuntime::is_type(node_type) {
        let mut node = template.clone();
        node["node_type"] = json!(node_type);
        node["name"] = json!(name);
        node["template_id"] = json!(template_id);
        node["x"] = usage.get("x").cloned().unwrap_or(json!(0));
        node["y"] = usage.get("y").cloned().unwrap_or(json!(0));
        for key in [
            "ports_mapping",
            "remote_console_host",
            "remote_console_port",
            "remote_console_type",
            "remote_console_http_path",
            "nat_interface",
        ] {
            if let Some(value) = usage.get(key) {
                node[key] = value.clone();
            }
        }
        if let Some(compute_id) = usage.get("compute_id") {
            node["compute_id"] = compute_id.clone();
        }
        return create_builtin_node(state, project_id.to_string(), node).await;
    }
    let mut node = if node_type == "docker" {
        json!({
            "name": name,
            "node_type": "docker",
            "image": template["image"],
            "image_digest": template["image_digest"],
            "template_id": template_id,
            "adapters": template["adapters"],
            "memory": template["memory"],
            "cpus": template["cpus"],
            "environment": template["environment"],
            "extra_hosts": template["extra_hosts"],
            "extra_volumes": template["extra_volumes"],
            "extra_configs": usage
                .get("extra_configs")
                .filter(|value| !value.is_null())
                .cloned()
                .or_else(|| {
                    template
                        .get("extra_configs")
                        .filter(|value| !value.is_null())
                        .cloned()
                })
                .unwrap_or(json!([])),
            "startup_config_content": usage
                .get("startup_config_content")
                .filter(|value| !value.is_null())
                .cloned()
                .or_else(|| {
                    template
                        .get("startup_config_content")
                        .filter(|value| !value.is_null())
                        .cloned()
                })
                .unwrap_or(json!("")),
            "console_type": template["console_type"],
            "console_resolution": template["console_resolution"],
            "console_http_port": template["console_http_port"],
            "console_http_path": template["console_http_path"],
            "aux_type": template["aux_type"],
            "custom_adapters": template["custom_adapters"],
            "x": usage.get("x").cloned().unwrap_or(json!(0)),
            "y": usage.get("y").cloned().unwrap_or(json!(0))
        })
    } else if node_type == "qemu" {
        json!({
            "name": name,
            "node_type": "qemu",
            "hda_disk_image": template["hda_disk_image"],
            "template_id": template_id,
            "adapters": template["adapters"],
            "memory": template["memory"],
            "cpus": template["cpus"],
            "platform": template["platform"],
            "adapter_type": template["adapter_type"],
            "hda_disk_interface": template["hda_disk_interface"],
            "boot_priority": template["boot_priority"],
            "linked_clone": template["linked_clone"],
            "maxcpus": template
                .get("maxcpus")
                .filter(|value| !value.is_null())
                .cloned()
                .or_else(|| template.get("cpus").cloned())
                .unwrap_or(json!(1)),
            "hdb_disk_image": template["hdb_disk_image"],
            "hdb_disk_interface": template["hdb_disk_interface"],
            "hdb_disk_image_backing_file": template["hdb_disk_image_backing_file"],
            "hdc_disk_image": template["hdc_disk_image"],
            "hdc_disk_interface": template["hdc_disk_interface"],
            "hdc_disk_image_backing_file": template["hdc_disk_image_backing_file"],
            "hdd_disk_image": template["hdd_disk_image"],
            "hdd_disk_interface": template["hdd_disk_interface"],
            "hdd_disk_image_backing_file": template["hdd_disk_image_backing_file"],
            "hda_disk_image_backing_file": template["hda_disk_image_backing_file"],
            "disk_images_to_reset": template["disk_images_to_reset"],
            "cdrom_image": template["cdrom_image"],
            "cdrom_image2": template["cdrom_image2"],
            "bios_image": template["bios_image"],
            "kernel_image": template["kernel_image"],
            "initrd": template["initrd"],
            "kernel_command_line": template["kernel_command_line"],
            "console_type": template["console_type"],
            "aux_type": template["aux_type"],
            "on_close": template["on_close"],
            "process_priority": template["process_priority"],
            "options": template["options"],
            "tpm": template["tpm"],
            "uefi": template["uefi"],
            "replicate_network_connection_state": template["replicate_network_connection_state"],
            "create_config_disk": template["create_config_disk"],
            "cpu_throttling": template["cpu_throttling"],
            "x": usage.get("x").cloned().unwrap_or(json!(0)),
            "y": usage.get("y").cloned().unwrap_or(json!(0))
        })
    } else if node_type == "vpcs" {
        json!({
            "name": name,
            "node_type": "vpcs",
            "template_id": template_id,
            "adapters": 1,
            "memory": 0,
            "cpus": 0,
            "startup_script": usage
                .get("startup_script")
                .filter(|value| !value.is_null())
                .cloned()
                .or_else(|| template.get("startup_script").cloned())
                .unwrap_or(json!("")),
            "base_script_file": template["base_script_file"],
            "console_type": template["console_type"],
            "console_auto_start": template["console_auto_start"],
            "x": usage.get("x").cloned().unwrap_or(json!(0)),
            "y": usage.get("y").cloned().unwrap_or(json!(0))
        })
    } else if node_type == "dynamips" {
        json!({
            "name": name,
            "node_type": "dynamips",
            "template_id": template_id,
            "platform": template["platform"],
            "chassis": template["chassis"],
            "slot0": template["slot0"],
            "image": template["image"],
            "adapters": template["adapters"],
            "ram": template["memory"],
            "nvram": template["nvram"],
            "dynamips_id": template["dynamips_id"],
            "idlepc": usage.get("idlepc").cloned().unwrap_or(template["idlepc"].clone()),
            "startup_config": usage
                .get("startup_config")
                .filter(|value| !value.is_null())
                .cloned()
                .or_else(|| template.get("startup_config").cloned())
                .unwrap_or(json!("")),
            "private_config": template["private_config"],
            "mmap": template["mmap"],
            "sparsemem": template["sparsemem"],
            "clock_divisor": template["clock_divisor"],
            "idlemax": template["idlemax"],
            "idlesleep": template["idlesleep"],
            "exec_area": template["exec_area"],
            "disk0": template["disk0"],
            "disk1": template["disk1"],
            "auto_delete_disks": template["auto_delete_disks"],
            "mac_addr": template["mac_addr"],
            "system_id": template["system_id"],
            "slot1": template["slot1"],
            "slot2": template["slot2"],
            "slot3": template["slot3"],
            "slot4": template["slot4"],
            "slot5": template["slot5"],
            "slot6": template["slot6"],
            "wic0": template["wic0"],
            "wic1": template["wic1"],
            "wic2": template["wic2"],
            "npe": template["npe"],
            "midplane": template["midplane"],
            "iomem": template["iomem"],
            "ghost_ios": template["ghost_ios"],
            "console_type": template["console_type"],
            "aux_type": template["aux_type"],
            "x": usage.get("x").cloned().unwrap_or(json!(0)),
            "y": usage.get("y").cloned().unwrap_or(json!(0))
        })
    } else {
        return Err(ApiError::bad_request(format!(
            "Template runtime '{node_type}' is not supported"
        )));
    };
    if node_type == "docker"
        && let Some(start_command) = usage
            .get("start_command")
            .and_then(Value::as_str)
            .or_else(|| template.get("start_command").and_then(Value::as_str))
    {
        node["start_command"] = json!(start_command);
    }
    if let Some(compute_id) = usage.get("compute_id") {
        node["compute_id"] = compute_id.clone();
    }
    match node_type {
        "docker" => {
            super::docker_node::create_docker_node(state, project_id.to_string(), node).await
        }
        "qemu" => super::qemu_node::create_qemu_node(state, project_id.to_string(), node).await,
        "vpcs" => super::vpcs_node::create_vpcs_node(state, project_id.to_string(), node).await,
        "dynamips" => {
            super::dynamips_node::create_dynamips_node(state, project_id.to_string(), node).await
        }
        _ => unreachable!("template runtime was validated"),
    }
}
