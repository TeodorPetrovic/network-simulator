use super::*;

pub(super) fn validate_node_configuration_patch(
    node_type: &str,
    patch: Value,
) -> Result<Value, ApiError> {
    let patch = patch
        .as_object()
        .ok_or_else(|| ApiError::bad_request("device configuration must be an object"))?;
    if patch.is_empty() {
        return Err(ApiError::bad_request("device configuration is empty"));
    }
    let allowed: &[&str] = match node_type {
        "qemu" => &[
            "name",
            "memory",
            "cpus",
            "adapters",
            "adapter_type",
            "hda_disk_interface",
            "boot_priority",
        ],
        "docker" => &[
            "name",
            "memory",
            "cpus",
            "adapters",
            "start_command",
            "environment",
            "extra_hosts",
            "extra_volumes",
            "extra_configs",
            "startup_config_content",
            "console_type",
            "console_resolution",
            "console_http_port",
            "console_http_path",
            "aux_type",
            "custom_adapters",
        ],
        "vpcs" => &["name", "startup_script", "console_type"],
        "dynamips" => &["name", "startup_config"],
        _ => return Err(ApiError::bad_request("unsupported per-device runtime")),
    };
    let mut validated = Map::new();
    for (key, value) in patch {
        if !allowed.contains(&key.as_str()) {
            return Err(ApiError::bad_request(format!(
                "'{key}' cannot be changed in this device configuration"
            )));
        }
        let value = match key.as_str() {
            "name" => {
                let name = value
                    .as_str()
                    .map(str::trim)
                    .filter(|name| !name.is_empty() && name.len() <= 128)
                    .ok_or_else(|| ApiError::bad_request("device name must be 1–128 characters"))?;
                json!(name)
            }
            "memory" => {
                let memory = value
                    .as_u64()
                    .filter(|memory| {
                        if node_type == "qemu" {
                            (16..=1_048_576).contains(memory)
                        } else {
                            *memory <= 1_048_576
                        }
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request("memory is outside the supported range")
                    })?;
                json!(memory)
            }
            "cpus" => {
                if node_type == "qemu" {
                    let cpus = value
                        .as_u64()
                        .filter(|cpus| (1..=256).contains(cpus))
                        .ok_or_else(|| ApiError::bad_request("QEMU CPUs must be from 1 to 256"))?;
                    json!(cpus)
                } else {
                    let cpus = value
                        .as_f64()
                        .filter(|cpus| cpus.is_finite() && (0.0..=256.0).contains(cpus))
                        .ok_or_else(|| {
                            ApiError::bad_request("Docker CPU limit must be from 0 to 256")
                        })?;
                    json!(cpus)
                }
            }
            "adapters" => {
                let adapters = value
                    .as_u64()
                    .filter(|adapters| (1..=99).contains(adapters))
                    .ok_or_else(|| ApiError::bad_request("adapters must be from 1 to 99"))?;
                json!(adapters)
            }
            "adapter_type" => {
                let model = value
                    .as_str()
                    .filter(|model| {
                        matches!(*model, "e1000" | "virtio-net-pci" | "rtl8139" | "vmxnet3")
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request("unsupported QEMU network adapter model")
                    })?;
                json!(model)
            }
            "hda_disk_interface" => {
                let interface = value
                    .as_str()
                    .filter(|interface| matches!(*interface, "ide" | "scsi" | "virtio" | "sd"))
                    .ok_or_else(|| {
                        ApiError::bad_request("unsupported QEMU primary disk interface")
                    })?;
                json!(interface)
            }
            "boot_priority" => {
                let priority = value
                    .as_str()
                    .filter(|priority| matches!(*priority, "c" | "d" | "n" | "dc" | "cd"))
                    .ok_or_else(|| ApiError::bad_request("unsupported QEMU boot priority"))?;
                json!(priority)
            }
            "start_command" | "environment" | "extra_hosts" => {
                let text = value
                    .as_str()
                    .filter(|text| text.len() <= 65_536)
                    .ok_or_else(|| {
                        ApiError::bad_request(format!("{key} must be text under 64 KiB"))
                    })?;
                json!(text)
            }
            "startup_config_content" => {
                let text = value
                    .as_str()
                    .filter(|text| text.len() <= 1024 * 1024)
                    .ok_or_else(|| {
                        ApiError::bad_request("startup_config_content must be text under 1 MiB")
                    })?;
                json!(text)
            }
            "extra_configs" => {
                let configs = validate_docker_extra_configs_api(Some(value), None)?;
                serde_json::to_value(configs).map_err(internal_json_error)?
            }
            "startup_script" | "startup_config" => {
                let text = value
                    .as_str()
                    .filter(|text| text.len() <= 1_048_576)
                    .ok_or_else(|| {
                        ApiError::bad_request(format!("{key} must be text under 1 MiB"))
                    })?;
                json!(text)
            }
            "extra_volumes" => {
                let volumes = value.as_array().ok_or_else(|| {
                    ApiError::bad_request("extra_volumes must be an array of container paths")
                })?;
                if volumes.len() > 64 || volumes.iter().any(|volume| volume.as_str().is_none()) {
                    return Err(ApiError::bad_request(
                        "extra_volumes must contain at most 64 text paths",
                    ));
                }
                json!(volumes)
            }
            "console_type" => {
                let value = value
                    .as_str()
                    .filter(|value| {
                        if node_type == "vpcs" {
                            matches!(*value, "telnet" | "ssh" | "none")
                        } else {
                            matches!(*value, "none" | "telnet" | "vnc" | "http" | "https")
                        }
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request(format!("unsupported {node_type} console_type"))
                    })?;
                json!(value)
            }
            "console_resolution" => {
                let value = value
                    .as_str()
                    .filter(|value| {
                        value.split_once('x').is_some_and(|(width, height)| {
                            !width.is_empty()
                                && !height.is_empty()
                                && width.bytes().all(|byte| byte.is_ascii_digit())
                                && height.bytes().all(|byte| byte.is_ascii_digit())
                        })
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request("console_resolution must use WIDTHxHEIGHT")
                    })?;
                json!(value)
            }
            "console_http_port" => {
                let port = value
                    .as_u64()
                    .filter(|port| (1..=65535).contains(port))
                    .ok_or_else(|| {
                        ApiError::bad_request("console_http_port must be from 1 to 65535")
                    })?;
                json!(port)
            }
            "console_http_path" => {
                let path = value
                    .as_str()
                    .filter(|path| {
                        path.starts_with('/') && !path.bytes().any(|byte| byte.is_ascii_control())
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request("console_http_path must be an absolute URL path")
                    })?;
                json!(path)
            }
            "aux_type" => {
                let value = value
                    .as_str()
                    .filter(|value| matches!(*value, "none" | "telnet" | "ssh"))
                    .ok_or_else(|| ApiError::bad_request("unsupported Docker aux_type"))?;
                json!(value)
            }
            "custom_adapters" => {
                if !value.is_array() || value.as_array().is_some_and(|adapters| adapters.len() > 99)
                {
                    return Err(ApiError::bad_request(
                        "custom_adapters must be an array of at most 99 entries",
                    ));
                }
                serde_json::from_value::<Vec<crate::runtime::DockerCustomAdapter>>(value.clone())
                    .map_err(|_| {
                    ApiError::bad_request("custom_adapters contain an invalid adapter definition")
                })?;
                json!(value)
            }
            _ => unreachable!("keys were restricted above"),
        };
        validated.insert(key.clone(), value);
    }
    Ok(Value::Object(validated))
}

pub(super) async fn validate_adapter_count_against_links(
    pool: &SqlitePool,
    project_id: &str,
    node_id: &str,
    node_type: &str,
    adapters: u64,
) -> Result<(), ApiError> {
    let links = list_objects(pool, project_id, "link").await?.0;
    for link in links {
        let Some(endpoints) = link["nodes"].as_array() else {
            continue;
        };
        for endpoint in endpoints {
            if endpoint["node_id"].as_str() != Some(node_id) {
                continue;
            }
            let configured_port = if node_type == "docker" {
                endpoint["port_number"].as_u64()
            } else {
                endpoint["adapter_number"].as_u64()
            }
            .unwrap_or_default();
            if configured_port >= adapters {
                return Err(ApiError::conflict(format!(
                    "Adapter count cannot be reduced to {adapters}; a link is using port {configured_port}"
                )));
            }
        }
    }
    Ok(())
}

pub(super) async fn validate_builtin_configuration_links(
    pool: &SqlitePool,
    project_id: &str,
    node_id: &str,
    current: &Value,
    updated: &Value,
) -> Result<(), ApiError> {
    let node_type = updated["node_type"].as_str().unwrap_or_default();
    let current_count = current["ports_mapping"]
        .as_array()
        .map(Vec::len)
        .unwrap_or_default();
    let updated_count = updated["ports_mapping"]
        .as_array()
        .map(Vec::len)
        .unwrap_or_default();
    let links = list_objects(pool, project_id, "link").await?.0;
    for link in links {
        let Some(endpoints) = link["nodes"].as_array() else {
            continue;
        };
        for endpoint in endpoints {
            if endpoint["node_id"].as_str() != Some(node_id) {
                continue;
            }
            let port = endpoint["port_number"].as_u64().unwrap_or_default() as usize;
            if node_type == "ethernet_switch" && current_count != updated_count {
                return Err(ApiError::conflict(
                    "Cannot change the port count of an Ethernet switch that is already connected",
                ));
            }
            if port >= updated_count {
                return Err(ApiError::conflict(format!(
                    "Cannot remove port {port} while it is connected"
                )));
            }
        }
    }
    Ok(())
}

pub(super) async fn persist_node_configuration(
    state: AppState,
    project_id: &str,
    node_id: &str,
    patch: Value,
    extra_fields: Map<String, Value>,
) -> Result<Json<Value>, ApiError> {
    let mut tx = state.store.pool().begin().await?;
    ensure_project_exists(&mut tx, project_id).await?;
    let row = sqlx::query(
        "SELECT object_json FROM topology_objects WHERE project_id = ? AND kind = 'node' AND object_id = ?",
    )
    .bind(project_id)
    .bind(node_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::not_found(format!("Node {node_id} does not exist")))?;
    let mut node = decode_json(row.get::<String, _>("object_json"))?;
    let object = as_object_mut(&mut node, "stored node is invalid")?;
    for (key, value) in patch.as_object().expect("validated patch is an object") {
        object.insert(key.clone(), value.clone());
    }
    for (key, value) in extra_fields {
        object.insert(key, value);
    }
    let status = if BuiltinRuntime::is_type(
        object
            .get("node_type")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    ) {
        "started"
    } else {
        "stopped"
    };
    object.insert("status".to_owned(), json!(status));
    let name = required_string(object, "name")?.to_owned();
    sqlx::query(
        "UPDATE topology_objects SET name = ?, object_json = ? WHERE project_id = ? AND kind = 'node' AND object_id = ?",
    )
    .bind(name)
    .bind(serde_json::to_string(&node).map_err(internal_json_error)?)
    .bind(project_id)
    .bind(node_id)
    .execute(&mut *tx)
    .await?;
    let event = Store::append_event(&mut tx, project_id, "node.updated", &node).await?;
    tx.commit().await?;
    state.store.publish(event);
    Ok(Json(node))
}
