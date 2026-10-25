use super::*;

pub(super) async fn link_runtime_endpoints(
    pool: &SqlitePool,
    project_id: &str,
    link: &Value,
) -> Result<Vec<Value>, ApiError> {
    let endpoints = link
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::bad_request("link must contain a nodes array"))?;
    let mut runtime_endpoints = Vec::with_capacity(endpoints.len());
    for endpoint in endpoints {
        let node_id = endpoint
            .get("node_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::bad_request("each link endpoint requires node_id"))?;
        let row = sqlx::query(
            "SELECT object_json FROM topology_objects
             WHERE project_id = ? AND kind = 'node' AND object_id = ?",
        )
        .bind(project_id)
        .bind(node_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| {
            ApiError::bad_request(format!("Node {node_id} does not exist in this project"))
        })?;
        let node = decode_json(row.get::<String, _>("object_json"))?;
        let node_type = node
            .get("node_type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let adapter = endpoint["adapter_number"].as_u64().unwrap_or_default();
        let port = endpoint["port_number"].as_u64().unwrap_or_default();
        let adapters = node.get("adapters").and_then(Value::as_u64).unwrap_or(8);
        let builtin_ports = node["ports_mapping"]
            .as_array()
            .map(Vec::len)
            .unwrap_or_default() as u64;
        let dynamips_slots = if node_type == "dynamips" {
            (0..=6).filter_map(|slot| {
                let key = format!("slot{slot}");
                node.get(&key).and_then(Value::as_str)
                    .and_then(crate::dynamips::module_port_count)
                    .map(|ports| (slot as u64, ports as u64))
            }).collect::<Vec<_>>()
        } else { Vec::new() };
        let port_capacity = if BuiltinRuntime::is_type(node_type) {
            builtin_ports
        } else if node_type == "docker" || node_type == "qemu" {
            adapters
        } else if node_type == "dynamips" {
            dynamips_slots.iter().map(|(_, ports)| ports).sum()
        } else {
            1
        };
        let valid_endpoint = match node_type {
            "docker" => adapter == 0 && port < adapters,
            "qemu" => adapter < adapters && port == 0,
            "vpcs" => adapter == 0 && port == 0,
            "dynamips" => dynamips_slots.iter().any(|(slot, ports)| *slot == adapter && port < *ports),
            "ethernet_switch" | "ethernet_hub" | "cloud" => adapter == 0 && port < builtin_ports,
            "nat" => adapter == 0 && port == 0 && builtin_ports == 1,
            _ => {
                return Err(ApiError::bad_request(format!(
                    "Node type '{node_type}' has no Ethernet runtime"
                )));
            }
        };
        if !valid_endpoint {
            return Err(ApiError::bad_request(format!(
                "Port {adapter}/{port} is outside the configured {port_capacity} {node_type} ports"
            )));
        }
        let mut runtime_endpoint = json!({
            "node_id": node_id,
            "node_type": node_type,
            "adapter_number": adapter,
            "port_number": port
        });
        if node_type == "docker" {
            runtime_endpoint["container_id"] =
                node.get("container_id").cloned().ok_or_else(|| {
                    ApiError::conflict(format!("Node {node_id} has no Docker container"))
                })?;
        }
        runtime_endpoints.push(runtime_endpoint);
    }
    Ok(runtime_endpoints)
}

pub(super) async fn load_topology_node(
    pool: &SqlitePool,
    project_id: &str,
    node_id: &str,
) -> Result<Value, ApiError> {
    let row = sqlx::query(
        "SELECT object_json FROM topology_objects
         WHERE project_id = ? AND kind = 'node' AND object_id = ?",
    )
    .bind(project_id)
    .bind(node_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| ApiError::not_found(format!("Node {node_id} does not exist")))?;
    decode_json(row.get::<String, _>("object_json"))
}

pub(super) async fn runtime_endpoint_status(
    state: &AppState,
    project_id: &str,
    endpoint: &Value,
) -> Result<String, ApiError> {
    let node_id = endpoint["node_id"].as_str().unwrap_or_default();
    let node = load_topology_node(state.store.pool(), project_id, node_id).await?;
    let node_type = endpoint["node_type"].as_str().unwrap_or_default();
    if BuiltinRuntime::is_type(node_type) {
        return Ok(node["status"].as_str().unwrap_or("started").to_owned());
    }
    let status = match node_type {
        "docker" => {
            state
                .runtime
                .container_status(endpoint["container_id"].as_str().unwrap_or_default())
                .await
        }
        "qemu" => {
            let vm = state.qemu.vm_from_node(&node).map_err(runtime_api_error)?;
            state.qemu.status(&vm).await
        }
        "vpcs" => state.vpcs.status(node_id).await,
        "dynamips" => {
            let vm = state
                .dynamips
                .vm_from_node(&node)
                .map_err(runtime_api_error)?;
            state.dynamips.status(&vm).await
        }
        node_type => {
            return Err(ApiError::bad_request(format!(
                "Unsupported node type '{node_type}'"
            )));
        }
    }
    .map_err(runtime_api_error)?;
    Ok(status)
}

pub(super) async fn attach_runtime_link_endpoint(
    state: &AppState,
    project_id: &str,
    link: &Value,
    endpoint: &Value,
    runtime_endpoint: &Value,
) -> Result<(), ApiError> {
    let node_id = endpoint["node_id"].as_str().unwrap_or_default();
    let node = load_topology_node(state.store.pool(), project_id, node_id).await?;
    let engine = LinkEngine::new(state.ubridge.clone());
    let filters = link_filters_for_endpoint(link, endpoint);
    let suspended = link["suspend"].as_bool().unwrap_or(false);
    if matches!(
        runtime_endpoint["node_type"].as_str(),
        Some("cloud" | "nat")
    ) {
        BuiltinRuntime::validate_host_interfaces(&node).map_err(ApiError::bad_request)?;
    }
    let endpoint_id = endpoint["runtime_endpoint_id"].as_str().unwrap_or_default();
    let relay_already_exists = state.ubridge.has_link_endpoint(endpoint_id).await;
    let capture_selected = link["capturing"].as_bool().unwrap_or(false)
        && link["capture_endpoint_id"].as_str() == Some(endpoint_id);
    let capture_needed = capture_selected && !relay_already_exists;
    let mut endpoint = endpoint.clone();
    match runtime_endpoint["node_type"].as_str().unwrap_or_default() {
        "docker" => {
            let adapter = state
                .runtime
                .adapter_nio(&endpoint)
                .map_err(runtime_api_error)?;
            let AdapterNio::Ethernet {
                host_interface,
                guest_interface,
            } = adapter.clone()
            else {
                return Err(ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Docker did not provide an Ethernet NIO",
                ));
            };
            let container_id = runtime_endpoint["container_id"]
                .as_str()
                .unwrap_or_default();
            let endpoint_id = endpoint["runtime_endpoint_id"].as_str().unwrap_or_default();
            let setup = async {
                if !state.ubridge.has_link_endpoint(endpoint_id).await {
                    let pid = state
                        .runtime
                        .container_pid(container_id)
                        .await
                        .map_err(runtime_api_error)?;
                    let _ = state.ubridge.delete_veth(&host_interface).await;
                    state
                        .ubridge
                        .create_veth_pair(&host_interface, &guest_interface)
                        .await
                        .map_err(runtime_api_error)?;
                    let port = endpoint["port_number"].as_u64().unwrap_or_default();
                    let mac = docker_interface_mac(node_id, port)?;
                    state
                        .ubridge
                        .set_docker_mac_address(&guest_interface, &mac)
                        .await
                        .map_err(runtime_api_error)?;
                    state
                        .ubridge
                        .set_interface_up(&guest_interface)
                        .await
                        .map_err(runtime_api_error)?;
                    state
                        .ubridge
                        .set_interface_up(&host_interface)
                        .await
                        .map_err(runtime_api_error)?;
                    state
                        .ubridge
                        .move_veth_to_container(&guest_interface, pid, &guest_interface)
                        .await
                        .map_err(runtime_api_error)?;
                }
                engine
                    .attach_endpoint(&endpoint, adapter, &filters, suspended, None)
                    .await
                    .map_err(runtime_api_error)
            }
            .await;
            if let Err(error) = setup {
                let _ = state.ubridge.delete_veth(&host_interface).await;
                return Err(error);
            }
            endpoint["runtime_host_veth"] = json!(host_interface);
        }
        "qemu" => {
            let vm = state.qemu.vm_from_node(&node).map_err(runtime_api_error)?;
            let adapter = state
                .qemu
                .adapter_nio(&endpoint)
                .map_err(runtime_api_error)?;
            let qemu_link = qemu_link_from_endpoint(&endpoint)?;
            attach_runtime_nio_with_rollback(
                !relay_already_exists,
                || async {
                    state
                        .qemu
                        .attach_nio(&vm, &qemu_link)
                        .await
                        .map_err(runtime_api_error)
                },
                || async {
                    engine
                        .attach_endpoint(&endpoint, adapter, &filters, suspended, None)
                        .await
                        .map_err(runtime_api_error)
                },
                || async {
                    if !relay_already_exists {
                        let _ = state.qemu.detach_nio(&vm, &qemu_link).await;
                    }
                },
            )
            .await?;
        }
        "vpcs" => {
            if let Some(port) = state.vpcs.adapter_udp_port(node_id).await {
                endpoint["runtime_adapter_port"] = json!(port);
            }
            if let Some(port) = state.vpcs.active_udp_port(node_id).await {
                endpoint["runtime_udp_port"] = json!(port);
            }
            let adapter = state
                .vpcs
                .adapter_nio(&endpoint)
                .map_err(runtime_api_error)?;
            engine
                .attach_endpoint(&endpoint, adapter, &filters, suspended, None)
                .await
                .map_err(runtime_api_error)?;
        }
        "dynamips" => {
            let vm = state
                .dynamips
                .vm_from_node(&node)
                .map_err(runtime_api_error)?;
            let adapter = state
                .dynamips
                .adapter_nio(&endpoint)
                .map_err(runtime_api_error)?;
            let AdapterNio::Udp {
                adapter_port,
                runtime_port,
            } = adapter
            else {
                unreachable!()
            };
            let adapter_number = endpoint["adapter_number"].as_u64().unwrap_or_default() as u32;
            let port_number = endpoint["port_number"].as_u64().unwrap_or_default() as u32;
            attach_runtime_nio_with_rollback(
                !relay_already_exists,
                || async {
                    state
                        .dynamips
                        .attach_nio(&vm, adapter_number, port_number, runtime_port, adapter_port)
                        .await
                        .map_err(runtime_api_error)
                },
                || async {
                    engine
                        .attach_endpoint(&endpoint, adapter, &filters, suspended, None)
                        .await
                        .map_err(runtime_api_error)
                },
                || async {
                    if !relay_already_exists {
                        let _ = state
                            .dynamips
                            .detach_nio(&vm, adapter_number, port_number)
                            .await;
                    }
                },
            )
            .await?;
        }
        "ethernet_switch" | "ethernet_hub" | "cloud" | "nat" => {
            let adapter =
                BuiltinRuntime::adapter_nio(&node, &endpoint).map_err(runtime_api_error)?;
            state
                .builtin
                .attach_switch_port(&node, &endpoint)
                .await
                .map_err(runtime_api_error)?;
            if let Err(error) = engine
                .attach_endpoint(&endpoint, adapter, &filters, suspended, None)
                .await
            {
                state
                    .builtin
                    .detach_switch_port(
                        node_id,
                        endpoint["port_number"].as_u64().unwrap_or_default() as u16,
                    )
                    .await;
                return Err(runtime_api_error(error));
            }
        }
        node_type => {
            return Err(ApiError::bad_request(format!(
                "Unsupported node type '{node_type}'"
            )));
        }
    }
    if capture_needed {
        let capture_file_name = safe_capture_file_name(
            link["capture_file_name"]
                .as_str()
                .ok_or_else(|| ApiError::conflict("Capturing link has no capture file name"))?,
        )?;
        let stored_path = link["capture_file_path"]
            .as_str()
            .ok_or_else(|| ApiError::conflict("Capturing link has no capture file path"))?;
        let capture_path = super::canonical_capture_file_path(
            project_id,
            &capture_file_name,
            Some(stored_path),
            false,
        )?;
        engine
            .start_capture(&endpoint, &capture_path.to_string_lossy())
            .await
            .map_err(runtime_api_error)?;
    }
    Ok(())
}

pub(super) async fn attach_runtime_nio_with_rollback<
    Attach,
    AttachFuture,
    Relay,
    RelayFuture,
    Cleanup,
    CleanupFuture,
>(
    cleanup_runtime_on_relay_failure: bool,
    attach: Attach,
    attach_relay: Relay,
    cleanup: Cleanup,
) -> Result<(), ApiError>
where
    Attach: FnOnce() -> AttachFuture,
    AttachFuture: std::future::Future<Output = Result<(), ApiError>>,
    Relay: FnOnce() -> RelayFuture,
    RelayFuture: std::future::Future<Output = Result<(), ApiError>>,
    Cleanup: FnOnce() -> CleanupFuture,
    CleanupFuture: std::future::Future<Output = ()>,
{
    attach().await?;
    if let Err(error) = attach_relay().await {
        if cleanup_runtime_on_relay_failure {
            cleanup().await;
        }
        return Err(error);
    }
    Ok(())
}

pub(super) fn link_filters_for_endpoint(link: &Value, endpoint: &Value) -> Value {
    let selected = link["filter_endpoint_id"].as_str().or_else(|| {
        link["nodes"]
            .as_array()
            .and_then(|nodes| nodes.first())
            .and_then(|first| first["runtime_endpoint_id"].as_str())
    });
    if selected == endpoint["runtime_endpoint_id"].as_str() {
        link.get("filters").cloned().unwrap_or_else(|| json!({}))
    } else {
        json!({})
    }
}

pub(super) async fn detach_runtime_link_endpoint(
    state: &AppState,
    project_id: &str,
    endpoint: &Value,
    runtime_endpoint: &Value,
) -> Result<(), ApiError> {
    let node_id = endpoint["node_id"].as_str().unwrap_or_default();
    let engine = LinkEngine::new(state.ubridge.clone());
    let engine_result = engine.detach_endpoint(endpoint).await;
    match runtime_endpoint["node_type"].as_str().unwrap_or_default() {
        "docker" => {
            let host = endpoint["runtime_host_veth"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| {
                    endpoint["runtime_endpoint_id"]
                        .as_str()
                        .and_then(|id| ubridge::host_veth_name(id).ok())
                });
            if let Some(host) = host {
                let _ = state.ubridge.delete_veth(&host).await;
            }
        }
        // Like GNS3, QEMU keeps its boot-time NIC and UDP tunnel; only the relay side is torn down.
        "qemu" => {}
        "vpcs" => {}
        "dynamips" => {
            let node = load_topology_node(state.store.pool(), project_id, node_id).await?;
            let vm = state
                .dynamips
                .vm_from_node(&node)
                .map_err(runtime_api_error)?;
            let _ = state
                .dynamips
                .detach_nio(
                    &vm,
                    endpoint["adapter_number"].as_u64().unwrap_or_default() as u32,
                    endpoint["port_number"].as_u64().unwrap_or_default() as u32,
                )
                .await;
        }
        "ethernet_switch" | "ethernet_hub" => {
            state
                .builtin
                .detach_switch_port(
                    node_id,
                    endpoint["port_number"].as_u64().unwrap_or_default() as u16,
                )
                .await;
        }
        _ => {}
    }
    engine_result.map_err(runtime_api_error)
}

pub(super) fn qemu_link_from_endpoint(endpoint: &Value) -> Result<QemuLink, ApiError> {
    Ok(QemuLink {
        adapter_number: endpoint["adapter_number"].as_u64().unwrap_or_default() as u32,
        adapter_port: endpoint["runtime_adapter_port"]
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .ok_or_else(|| ApiError::conflict("QEMU link has no adapter UDP port"))?,
        runtime_port: endpoint["runtime_udp_port"]
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .ok_or_else(|| ApiError::conflict("QEMU link has no runtime UDP port"))?,
    })
}

pub(super) async fn node_link_entries(
    state: &AppState,
    project_id: &str,
    node_id: &str,
) -> Result<Vec<(Value, Value, Value)>, ApiError> {
    let links = list_objects(state.store.pool(), project_id, "link")
        .await?
        .0;
    let mut entries = Vec::new();
    for link in links {
        let Some(endpoint) = link
            .get("nodes")
            .and_then(Value::as_array)
            .and_then(|nodes| {
                nodes
                    .iter()
                    .find(|endpoint| endpoint["node_id"].as_str() == Some(node_id))
            })
            .cloned()
        else {
            continue;
        };
        let runtime_endpoint = link_runtime_endpoints(state.store.pool(), project_id, &link)
            .await?
            .into_iter()
            .find(|candidate| candidate["node_id"].as_str() == Some(node_id))
            .ok_or_else(|| {
                ApiError::conflict(format!("Link endpoint for {node_id} has no runtime"))
            })?;
        entries.push((link, endpoint, runtime_endpoint));
    }
    Ok(entries)
}

pub(super) async fn attach_node_links(
    state: &AppState,
    project_id: &str,
    node_id: &str,
    before_start: bool,
) -> Result<(), ApiError> {
    let entries = node_link_entries(state, project_id, node_id).await?;
    for (link, endpoint, runtime_endpoint) in entries {
        let node_type = runtime_endpoint["node_type"].as_str().unwrap_or_default();
        if before_start && node_type == "vpcs" {
            let adapter_port = endpoint["runtime_adapter_port"]
                .as_u64()
                .and_then(|port| u16::try_from(port).ok())
                .ok_or_else(|| ApiError::conflict("VPCS link has no adapter UDP port"))?;
            let runtime_port = endpoint["runtime_udp_port"]
                .as_u64()
                .and_then(|port| u16::try_from(port).ok())
                .ok_or_else(|| ApiError::conflict("VPCS link has no runtime UDP port"))?;
            state
                .vpcs
                .set_nio_ports(node_id, adapter_port, runtime_port)
                .await;
        }
        if before_start && node_type == "dynamips" {
            // Dynamips binds its UDP NIO through the hypervisor before the IOS
            // process starts. The other runtimes are attached after startup.
            attach_runtime_link_endpoint(state, project_id, &link, &endpoint, &runtime_endpoint)
                .await?;
        }
        if !before_start && node_type != "dynamips" {
            attach_runtime_link_endpoint(
                state,
                project_id,
                &link,
                &endpoint,
                &runtime_endpoint,
            )
            .await?;
        }
    }
    Ok(())
}

pub(super) async fn detach_node_links(
    state: &AppState,
    project_id: &str,
    node_id: &str,
) -> Result<(), ApiError> {
    let entries = node_link_entries(state, project_id, node_id).await?;
    let mut first_error = None;
    for (_link, endpoint, runtime_endpoint) in entries.into_iter().rev() {
        if let Err(error) =
            detach_runtime_link_endpoint(state, project_id, &endpoint, &runtime_endpoint).await
        {
            first_error.get_or_insert(error);
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Store;
    use tempfile::TempDir;

    #[tokio::test]
    async fn dynamips_link_validation_uses_each_configured_slot_port_count() {
        let directory = TempDir::new().unwrap();
        let store = Store::connect(directory.path().join("links.sqlite").to_str().unwrap())
            .await
            .unwrap();
        let project_id = uuid::Uuid::new_v4().to_string();
        let node_id = uuid::Uuid::new_v4().to_string();
        let project = json!({"project_id":project_id,"name":"links","status":"opened"});
        sqlx::query("INSERT INTO projects(project_id,name,status,project_json) VALUES(?,?,'opened',?)")
            .bind(&project_id).bind("links").bind(project.to_string())
            .execute(store.pool()).await.unwrap();
        let node = json!({"node_id":node_id,"project_id":project_id,"node_type":"dynamips","slot0":"NM-1FE-TX","slot1":"NM-4T"});
        sqlx::query("INSERT INTO topology_objects(project_id,kind,object_id,name,object_json) VALUES(?,'node',?,'R1',?)")
            .bind(&project_id).bind(&node_id).bind(node.to_string())
            .execute(store.pool()).await.unwrap();

        let valid = json!({"nodes":[{"node_id":node_id,"adapter_number":1,"port_number":3}]});
        let endpoints = link_runtime_endpoints(store.pool(), &project_id, &valid).await.unwrap();
        assert_eq!(endpoints[0]["adapter_number"], 1);
        assert_eq!(endpoints[0]["port_number"], 3);

        let invalid = json!({"nodes":[{"node_id":node_id,"adapter_number":1,"port_number":4}]});
        let error = link_runtime_endpoints(store.pool(), &project_id, &invalid).await.unwrap_err();
        assert_eq!(error.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(error.message.contains("5 dynamips ports"));
    }
}
