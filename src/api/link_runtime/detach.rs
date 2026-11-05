use super::*;

pub(crate) async fn detach_runtime_link_endpoint(
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
        "qemu" => {
            let node = load_topology_node(state.store.pool(), project_id, node_id).await?;
            let vm = state.qemu.vm_from_node(&node).map_err(runtime_api_error)?;
            if vm.replicate_network_connection_state {
                let adapter = endpoint["adapter_number"].as_u64().unwrap_or_default() as u32;
                if let Err(error) = state.qemu.set_adapter_link(&vm, adapter, false).await {
                    tracing::warn!(
                        node_id,
                        adapter,
                        error = error.message(),
                        "Could not lower QEMU link carrier after detach"
                    );
                }
            }
        }
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

pub(crate) fn qemu_link_from_endpoint(endpoint: &Value, suspended: bool) -> Result<QemuLink, ApiError> {
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
        suspended,
    })
}

