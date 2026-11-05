use super::*;

pub(crate) async fn link_runtime_endpoints(
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

pub(crate) async fn load_topology_node(
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

pub(crate) async fn runtime_endpoint_status(
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


mod attach;
mod detach;
mod node_links;

pub(crate) use attach::*;
pub(crate) use detach::*;
pub(crate) use node_links::*;

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
