use super::*;

pub(super) async fn apply_node_lifecycle(
    state: &AppState,
    project_id: &str,
    node: &mut Value,
    action: NodeAction,
) -> Result<String, ApiError> {
    let node_type = node
        .get("node_type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let node_id = node
        .get("node_id")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::conflict("Node has no node ID"))?;
    let before_stop = matches!(&action, NodeAction::Stop) && !BuiltinRuntime::is_type(node_type)
        || (matches!(&action, NodeAction::Reload) && matches!(node_type, "docker" | "vpcs"));
    if before_stop {
        detach_node_links(state, project_id, node_id).await?;
    }
    match node_type {
        "docker" => {
            let container_id = node
                .get("container_id")
                .and_then(Value::as_str)
                .ok_or_else(|| ApiError::conflict("Docker node has no runtime container"))?;
            let status = state
                .runtime
                .lifecycle(container_id, action)
                .await
                .map_err(runtime_api_error)?;
            if matches!(action, NodeAction::Start | NodeAction::Reload) {
                if let Err(error) = attach_node_links(state, project_id, node_id, false).await {
                    let _ = detach_node_links(state, project_id, node_id).await;
                    let _ = state
                        .runtime
                        .lifecycle(container_id, NodeAction::Stop)
                        .await;
                    return Err(error);
                }
            }
            Ok(status)
        }
        "qemu" => {
            let vm = state.qemu.vm_from_node(node).map_err(runtime_api_error)?;
            let links = if matches!(&action, NodeAction::Start) {
                node_link_entries(state, project_id, node_id)
                    .await?
                    .into_iter()
                    .map(|(link, endpoint, _)| {
                        qemu_link_from_endpoint(
                            &endpoint,
                            link["suspend"].as_bool().unwrap_or(false),
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                Vec::new()
            };
            let status = state
                .qemu
                .lifecycle(&vm, action, &links)
                .await
                .map_err(runtime_api_error)?;
            if matches!(&action, NodeAction::Start) {
                if let Err(error) = attach_node_links(state, project_id, node_id, false).await {
                    let _ = detach_node_links(state, project_id, node_id).await;
                    let _ = state.qemu.lifecycle(&vm, NodeAction::Stop, &[]).await;
                    return Err(error);
                }
            }
            Ok(status)
        }
        "vpcs" => {
            let name = node
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| ApiError::conflict("VPCS node has no name"))?;
            if matches!(action, NodeAction::Start | NodeAction::Reload) {
                for (_link, endpoint, _runtime) in
                    node_link_entries(state, project_id, node_id).await?
                {
                    if let (Some(adapter_port), Some(runtime_port)) = (
                        endpoint["runtime_adapter_port"]
                            .as_u64()
                            .and_then(|port| u16::try_from(port).ok()),
                        endpoint["runtime_udp_port"]
                            .as_u64()
                            .and_then(|port| u16::try_from(port).ok()),
                    ) {
                        state
                            .vpcs
                            .set_nio_ports(node_id, adapter_port, runtime_port)
                            .await;
                    }
                }
            }
            let startup_script = if matches!(action, NodeAction::Start | NodeAction::Reload) {
                vpcs_startup_script(state.store.pool(), project_id, node).await?
            } else {
                None
            };
            let status = state
                .vpcs
                .lifecycle(project_id, node_id, name, startup_script.as_deref(), action)
                .await
                .map_err(runtime_api_error)?;
            if matches!(action, NodeAction::Start | NodeAction::Reload) {
                if let Err(error) = attach_node_links(state, project_id, node_id, false).await {
                    let _ = detach_node_links(state, project_id, node_id).await;
                    let _ = state
                        .vpcs
                        .lifecycle(project_id, node_id, name, None, NodeAction::Stop)
                        .await;
                    return Err(error);
                }
            }
            Ok(status)
        }
        "dynamips" => {
            let vm = state
                .dynamips
                .vm_from_node(node)
                .map_err(runtime_api_error)?;
            if matches!(&action, NodeAction::Start) {
                attach_node_links(state, project_id, node_id, true).await?;
            }
            let status = state
                .dynamips
                .lifecycle(&vm, action)
                .await
                .map_err(runtime_api_error)?;
            if matches!(&action, NodeAction::Start)
                && let Err(error) = attach_node_links(state, project_id, node_id, false).await
            {
                let _ = detach_node_links(state, project_id, node_id).await;
                let _ = state.dynamips.lifecycle(&vm, NodeAction::Stop).await;
                return Err(error);
            }
            Ok(status)
        }
        "ethernet_switch" | "ethernet_hub" => Err(ApiError::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "Ethernet switches and hubs are always running",
        )),
        "cloud" | "nat" => match action {
            NodeAction::Start => {
                attach_node_links(state, project_id, node_id, false).await?;
                Ok("started".to_owned())
            }
            NodeAction::Stop | NodeAction::Reload | NodeAction::Suspend => Err(ApiError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                "Cloud and NAT nodes cannot be stopped, reloaded, or suspended",
            )),
        },
        _ => Err(ApiError::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "This node has no implemented runtime adapter",
        )),
    }
}

pub(super) async fn vpcs_startup_script(
    pool: &SqlitePool,
    project_id: &str,
    node: &Value,
) -> Result<Option<String>, ApiError> {
    if let Some(script) = node
        .get("startup_script")
        .and_then(Value::as_str)
        .filter(|script| !script.is_empty())
    {
        return Ok(Some(script.to_owned()));
    }
    let Some(base_script_file) = node
        .get("base_script_file")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
    else {
        return Ok(None);
    };
    let path = FsPath::new(base_script_file);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || base_script_file.contains('\\')
    {
        return Err(ApiError::bad_request(
            "VPCS base_script_file must be a relative project file path",
        ));
    }
    let normalized = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    let row =
        sqlx::query("SELECT contents FROM project_files WHERE project_id = ? AND file_path = ?")
            .bind(project_id)
            .bind(normalized)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|row| String::from_utf8_lossy(&row.get::<Vec<u8>, _>("contents")).into_owned()))
}

pub(super) async fn stop_runtime_node(
    state: &AppState,
    project_id: &str,
    node: &Value,
) -> Result<(), ApiError> {
    let Some(node_type) = node.get("node_type").and_then(Value::as_str) else {
        return Ok(());
    };
    if BuiltinRuntime::is_type(node_type) {
        let node_id = node
            .get("node_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        state.builtin.stop_node(node_id).await;
        return Ok(());
    }
    if node_type != "docker"
        && node_type != "qemu"
        && node_type != "vpcs"
        && node_type != "dynamips"
    {
        return Ok(());
    }
    match node_type {
        "docker" if node.get("container_id").and_then(Value::as_str).is_none() => return Ok(()),
        "qemu"
            if node
                .get("runtime_overlay")
                .and_then(Value::as_str)
                .is_none() =>
        {
            return Ok(());
        }
        "dynamips"
            if node
                .get("runtime_working_dir")
                .and_then(Value::as_str)
                .is_none()
                || node
                    .get("runtime_hypervisor_port")
                    .and_then(Value::as_u64)
                    .is_none() =>
        {
            return Ok(());
        }
        _ => {}
    }
    let mut node = node.clone();
    apply_node_lifecycle(state, project_id, &mut node, NodeAction::Stop).await?;
    Ok(())
}

pub(super) async fn cleanup_node_runtime(state: &AppState, node: &Value) -> Result<(), ApiError> {
    let node_type = node
        .get("node_type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let node_id = node
        .get("node_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match node_type {
        "docker" => match node.get("container_id").and_then(Value::as_str) {
            Some(container_id) => state
                .runtime
                .remove_node_resources(node_id, Some(container_id))
                .await
                .map_err(runtime_api_error),
            None => Ok(()),
        },
        "qemu" => {
            if node
                .get("runtime_overlay")
                .and_then(Value::as_str)
                .is_none()
            {
                return Ok(());
            }
            let vm = state.qemu.vm_from_node(node).map_err(runtime_api_error)?;
            state.qemu.remove_vm(&vm).await.map_err(runtime_api_error)
        }
        "vpcs" => state
            .vpcs
            .remove_node(
                node.get("project_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                node_id,
            )
            .await
            .map_err(runtime_api_error),
        "dynamips" => {
            if node
                .get("runtime_working_dir")
                .and_then(Value::as_str)
                .is_none()
                || node
                    .get("runtime_hypervisor_port")
                    .and_then(Value::as_u64)
                    .is_none()
            {
                return Ok(());
            }
            let vm = state
                .dynamips
                .vm_from_node(node)
                .map_err(runtime_api_error)?;
            state
                .dynamips
                .remove_router(&vm)
                .await
                .map_err(runtime_api_error)
        }
        "ethernet_switch" | "ethernet_hub" | "cloud" | "nat" => {
            state.builtin.stop_node(node_id).await;
            Ok(())
        }
        _ => Ok(()),
    }
}

pub(super) fn docker_interface_mac(
    node_id: &str,
    interface_number: u64,
) -> Result<String, ApiError> {
    let node_id =
        Uuid::parse_str(node_id).map_err(|_| ApiError::bad_request("Docker node ID is invalid"))?;
    let bytes = node_id.as_bytes();
    Ok(format!(
        "02:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], interface_number as u8
    ))
}

pub(super) async fn cleanup_link_runtime(
    state: &AppState,
    project_id: &str,
    link: &Value,
) -> Result<(), ApiError> {
    let runtime_endpoints = link_runtime_endpoints(state.store.pool(), project_id, link)
        .await
        .unwrap_or_default();
    for endpoint in link
        .get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let node_id = endpoint["node_id"].as_str().unwrap_or_default();
        if let Some(runtime_endpoint) = runtime_endpoints
            .iter()
            .find(|candidate| candidate["node_id"].as_str() == Some(node_id))
        {
            detach_runtime_link_endpoint(state, project_id, endpoint, runtime_endpoint).await?;
        } else {
            let _ = LinkEngine::new(state.ubridge.clone())
                .detach_endpoint(endpoint)
                .await;
            if let Some(endpoint_id) = endpoint["runtime_endpoint_id"].as_str()
                && let Ok(host) = ubridge::host_veth_name(endpoint_id)
            {
                let _ = state.ubridge.delete_veth(&host).await;
            }
        }
    }
    Ok(())
}
