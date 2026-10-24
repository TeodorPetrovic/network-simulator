use super::*;

pub(super) async fn bulk_nodes(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<Uuid>,
    action: NodeAction,
) -> Result<StatusCode, ApiError> {
    let pid = project_id.to_string();
    let nodes = list_objects(state.store.pool(), &pid, "node").await?.0;
    let mut first_error = None;
    for node in nodes {
        if !bulk_node_action_applies(&node, action) {
            continue;
        }
        let id = node["node_id"]
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or_else(|| ApiError::bad_request("node has invalid ID"))?;
        if let Err(error) = change_node_runtime_state(state.clone(), project_id, id, action).await {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) fn bulk_node_action_applies(node: &Value, action: NodeAction) -> bool {
    let node_type = node
        .get("node_type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let status = node
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("stopped");

    match node_type {
        "ethernet_switch" | "ethernet_hub" => false,
        "cloud" | "nat" if !matches!(action, NodeAction::Start) => false,
        "vpcs" if matches!(action, NodeAction::Suspend) => false,
        "docker" | "qemu" | "vpcs" | "dynamips" | "cloud" | "nat" => match action {
            NodeAction::Start => status != "started",
            NodeAction::Stop => status == "started" || status == "suspended",
            NodeAction::Suspend | NodeAction::Reload => status == "started",
        },
        _ => false,
    }
}

pub(super) async fn start_all_nodes(
    State(s): State<AppState>,
    AxumPath(p): AxumPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    bulk_nodes(State(s), AxumPath(p), NodeAction::Start).await
}
pub(super) async fn stop_all_nodes(
    State(s): State<AppState>,
    AxumPath(p): AxumPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    bulk_nodes(State(s), AxumPath(p), NodeAction::Stop).await
}
pub(super) async fn suspend_all_nodes(
    State(s): State<AppState>,
    AxumPath(p): AxumPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    bulk_nodes(State(s), AxumPath(p), NodeAction::Suspend).await
}
pub(super) async fn reload_all_nodes(
    State(s): State<AppState>,
    AxumPath(p): AxumPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    bulk_nodes(State(s), AxumPath(p), NodeAction::Reload).await
}
pub(super) async fn reset_all_consoles(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    let pid = project_id.to_string();
    ensure_project_exists_pool(state.store.pool(), &pid).await?;
    Err(ApiError::new(
        StatusCode::NOT_IMPLEMENTED,
        "Resetting runtime consoles is not supported",
    ))
}

