use super::*;

pub(super) async fn delete_node(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    delete_node_and_connected_links(state, project_id.to_string(), node_id.to_string()).await
}

pub(super) async fn start_node(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    change_node_runtime_state(state, project_id, node_id, NodeAction::Start).await
}

pub(super) async fn stop_node(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    change_node_runtime_state(state, project_id, node_id, NodeAction::Stop).await
}

pub(super) async fn suspend_node(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    let node = get_topology_object(
        state.store.pool(),
        &project_id.to_string(),
        "node",
        &node_id.to_string(),
    )
    .await?
    .0;
    if node.get("node_type").and_then(Value::as_str) == Some("vpcs") {
        return Err(ApiError::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "VPCS nodes do not support suspend",
        ));
    }
    change_node_runtime_state(state, project_id, node_id, NodeAction::Suspend).await
}

pub(super) async fn reload_node(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    change_node_runtime_state(state, project_id, node_id, NodeAction::Reload).await
}

pub(super) async fn change_node_runtime_state(
    state: AppState,
    project_id: Uuid,
    node_id: Uuid,
    action: NodeAction,
) -> Result<Json<Value>, ApiError> {
    let project_id = project_id.to_string();
    let node_id = node_id.to_string();
    let _project_lock = state.lock_project_operations(&project_id).await;
    let mut node = get_topology_object(state.store.pool(), &project_id, "node", &node_id)
        .await?
        .0;
    let previous_status = node.get("status").and_then(Value::as_str).map(str::to_owned);
    let status = apply_node_lifecycle(&state, &project_id, &mut node, action).await?;
    node["status"] = json!(status);
    let mut tx = state.store.pool().begin().await?;
    sqlx::query(
        "UPDATE topology_objects SET object_json = ?
         WHERE project_id = ? AND kind = 'node' AND object_id = ?",
    )
    .bind(serde_json::to_string(&node).map_err(internal_json_error)?)
    .bind(&project_id)
    .bind(&node_id)
    .execute(&mut *tx)
    .await?;
    let event = if node_status_changed(previous_status.as_deref(), &status) {
        Some(Store::append_event(&mut tx, &project_id, "node.updated", &node).await?)
    } else {
        None
    };
    tx.commit().await?;
    if let Some(event) = event {
        state.store.publish(event);
    }
    Ok(Json(node))
}

fn node_status_changed(previous: Option<&str>, next: &str) -> bool {
    previous != Some(next)
}

#[cfg(test)]
mod tests {
    #[test]
    fn duplicate_node_status_updates_do_not_emit_events() {
        assert!(!super::node_status_changed(Some("started"), "started"));
        assert!(super::node_status_changed(Some("stopped"), "started"));
        assert!(super::node_status_changed(None, "started"));
    }
}
