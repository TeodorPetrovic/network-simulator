use super::*;

pub(super) async fn list_objects(
    pool: &SqlitePool,
    project_id: &str,
    kind: &str,
) -> Result<Json<Vec<Value>>, ApiError> {
    ensure_project_exists_pool(pool, project_id).await?;
    let rows = sqlx::query(
        "SELECT object_json FROM topology_objects
         WHERE project_id = ? AND kind = ? ORDER BY name, object_id",
    )
    .bind(project_id)
    .bind(kind)
    .fetch_all(pool)
    .await?;
    let values = rows
        .into_iter()
        .map(|row| decode_json(row.get::<String, _>("object_json")))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(values))
}

pub(super) async fn refresh_node_statuses(
    state: &AppState,
    project_id: &str,
    mut nodes: Vec<Value>,
) -> Vec<Value> {
    if nodes.is_empty() {
        return nodes;
    }
    let statuses = state.runtime.project_node_states(project_id).await.ok();
    let mut qemu_queries = Vec::new();
    let mut dynamips_queries = Vec::new();
    for (index, node) in nodes.iter_mut().enumerate() {
        let Some(node_id) = node.get("node_id").and_then(Value::as_str) else {
            continue;
        };
        match node.get("node_type").and_then(Value::as_str) {
            Some("docker") if node.get("container_id").is_some() => {
                let Some(statuses) = statuses.as_ref() else {
                    continue;
                };
                match statuses.get(node_id) {
                    Some(status) => {
                        node["status"] = json!(status);
                        node["runtime_missing"] = json!(false);
                    }
                    None => {
                        node["status"] = json!("stopped");
                        node["runtime_missing"] = json!(true);
                    }
                }
            }
            Some("qemu") => match state.qemu.vm_from_node(node) {
                Ok(vm) if !vm.overlay.is_file() => {
                    node["status"] = json!("stopped");
                    node["runtime_missing"] = json!(true);
                }
                Ok(vm) => qemu_queries.push((index, node_id.to_owned(), vm)),
                Err(error) => {
                    tracing::debug!(error = %error.message(), %project_id, %node_id, "QEMU node metadata is invalid")
                }
            },
            Some("vpcs") => match state.vpcs.status(node_id).await {
                Ok(status) => {
                    node["status"] = json!(status);
                    node["runtime_missing"] = json!(false);
                }
                Err(error) => {
                    tracing::debug!(error = %error.message(), %project_id, %node_id, "VPCS node state refresh was unavailable")
                }
            },
            Some("dynamips") => match state.dynamips.vm_from_node(node) {
                Ok(vm) => dynamips_queries.push((index, node_id.to_owned(), vm)),
                Err(error) => {
                    tracing::debug!(error = %error.message(), %project_id, %node_id, "Dynamips node metadata is invalid")
                }
            },
            _ => {}
        }
    }
    let qemu_results = futures_util::stream::iter(qemu_queries)
        .map(|(index, node_id, vm)| async move { (index, node_id, state.qemu.status(&vm).await) })
        .buffer_unordered(16)
        .collect::<Vec<_>>()
        .await;
    for (index, node_id, result) in qemu_results {
        match result {
            Ok(status) => {
                nodes[index]["status"] = json!(status);
                nodes[index]["runtime_missing"] = json!(false);
            }
            Err(error) => {
                tracing::debug!(error = %error.message(), %project_id, %node_id, "QEMU state refresh was unavailable")
            }
        }
    }
    let dynamips_results =
        futures_util::stream::iter(dynamips_queries)
            .map(|(index, node_id, vm)| async move {
                (index, node_id, state.dynamips.status(&vm).await)
            })
            .buffer_unordered(16)
            .collect::<Vec<_>>()
            .await;
    for (index, node_id, result) in dynamips_results {
        match result {
            Ok(status) => {
                nodes[index]["status"] = json!(status);
                nodes[index]["runtime_missing"] = json!(false);
            }
            Err(error) => {
                nodes[index]["status"] = json!("stopped");
                nodes[index]["runtime_missing"] = json!(false);
                tracing::debug!(error = %error.message(), %project_id, %node_id, "Dynamips state refresh was unavailable")
            }
        }
    }
    nodes
}

pub(super) async fn list_objects_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    project_id: &str,
    kind: &str,
) -> Result<Vec<Value>, ApiError> {
    let rows = sqlx::query(
        "SELECT object_json FROM topology_objects
         WHERE project_id = ? AND kind = ? ORDER BY name, object_id",
    )
    .bind(project_id)
    .bind(kind)
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter()
        .map(|row| decode_json(row.get::<String, _>("object_json")))
        .collect()
}
