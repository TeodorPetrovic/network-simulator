use super::*;

pub(super) async fn controller_statistics(
    State(state): State<AppState>,
) -> Result<Json<Value>, ApiError> {
    let project_counts =
        sqlx::query("SELECT status, COUNT(*) AS total FROM projects GROUP BY status")
            .fetch_all(state.store.pool())
            .await?;
    let mut total = 0_i64;
    let mut opened = 0_i64;
    let mut closed = 0_i64;
    for row in project_counts {
        let status: String = row.get("status");
        let count: i64 = row.get("total");
        total += count;
        if status == "opened" {
            opened += count;
        }
        if status == "closed" {
            closed += count;
        }
    }
    let totals = sqlx::query("SELECT kind, COUNT(*) AS total FROM topology_objects GROUP BY kind")
        .fetch_all(state.store.pool())
        .await?;
    let mut nodes = 0_i64;
    let mut links = 0_i64;
    for row in totals {
        match row.get::<String, _>("kind").as_str() {
            "node" => nodes = row.get::<i64, _>("total"),
            "link" => links = row.get::<i64, _>("total"),
            _ => {}
        }
    }
    let node_rows = sqlx::query(
        "SELECT COALESCE(json_extract(object_json, '$.node_type'), 'unknown') AS node_type,
                COALESCE(json_extract(object_json, '$.status'), 'unknown') AS status,
                COUNT(*) AS total
         FROM topology_objects WHERE kind='node'
         GROUP BY node_type, status",
    )
    .fetch_all(state.store.pool())
    .await?;
    let mut by_type = serde_json::Map::new();
    let mut by_status = serde_json::Map::new();
    for row in node_rows {
        let node_type: String = row.get("node_type");
        let status: String = row.get("status");
        let count: i64 = row.get("total");
        let type_count = by_type.entry(node_type).or_insert_with(|| json!(0_i64));
        *type_count = json!(type_count.as_i64().unwrap_or_default() + count);
        let status_count = by_status.entry(status).or_insert_with(|| json!(0_i64));
        *status_count = json!(status_count.as_i64().unwrap_or_default() + count);
    }
    let captures: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM topology_objects
         WHERE kind='link' AND COALESCE(json_extract(object_json, '$.capturing'), 0) = 1",
    )
    .fetch_one(state.store.pool())
    .await?;
    let wireshark_containers: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM topology_objects
         WHERE kind='link' AND COALESCE(json_extract(object_json, '$.wireshark'), 0) = 1",
    )
    .fetch_one(state.store.pool())
    .await?;
    let qemu = state.qemu.info().await;
    let docker = state.runtime.info().await;
    Ok(Json(json!({
        "uptime_seconds": state.started_at.elapsed().as_secs(),
        "computes": [{"compute_id":"local","compute_name":"Local","statistics":{"qemu":qemu,"docker":docker}}],
        "projects":{"total":total,"opened":opened,"closed":closed},
        "nodes":{"total":nodes,"by_type":by_type,"by_status":by_status},
        "links":{"total":links,"capturing":captures},
        "webwireshark":{"containers":wireshark_containers}
    })))
}
