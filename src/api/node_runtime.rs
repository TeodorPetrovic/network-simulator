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
            // GNS3 refuses to start a Docker node without uBridge; here a node without links still
            // starts (with no NICs, as before), so the init shim must not wait for any.
            let adapters = if state.ubridge.binary_available() {
                docker_adapter_count(node)
            } else {
                if matches!(action, NodeAction::Start | NodeAction::Reload) {
                    tracing::warn!(node_id, "uBridge is not installed; Docker adapters are not created");
                }
                0
            };
            // A container that is already running (or paused) has been through init.sh.
            let was_running = matches!(
                state
                    .runtime
                    .container_status(container_id)
                    .await
                    .as_deref(),
                Ok("started" | "suspended")
            );
            if matches!(action, NodeAction::Start | NodeAction::Reload) {
                // GNS3 waits for every adapter (GNS3_MAX_ETHERNET), linked or not.
                state
                    .runtime
                    .write_expected_adapters(
                        project_id,
                        node_id,
                        adapters,
                        !was_running || matches!(action, NodeAction::Reload),
                    )
                    .map_err(runtime_api_error)?;
                // "Edit network configuration": files written through the node file API
                // are applied by the init shim (ifup) when the container boots.
                if let Some(row) = sqlx::query(
                    "SELECT contents FROM node_files WHERE project_id = ? AND node_id = ? AND file_path = 'etc/network/interfaces'",
                )
                .bind(project_id)
                .bind(node_id)
                .fetch_optional(state.store.pool())
                .await?
                {
                    let contents: Vec<u8> = row.get("contents");
                    state
                        .runtime
                        .write_network_config(project_id, node_id, &contents)
                        .map_err(runtime_api_error)?;
                }
            }
            let status = state
                .runtime
                .lifecycle(container_id, action)
                .await
                .map_err(runtime_api_error)?;
            if matches!(action, NodeAction::Start | NodeAction::Reload) {
                let fresh_start = !was_running || matches!(action, NodeAction::Reload);
                let attached = match create_docker_adapters(
                    state,
                    node_id,
                    container_id,
                    adapters,
                    !fresh_start,
                )
                .await
                {
                    Ok(()) => attach_node_links(state, project_id, node_id, false).await,
                    Err(error) => Err(error),
                };
                if let Err(error) = attached {
                    let _ = detach_node_links(state, project_id, node_id).await;
                    let _ = state
                        .runtime
                        .lifecycle(container_id, NodeAction::Stop)
                        .await;
                    remove_docker_adapters(state, node_id, adapters).await;
                    return Err(error);
                }
            }
            if matches!(action, NodeAction::Stop) {
                // The veths die with the container's network namespace; this only sweeps leftovers.
                remove_docker_adapters(state, node_id, adapters).await;
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
            let mut saved_script = None;
            if matches!(action, NodeAction::Stop | NodeAction::Reload) {
                // `save` in the VPCS console rewrites startup.vpc. Keep it in the node so the next
                // start (which regenerates the file) boots with the addresses the user configured.
                let current = vpcs_startup_script(state.store.pool(), project_id, node).await?;
                saved_script =
                    state
                        .vpcs
                        .saved_startup_script(project_id, node_id, name, current.as_deref());
            }
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
            if let Some(saved) = saved_script {
                node["startup_script"] = json!(saved);
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
        "iou" => Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "IOU template and node data are supported, but the IOU runtime binary is not installed",
        )),
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

pub(super) fn docker_adapter_count(node: &Value) -> u64 {
    node.get("adapters").and_then(Value::as_u64).unwrap_or(1)
}

/// GNS3 `DockerVM.start()` calls `_add_ubridge_connection(nio, adapter)` for every adapter,
/// linked or not, right after `docker start`: each adapter gets a veth pair whose guest end
/// becomes `eth{N}` in the container (init.sh waits for all of them) and whose host end stays
/// down (no carrier) until a link attaches to it.
pub(super) async fn create_docker_adapters(
    state: &AppState,
    node_id: &str,
    container_id: &str,
    adapters: u64,
    container_initialized: bool,
) -> Result<(), ApiError> {
    if adapters == 0 {
        return Ok(());
    }
    let pid = state
        .runtime
        .container_pid(container_id)
        .await
        .map_err(runtime_api_error)?;
    for port in 0..adapters {
        ensure_docker_adapter(
            state,
            node_id,
            container_id,
            pid,
            port,
            container_initialized,
        )
        .await?;
    }
    Ok(())
}

/// Make sure adapter `port` of a running container exists as `eth{port}`; returns the veth
/// host end. An existing pair is reused untouched (its guest end is inside the container).
/// `container_initialized`: init.sh has already brought the NICs up, so a NIC created now
/// must be brought up in the container explicitly.
pub(super) async fn ensure_docker_adapter(
    state: &AppState,
    node_id: &str,
    container_id: &str,
    pid: u32,
    port: u64,
    container_initialized: bool,
) -> Result<String, ApiError> {
    let (host, guest) =
        ubridge::docker_adapter_veth_names(node_id, port).map_err(runtime_api_error)?;
    if ubridge::host_interface_exists(&host) && !ubridge::host_interface_exists(&guest) {
        return Ok(host);
    }
    // A pair whose guest end is still in the root namespace is from a failed start.
    let _ = state.ubridge.delete_veth(&host).await;
    let _ = state.ubridge.delete_veth(&guest).await;
    let guest_name = format!("eth{port}");
    let created = async {
        let mac = docker_interface_mac(node_id, port)?;
        state
            .ubridge
            .create_veth_pair(&host, &guest)
            .await
            .map_err(runtime_api_error)?;
        state
            .ubridge
            .set_interface_down(&host)
            .await
            .map_err(runtime_api_error)?;
        state
            .ubridge
            .set_docker_mac_address(&guest, &mac)
            .await
            .map_err(runtime_api_error)?;
        state
            .ubridge
            .move_veth_to_container(&guest, pid, &guest_name)
            .await
            .map_err(runtime_api_error)
    }
    .await;
    if let Err(error) = created {
        let _ = state.ubridge.delete_veth(&host).await;
        return Err(error);
    }
    if container_initialized
        && let Err(error) = state
            .runtime
            .set_container_interface_up(container_id, &guest_name)
            .await
    {
        tracing::warn!(
            node_id,
            interface = guest_name,
            error = error.message(),
            "Could not bring a late Docker NIC up"
        );
    }
    Ok(host)
}

pub(super) async fn remove_docker_adapters(state: &AppState, node_id: &str, adapters: u64) {
    for port in 0..adapters {
        if let Ok((host, guest)) = ubridge::docker_adapter_veth_names(node_id, port) {
            for interface in [host, guest] {
                if ubridge::host_interface_exists(&interface) {
                    let _ = state.ubridge.delete_veth(&interface).await;
                }
            }
        }
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
