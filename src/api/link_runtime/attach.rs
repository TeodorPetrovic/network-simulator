use super::*;

pub(crate) async fn attach_runtime_link_endpoint(
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
            let qemu_link = qemu_link_from_endpoint(&endpoint, suspended)?;
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
            if vm.replicate_network_connection_state {
                if let Err(error) = state
                    .qemu
                    .set_adapter_link(&vm, qemu_link.adapter_number, !suspended)
                    .await
                {
                    tracing::warn!(
                        node_id,
                        adapter = qemu_link.adapter_number,
                        error = error.message(),
                        "Could not synchronize QEMU link carrier after attach"
                    );
                }
            }
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

pub(crate) async fn attach_runtime_nio_with_rollback<
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

pub(crate) fn link_filters_for_endpoint(link: &Value, endpoint: &Value) -> Value {
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

