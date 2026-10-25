use super::*;

pub(super) async fn list_nodes(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Vec<Value>>, ApiError> {
    let project_id = project_id.to_string();
    let nodes = list_objects(state.store.pool(), &project_id, "node")
        .await?
        .0;
    let nodes = refresh_node_statuses(&state, &project_id, nodes).await;
    Ok(Json(filter_named(nodes, query.get("name"))))
}

pub(super) async fn list_links(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Vec<Value>>, ApiError> {
    let links = list_objects(state.store.pool(), &project_id.to_string(), "link").await?.0;
    Ok(Json(filter_named(links, query.get("name"))))
}

pub(super) async fn list_drawings(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Vec<Value>>, ApiError> {
    let drawings = list_objects(state.store.pool(), &project_id.to_string(), "drawing").await?.0;
    Ok(Json(filter_named(drawings, query.get("name"))))
}

fn filter_named(values: Vec<Value>, name: Option<&String>) -> Vec<Value> {
    values.into_iter().filter(|value| name.is_none_or(|name| value.get("name").and_then(Value::as_str) == Some(name.as_str()))).collect()
}

#[cfg(test)]
mod name_filter_tests {
    use super::filter_named;
    use serde_json::json;

    #[test]
    fn name_filter_is_exact_and_case_sensitive() {
        let values = vec![json!({"name":"Router"}), json!({"name":"Router 1"}), json!({"name":"router"})];
        assert_eq!(filter_named(values, Some(&"Router".to_owned())), vec![json!({"name":"Router"})]);
    }
}
