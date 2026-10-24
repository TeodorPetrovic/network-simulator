use super::*;

pub(super) async fn create_qemu_node(
    state: AppState,
    project_id: String,
    mut node: Value,
) -> Result<Response, ApiError> {
    let object = as_object_mut(&mut node, "node body must be an object")?;
    let name = required_string(object, "name")?.to_owned();
    let node_type = required_string(object, "node_type")?;
    if node_type != "qemu" {
        return Err(ApiError::bad_request(format!(
            "Unsupported QEMU node type '{node_type}'"
        )));
    }
    let image = required_string(object, "hda_disk_image")?.to_owned();
    if object
        .get("compute_id")
        .and_then(Value::as_str)
        .is_some_and(|compute_id| compute_id != "local")
    {
        return Err(ApiError::bad_request(
            "This runtime build can execute nodes only on the local QEMU compute",
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
    object.insert("node_type".into(), json!("qemu"));
    object.insert("hda_disk_image".into(), json!(image));
    set_default(object, "compute_id", json!("local"));
    set_default(object, "status", json!("stopped"));
    set_default(object, "x", json!(0));
    set_default(object, "y", json!(0));
    set_default(object, "z", json!(1));
    set_default(object, "locked", json!(false));
    set_default(object, "adapters", json!(1));
    set_default(object, "memory", json!(512));
    set_default(object, "cpus", json!(1));
    let default_maxcpus = object.get("cpus").and_then(Value::as_u64).unwrap_or(1);
    set_default(object, "maxcpus", json!(default_maxcpus));
    set_default(object, "linked_clone", json!(true));
    set_default(object, "hda_disk_interface", json!("ide"));
    set_default(object, "boot_priority", json!("c"));
    set_default(object, "console_type", json!("telnet"));
    set_default(object, "aux_type", json!("none"));
    set_default(object, "on_close", json!("power_off"));
    set_default(object, "process_priority", json!("normal"));
    set_default(object, "options", json!(""));
    set_default(object, "tpm", json!(false));
    set_default(object, "uefi", json!(false));
    set_default(object, "replicate_network_connection_state", json!(true));
    set_default(object, "create_config_disk", json!(false));
    set_default(object, "cpu_throttling", json!(0));
    set_default(object, "properties", json!({}));

    let memory_mb = object
        .get("memory")
        .map(|value| {
            value
                .as_i64()
                .ok_or_else(|| ApiError::bad_request("memory must be an integer number of MB"))
        })
        .transpose()?
        .unwrap_or(512);
    if !(16..=1_048_576).contains(&memory_mb) {
        return Err(ApiError::bad_request(
            "QEMU memory must be between 16 and 1048576 MB",
        ));
    }
    let cpus = object
        .get("cpus")
        .map(|value| {
            value
                .as_i64()
                .ok_or_else(|| ApiError::bad_request("QEMU cpus must be an integer"))
        })
        .transpose()?
        .unwrap_or(1);
    if !(1..=255).contains(&cpus) {
        return Err(ApiError::bad_request("QEMU cpus must be between 1 and 255"));
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
    if !(0..=275).contains(&adapters) {
        return Err(ApiError::bad_request(
            "QEMU adapters must be between 0 and 275",
        ));
    }
    let maxcpus = object
        .get("maxcpus")
        .and_then(Value::as_u64)
        .filter(|maxcpus| (1..=255).contains(maxcpus))
        .ok_or_else(|| ApiError::bad_request("QEMU maxcpus must be from 1 to 255"))?;
    if maxcpus < cpus as u64 {
        return Err(ApiError::bad_request(
            "QEMU maxcpus cannot be less than cpus",
        ));
    }
    let disk_images_to_reset = object
        .get("disk_images_to_reset")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .map(|value| {
                    value.as_str().map(str::to_owned).ok_or_else(|| {
                        ApiError::bad_request("disk_images_to_reset entries must be strings")
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let qemu_options = QemuTemplateOptions {
        platform: object
            .get("platform")
            .and_then(Value::as_str)
            .map(str::to_owned),
        adapter_type: object
            .get("adapter_type")
            .and_then(Value::as_str)
            .map(str::to_owned),
        hda_disk_interface: object
            .get("hda_disk_interface")
            .and_then(Value::as_str)
            .map(str::to_owned),
        boot_priority: object
            .get("boot_priority")
            .and_then(Value::as_str)
            .map(str::to_owned),
        linked_clone: object.get("linked_clone").and_then(Value::as_bool),
        maxcpus: Some(maxcpus as u32),
        hdb_disk_image: object
            .get("hdb_disk_image")
            .and_then(Value::as_str)
            .map(str::to_owned),
        hdb_disk_interface: object
            .get("hdb_disk_interface")
            .and_then(Value::as_str)
            .map(str::to_owned),
        hda_disk_image_backing_file: object
            .get("hda_disk_image_backing_file")
            .and_then(Value::as_str)
            .map(str::to_owned),
        hdb_disk_image_backing_file: object
            .get("hdb_disk_image_backing_file")
            .and_then(Value::as_str)
            .map(str::to_owned),
        hdc_disk_image: object
            .get("hdc_disk_image")
            .and_then(Value::as_str)
            .map(str::to_owned),
        hdc_disk_interface: object
            .get("hdc_disk_interface")
            .and_then(Value::as_str)
            .map(str::to_owned),
        hdc_disk_image_backing_file: object
            .get("hdc_disk_image_backing_file")
            .and_then(Value::as_str)
            .map(str::to_owned),
        hdd_disk_image: object
            .get("hdd_disk_image")
            .and_then(Value::as_str)
            .map(str::to_owned),
        hdd_disk_interface: object
            .get("hdd_disk_interface")
            .and_then(Value::as_str)
            .map(str::to_owned),
        hdd_disk_image_backing_file: object
            .get("hdd_disk_image_backing_file")
            .and_then(Value::as_str)
            .map(str::to_owned),
        disk_images_to_reset,
        cdrom_image: object
            .get("cdrom_image")
            .and_then(Value::as_str)
            .map(str::to_owned),
        bios_image: object
            .get("bios_image")
            .and_then(Value::as_str)
            .map(str::to_owned),
        kernel_image: object
            .get("kernel_image")
            .and_then(Value::as_str)
            .map(str::to_owned),
        initrd: object
            .get("initrd")
            .and_then(Value::as_str)
            .map(str::to_owned),
        kernel_command_line: object
            .get("kernel_command_line")
            .and_then(Value::as_str)
            .map(str::to_owned),
        console_type: object
            .get("console_type")
            .and_then(Value::as_str)
            .map(str::to_owned),
        aux_type: object
            .get("aux_type")
            .and_then(Value::as_str)
            .map(str::to_owned),
        on_close: object
            .get("on_close")
            .and_then(Value::as_str)
            .map(str::to_owned),
        process_priority: object
            .get("process_priority")
            .and_then(Value::as_str)
            .map(str::to_owned),
        options: object
            .get("options")
            .and_then(Value::as_str)
            .map(str::to_owned),
        tpm: object.get("tpm").and_then(Value::as_bool),
        uefi: object.get("uefi").and_then(Value::as_bool),
        replicate_network_connection_state: object
            .get("replicate_network_connection_state")
            .and_then(Value::as_bool),
        create_config_disk: object.get("create_config_disk").and_then(Value::as_bool),
        cpu_throttling: object
            .get("cpu_throttling")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok()),
    };
    object.remove("disk_images_to_reset");
    let vm = state
        .qemu
        .create_vm_with_options(
            &project_id,
            &node_id,
            &image,
            memory_mb as u64,
            cpus as u32,
            adapters as u32,
            &qemu_options,
        )
        .await
        .map_err(runtime_api_error)?;
    for (key, value) in state
        .qemu
        .runtime_fields(&vm)
        .as_object()
        .expect("runtime fields are an object")
    {
        object.insert(key.clone(), value.clone());
    }

    match create_topology_object(state.clone(), &project_id, "node", "node_id", node).await {
        Ok(response) => Ok(response),
        Err(error) => {
            if let Err(cleanup_error) = state.qemu.remove_vm(&vm).await {
                tracing::error!(error = %cleanup_error.message(), node_id, "could not roll back created QEMU disk overlay");
            }
            Err(error)
        }
    }
}
