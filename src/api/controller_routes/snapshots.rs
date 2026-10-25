use super::*;

pub(super) async fn create_snapshot(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<Uuid>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let pid = project_id.to_string();
    let name = required_string(
        body.as_object()
            .ok_or_else(|| ApiError::bad_request("snapshot body must be an object"))?,
        "name",
    )?
    .to_owned();
    ensure_project_exists_pool(state.store.pool(), &pid).await?;
    let project = fetch_project(state.store.pool(), &pid).await?;
    let mut objects = Vec::new();
    for kind in ["node", "link", "drawing"] {
        for item in list_objects(state.store.pool(), &pid, kind).await?.0 {
            objects.push(json!({"kind":kind,"value":item}));
        }
    }
    let id = Uuid::new_v4().to_string();
    let snapshot = json!({"project":project,"objects":objects});
    let description = body["description"].as_str().unwrap_or("");
    sqlx::query("INSERT INTO project_snapshots(snapshot_id,project_id,name,description,topology_json) VALUES(?,?,?,?,?)").bind(&id).bind(&pid).bind(&name).bind(description).bind(serde_json::to_string(&snapshot).map_err(internal_json_error)?).execute(state.store.pool()).await?;
    let row=sqlx::query("SELECT strftime('%s',created_at) AS epoch, created_at FROM project_snapshots WHERE snapshot_id=?").bind(&id).fetch_one(state.store.pool()).await?;
    let answer = json!({"snapshot_id":id,"project_id":pid,"name":name,"description":description,"filename":format!("{id}.snapshot"),"created_at":row.get::<String,_>("epoch").parse::<i64>().unwrap_or(0),"created":date_time(row.get::<String,_>("created_at"))});
    Ok((StatusCode::CREATED, Json(answer)).into_response())
}
pub(super) async fn list_snapshots(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<Uuid>,
) -> Result<Json<Vec<Value>>, ApiError> {
    let pid = project_id.to_string();
    ensure_project_exists_pool(state.store.pool(), &pid).await?;
    let rows=sqlx::query("SELECT snapshot_id,name,description,created_at,strftime('%s',created_at) AS epoch FROM project_snapshots WHERE project_id=? ORDER BY created_at,name").bind(&pid).fetch_all(state.store.pool()).await?;
    Ok(Json(rows.into_iter().map(|r|{let id:String=r.get("snapshot_id");json!({"snapshot_id":id,"project_id":pid,"name":r.get::<String,_>("name"),"description":r.get::<String,_>("description"),"filename":format!("{id}.snapshot"),"created_at":r.get::<String,_>("epoch").parse::<i64>().unwrap_or(0),"created":date_time(r.get::<String,_>("created_at"))})}).collect()))
}

fn date_time(timestamp: String) -> String {
    if timestamp.ends_with('Z') || timestamp.contains('+') { timestamp } else { format!("{}Z", timestamp.replace(' ', "T")) }
}
pub(super) async fn delete_snapshot(
    State(state): State<AppState>,
    AxumPath((project_id, snapshot_id)): AxumPath<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let result = sqlx::query("DELETE FROM project_snapshots WHERE project_id=? AND snapshot_id=?")
        .bind(project_id.to_string())
        .bind(snapshot_id.to_string())
        .execute(state.store.pool())
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::not_found("Snapshot does not exist"));
    }
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn restore_snapshot(
    State(state): State<AppState>,
    AxumPath((project_id, snapshot_id)): AxumPath<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    let pid = project_id.to_string();
    let row = sqlx::query(
        "SELECT topology_json FROM project_snapshots WHERE project_id=? AND snapshot_id=?",
    )
    .bind(&pid)
    .bind(snapshot_id.to_string())
    .fetch_optional(state.store.pool())
    .await?
    .ok_or_else(|| ApiError::not_found("Snapshot does not exist"))?;
    let snapshot = decode_json(row.get("topology_json"))?;
    let mut tx = state.store.pool().begin().await?;
    sqlx::query("DELETE FROM link_endpoints WHERE project_id=?")
        .bind(&pid)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM topology_objects WHERE project_id=?")
        .bind(&pid)
        .execute(&mut *tx)
        .await?;
    if let Some(items) = snapshot["objects"].as_array() {
        for item in items {
            let kind = item["kind"].as_str().unwrap_or("");
            let mut value = item["value"].clone();
            if kind == "node" {
                value["status"] = json!("stopped");
            }
            let key = match kind {
                "node" => "node_id",
                "link" => "link_id",
                "drawing" => "drawing_id",
                _ => continue,
            };
            let id = value[key]
                .as_str()
                .ok_or_else(|| ApiError::bad_request("snapshot object has no ID"))?
                .to_owned();
            let name = value.get("name").and_then(Value::as_str).map(str::to_owned);
            sqlx::query("INSERT INTO topology_objects(project_id,kind,object_id,name,object_json) VALUES(?,?,?,?,?)").bind(&pid).bind(kind).bind(&id).bind(name).bind(serde_json::to_string(&value).map_err(internal_json_error)?).execute(&mut *tx).await?;
            if kind == "link" {
                store_link_endpoints_in_tx(&mut tx, &pid, &id, &value).await?;
            }
        }
    }
    let mut project = snapshot["project"].clone();
    project["project_id"] = json!(pid);
    let name = project["name"].as_str().unwrap_or("Project").to_owned();
    let status = project["status"].as_str().unwrap_or("opened").to_owned();
    sqlx::query("UPDATE projects SET name=?,status=?,project_json=? WHERE project_id=?")
        .bind(name)
        .bind(status)
        .bind(serde_json::to_string(&project).map_err(internal_json_error)?)
        .bind(&pid)
        .execute(&mut *tx)
        .await?;
    let event = Store::append_event(&mut tx, &pid, "snapshot.restored", &project).await?;
    tx.commit().await?;
    state.store.publish(event);
    Ok(Json(project))
}
