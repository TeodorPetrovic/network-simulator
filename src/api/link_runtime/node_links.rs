use super::*;

pub(crate) async fn node_link_entries(
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

pub(crate) async fn attach_node_links(
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

pub(crate) async fn detach_node_links(
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
