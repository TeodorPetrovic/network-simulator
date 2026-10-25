use super::*;

pub(super) async fn get_drawing(
    State(state): State<AppState>,
    Path((project_id, drawing_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    get_topology_object(
        state.store.pool(),
        &project_id.to_string(),
        "drawing",
        &drawing_id.to_string(),
    )
    .await
}

pub(super) async fn update_drawing(
    State(state): State<AppState>,
    Path((project_id, drawing_id)): Path<(Uuid, Uuid)>,
    Json(patch): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    update_topology_object(
        state,
        project_id.to_string(),
        drawing_id.to_string(),
        "drawing",
        "drawing_id",
        patch,
    )
    .await
}

pub(super) async fn delete_drawing(
    State(state): State<AppState>,
    Path((project_id, drawing_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    delete_topology_object(
        state,
        project_id.to_string(),
        drawing_id.to_string(),
        "drawing",
    )
    .await
}

pub(super) async fn create_drawing(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Json(mut drawing): Json<Value>,
) -> Result<impl IntoResponse, ApiError> {
    let project_id = project_id.to_string();
    let object = as_object_mut(&mut drawing, "drawing body must be an object")?;
    let drawing_id = parse_or_generate_uuid(object.get("drawing_id"), "drawing_id")?;
    object.insert("drawing_id".into(), json!(drawing_id));
    object.insert("project_id".into(), json!(project_id));
    create_topology_object(state, &project_id, "drawing", "drawing_id", drawing).await
}

pub(super) async fn create_topology_object(
    state: AppState,
    project_id: &str,
    kind: &str,
    id_field: &str,
    mut value: Value,
) -> Result<Response, ApiError> {
    let object = value
        .as_object()
        .ok_or_else(|| ApiError::bad_request("topology object must be an object"))?;
    let object_id = object
        .get(id_field)
        .and_then(Value::as_str)
        .expect("create handlers fill generated IDs")
        .to_owned();
    let mut name = object
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut tx = state.store.begin_write().await?;
    ensure_project_exists(&mut tx, project_id).await?;
    let existing = sqlx::query(
        "SELECT object_json FROM topology_objects
         WHERE project_id = ? AND kind = ? AND object_id = ?",
    )
    .bind(project_id)
    .bind(kind)
    .bind(&object_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(row) = existing {
        let value = decode_json(row.get::<String, _>("object_json"))?;
        tx.commit().await?;
        return Ok(Json(value).into_response());
    }
    if kind == "node" {
        let strict_names = value.get("strict_names").and_then(Value::as_bool).unwrap_or(false);
        if let Some(base_name) = name.clone() {
            let duplicate: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM topology_objects WHERE project_id=? AND kind='node' AND name=?)")
                .bind(project_id).bind(&base_name).fetch_one(&mut *tx).await?;
            if duplicate != 0 && strict_names {
                return Err(ApiError::conflict(format!("Node name '{base_name}' is already used")));
            }
            if duplicate != 0 {
                let split = base_name.trim_end_matches(|ch: char| ch.is_ascii_digit()).len();
                let prefix = &base_name[..split];
                let initial = base_name[split..].parse::<u64>().unwrap_or(0) + 1;
                for suffix in initial..=u64::MAX {
                    let candidate = format!("{prefix}{suffix}");
                    let used: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM topology_objects WHERE project_id=? AND kind='node' AND name=?)")
                        .bind(project_id).bind(&candidate).fetch_one(&mut *tx).await?;
                    if used == 0 { name = Some(candidate.clone()); if let Some(object) = value.as_object_mut() { object.insert("name".into(), json!(candidate)); } break; }
                }
            }
        }
        if let Some(object) = value.as_object_mut() { object.remove("strict_names"); }
    }
    if kind == "link" {
        validate_link_in_tx(&mut tx, project_id, &value).await?;
    }
    sqlx::query(
        "INSERT INTO topology_objects (project_id, kind, object_id, name, object_json)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(project_id)
    .bind(kind)
    .bind(&object_id)
    .bind(name)
    .bind(serde_json::to_string(&value).map_err(internal_json_error)?)
    .execute(&mut *tx)
    .await?;
    if kind == "link" {
        store_link_endpoints_in_tx(&mut tx, project_id, &object_id, &value).await?;
    }
    let action = format!("{kind}.created");
    let event = Store::append_event(&mut tx, project_id, &action, &value).await?;
    tx.commit().await?;
    state.store.publish(event);
    Ok((StatusCode::CREATED, Json(value)).into_response())
}

pub(super) async fn get_topology_object(
    pool: &SqlitePool,
    project_id: &str,
    kind: &str,
    object_id: &str,
) -> Result<Json<Value>, ApiError> {
    let row = sqlx::query(
        "SELECT object_json FROM topology_objects
         WHERE project_id = ? AND kind = ? AND object_id = ?",
    )
    .bind(project_id)
    .bind(kind)
    .bind(object_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| ApiError::not_found(format!("{kind} {object_id} does not exist")))?;
    Ok(Json(decode_json(row.get::<String, _>("object_json"))?))
}

pub(super) async fn update_topology_object(
    state: AppState,
    project_id: String,
    object_id: String,
    kind: &str,
    id_field: &str,
    patch: Value,
) -> Result<Json<Value>, ApiError> {
    let patch_object = patch
        .as_object()
        .ok_or_else(|| ApiError::bad_request("topology update body must be an object"))?;
    let mut tx = state.store.begin_write().await?;
    ensure_project_exists(&mut tx, &project_id).await?;
    let row = sqlx::query(
        "SELECT object_json FROM topology_objects
         WHERE project_id = ? AND kind = ? AND object_id = ?",
    )
    .bind(&project_id)
    .bind(kind)
    .bind(&object_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::not_found(format!("{kind} {object_id} does not exist")))?;
    let mut value = decode_json(row.get::<String, _>("object_json"))?;
    {
        let object = as_object_mut(&mut value, "stored topology object is invalid")?;
        for (key, new_value) in patch_object {
            const NODE_READ_ONLY_FIELDS: &[&str] = &[
                "node_type",
                "compute_id",
                "template_id",
                "container_id",
                "image",
                "image_digest",
                "hda_disk_image",
                "runtime_overlay",
                "runtime_qmp_socket",
                "runtime_serial_socket",
                "runtime_pid_file",
                "runtime_log_file",
                "adapters",
                "memory",
                "cpus",
                "runtime_missing",
            ];
            if key != id_field
                && key != "project_id"
                && !(kind == "node" && NODE_READ_ONLY_FIELDS.contains(&key.as_str()))
            {
                object.insert(key.clone(), new_value.clone());
            }
        }
        object.insert(id_field.to_owned(), json!(object_id));
        object.insert("project_id".to_owned(), json!(project_id));
        if kind == "node" {
            let name = required_string(object, "name")?.to_owned();
            required_string(object, "node_type")?;
            object.insert("name".to_owned(), json!(name));
        }
    }
    if kind == "link" {
        sqlx::query("DELETE FROM link_endpoints WHERE project_id = ? AND link_id = ?")
            .bind(&project_id)
            .bind(&object_id)
            .execute(&mut *tx)
            .await?;
        validate_link_in_tx(&mut tx, &project_id, &value).await?;
    }
    let name = value.get("name").and_then(Value::as_str);
    sqlx::query(
        "UPDATE topology_objects SET name = ?, object_json = ?
         WHERE project_id = ? AND kind = ? AND object_id = ?",
    )
    .bind(name)
    .bind(serde_json::to_string(&value).map_err(internal_json_error)?)
    .bind(&project_id)
    .bind(kind)
    .bind(&object_id)
    .execute(&mut *tx)
    .await?;
    if kind == "link" {
        store_link_endpoints_in_tx(&mut tx, &project_id, &object_id, &value).await?;
    }
    let event =
        Store::append_event(&mut tx, &project_id, &format!("{kind}.updated"), &value).await?;
    tx.commit().await?;
    state.store.publish(event);
    Ok(Json(value))
}

pub(super) async fn delete_topology_object(
    state: AppState,
    project_id: String,
    object_id: String,
    kind: &str,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.store.begin_write().await?;
    ensure_project_exists(&mut tx, &project_id).await?;
    let row = sqlx::query(
        "SELECT object_json FROM topology_objects
         WHERE project_id = ? AND kind = ? AND object_id = ?",
    )
    .bind(&project_id)
    .bind(kind)
    .bind(&object_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::not_found(format!("{kind} {object_id} does not exist")))?;
    let value = decode_json(row.get::<String, _>("object_json"))?;
    if kind == "link" {
        sqlx::query("DELETE FROM link_endpoints WHERE project_id = ? AND link_id = ?")
            .bind(&project_id)
            .bind(&object_id)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query(
        "DELETE FROM topology_objects
         WHERE project_id = ? AND kind = ? AND object_id = ?",
    )
    .bind(&project_id)
    .bind(kind)
    .bind(&object_id)
    .execute(&mut *tx)
    .await?;
    let event =
        Store::append_event(&mut tx, &project_id, &format!("{kind}.deleted"), &value).await?;
    tx.commit().await?;
    state.store.publish(event);
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn delete_node_and_connected_links(
    state: AppState,
    project_id: String,
    node_id: String,
) -> Result<StatusCode, ApiError> {
    let _project_lock = state.lock_project_operations(&project_id).await;
    let mut tx = state.store.begin_write().await?;
    ensure_project_exists(&mut tx, &project_id).await?;
    let node_row = sqlx::query(
        "SELECT object_json FROM topology_objects
         WHERE project_id = ? AND kind = 'node' AND object_id = ?",
    )
    .bind(&project_id)
    .bind(&node_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::not_found(format!("Node {node_id} does not exist")))?;
    let node = decode_json(node_row.get::<String, _>("object_json"))?;
    if node.get("locked").and_then(Value::as_bool) == Some(true) {
        return Err(ApiError::conflict(format!("Node {node_id} is locked")));
    }

    let link_rows = sqlx::query(
        "SELECT DISTINCT o.object_id, o.object_json
         FROM topology_objects AS o
         JOIN link_endpoints AS e
           ON e.project_id = o.project_id AND e.link_id = o.object_id
         WHERE o.project_id = ? AND o.kind = 'link' AND e.node_id = ?",
    )
    .bind(&project_id)
    .bind(&node_id)
    .fetch_all(&mut *tx)
    .await?;
    for row in &link_rows {
        let link = decode_json(row.get::<String, _>("object_json"))?;
        cleanup_link_runtime(&state, &project_id, &link).await?;
    }
    cleanup_node_runtime(&state, &node).await?;
    let mut events = Vec::new();
    let mut removed_links = Vec::new();
    for row in link_rows {
        let link_id: String = row.get("object_id");
        let link = decode_json(row.get::<String, _>("object_json"))?;
        removed_links.push(link.clone());
        sqlx::query("DELETE FROM link_endpoints WHERE project_id = ? AND link_id = ?")
            .bind(&project_id)
            .bind(&link_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "DELETE FROM topology_objects
             WHERE project_id = ? AND kind = 'link' AND object_id = ?",
        )
        .bind(&project_id)
        .bind(&link_id)
        .execute(&mut *tx)
        .await?;
        events.push(Store::append_event(&mut tx, &project_id, "link.deleted", &link).await?);
    }
    sqlx::query("DELETE FROM node_files WHERE project_id = ? AND node_id = ?")
        .bind(&project_id)
        .bind(&node_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "DELETE FROM topology_objects
         WHERE project_id = ? AND kind = 'node' AND object_id = ?",
    )
    .bind(&project_id)
    .bind(&node_id)
    .execute(&mut *tx)
    .await?;
    events.push(Store::append_event(&mut tx, &project_id, "node.deleted", &node).await?);
    tx.commit().await?;
    let link_engine = LinkEngine::new(state.ubridge.clone());
    for link in &removed_links {
        link_engine.release_link_ports(link).await;
    }
    for event in events {
        state.store.publish(event);
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn get_topology_snapshot(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let project_id = project_id.to_string();
    let mut tx = state.store.begin_write().await?;
    let project_row = sqlx::query("SELECT project_json FROM projects WHERE project_id = ?")
        .bind(&project_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Project {project_id} does not exist")))?;
    let project = decode_json(project_row.get::<String, _>("project_json"))?;
    let nodes = list_objects_in_tx(&mut tx, &project_id, "node").await?;
    let links = list_objects_in_tx(&mut tx, &project_id, "link").await?;
    let drawings = list_objects_in_tx(&mut tx, &project_id, "drawing").await?;
    let cursor = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(sequence), 0) FROM project_events WHERE project_id = ?",
    )
    .bind(&project_id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    let nodes = refresh_node_statuses(&state, &project_id, nodes).await;
    Ok(Json(json!({
        "project": project,
        "nodes": nodes,
        "links": links,
        "drawings": drawings,
        "event_cursor": cursor
    })))
}
