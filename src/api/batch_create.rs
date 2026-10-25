use super::*;

pub(super) async fn create_nodes(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Json(nodes): Json<Vec<Value>>,
) -> Result<Json<Vec<Value>>, ApiError> {
    let mut results = Vec::with_capacity(nodes.len());
    for node in nodes {
        match node_templates::create_node(State(state.clone()), Path(project_id), Json(node)).await {
            Ok(response) => results.push(batch_result(response.into_response(), "node").await?),
            Err(error) => results.push(batch_error(error)),
        }
    }
    Ok(Json(results))
}

pub(super) async fn create_links(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Json(links): Json<Vec<Value>>,
) -> Result<Json<Vec<Value>>, ApiError> {
    let mut results = Vec::with_capacity(links.len());
    for link in links {
        match link_create::create_link(State(state.clone()), Path(project_id), Json(link)).await {
            Ok(response) => results.push(batch_result(response.into_response(), "link").await?),
            Err(error) => results.push(batch_error(error)),
        }
    }
    Ok(Json(results))
}

async fn batch_result(response: Response, key: &str) -> Result<Value, ApiError> {
    let status_code = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024).await
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Could not read batch result"))?;
    let value: Value = serde_json::from_slice(&bytes).map_err(internal_json_error)?;
    if status_code < 400 {
        Ok(json!({"status_code":status_code,key:value}))
    } else {
        Ok(json!({"status_code":status_code,"error":value}))
    }
}

fn batch_error(error: ApiError) -> Value {
    let status_code = error.status.as_u16();
    json!({"status_code":status_code,"error":{"code":error.code,"message":error.message}})
}
