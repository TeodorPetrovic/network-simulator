use super::*;

pub(super) async fn list_nodes(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Vec<Value>>, ApiError> {
    let project_id = project_id.to_string();
    let nodes = list_objects(state.store.pool(), &project_id, "node")
        .await?
        .0;
    Ok(Json(
        refresh_node_statuses(&state, &project_id, nodes).await,
    ))
}

pub(super) async fn list_links(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Vec<Value>>, ApiError> {
    list_objects(state.store.pool(), &project_id.to_string(), "link").await
}

pub(super) async fn list_drawings(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Vec<Value>>, ApiError> {
    list_objects(state.store.pool(), &project_id.to_string(), "drawing").await
}
