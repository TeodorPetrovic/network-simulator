use super::*;

pub(super) async fn list_projects(
    State(state): State<AppState>,
    Query(query): Query<HashMap<String, String>>,
    axum::Extension(principal): axum::Extension<auth::Principal>,
) -> Result<Json<Vec<Value>>, ApiError> {
    let rows =
        sqlx::query("SELECT project_id, project_json FROM projects ORDER BY name, project_id")
            .fetch_all(state.store.pool())
            .await?;
    let mut projects = Vec::new();
    for row in rows {
        let project = decode_json(row.get::<String, _>("project_json"))?;
        let project_id: String = row.get("project_id");
        if auth::can_audit_project(state.store.pool(), &principal, &project_id).await
            && query.get("name").is_none_or(|name| project["name"] == *name)
        {
            projects.push(project);
        }
    }
    Ok(Json(projects))
}

pub(super) async fn create_project(
    State(state): State<AppState>,
    Json(mut project): Json<Value>,
) -> Result<impl IntoResponse, ApiError> {
    let (name, project_id) = {
        let object = as_object_mut(&mut project, "project body must be an object")?;
        let name = required_string(object, "name")?.to_owned();
        let project_id = parse_or_generate_uuid(object.get("project_id"), "project_id")?;

        object.insert("project_id".into(), json!(project_id));
        object.insert("name".into(), json!(name));
        set_default(object, "path", Value::Null);
        set_default(object, "auto_start", json!(false));
        set_default(object, "auto_open", json!(false));
        set_default(object, "auto_close", json!(true));
        set_default(object, "scene_height", json!(1000));
        set_default(object, "scene_width", json!(2000));
        set_default(object, "zoom", json!(100));
        set_default(object, "show_layers", json!(false));
        set_default(object, "snap_to_grid", json!(false));
        set_default(object, "show_grid", json!(false));
        set_default(object, "grid_size", json!(75));
        set_default(object, "drawing_grid_size", json!(25));
        set_default(object, "show_interface_labels", json!(true));
        set_default(object, "status", json!("opened"));
        set_default(object, "filename", json!(format!("{name}.gns3")));
        (name, project_id)
    };

    let project_json = serde_json::to_string(&project)
        .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let mut tx = state.store.begin_write().await?;
    sqlx::query(
        "INSERT INTO projects (project_id, name, status, project_json) VALUES (?, ?, 'opened', ?)",
    )
    .bind(&project_id)
    .bind(&name)
    .bind(project_json)
    .execute(&mut *tx)
    .await?;
    let event = Store::append_event(&mut tx, &project_id, "project.created", &project).await?;
    tx.commit().await?;
    state.store.publish(event);

    Ok((StatusCode::CREATED, Json(project)))
}

pub(super) async fn get_project(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    Ok(etag_response(fetch_project(state.store.pool(), &project_id.to_string()).await?))
}

pub(super) async fn update_project(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    headers: axum::http::HeaderMap,
    Json(patch): Json<Value>,
) -> Result<Response, ApiError> {
    let project_id = project_id.to_string();
    let mut tx = state.store.begin_write().await?;
    let row = sqlx::query("SELECT project_json FROM projects WHERE project_id = ?")
        .bind(&project_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Project {project_id} does not exist")))?;
    let mut project = decode_json(row.get::<String, _>("project_json"))?;
    check_if_match(&headers, &project)?;
    let patch_object = patch
        .as_object()
        .ok_or_else(|| ApiError::bad_request("project update body must be an object"))?;
    const MUTABLE_FIELDS: &[&str] = &[
        "name",
        "auto_close",
        "auto_open",
        "auto_start",
        "scene_height",
        "scene_width",
        "zoom",
        "show_layers",
        "snap_to_grid",
        "show_grid",
        "grid_size",
        "drawing_grid_size",
        "show_interface_labels",
        "variables",
        "supplier",
    ];
    let name = {
        let project_object = as_object_mut(&mut project, "stored project is invalid")?;
        for field in MUTABLE_FIELDS {
            if let Some(value) = patch_object.get(*field) {
                project_object.insert((*field).to_owned(), value.clone());
            }
        }
        let name = required_string(project_object, "name")?.to_owned();
        project_object.insert("name".to_owned(), json!(name));
        name
    };
    sqlx::query("UPDATE projects SET name = ?, project_json = ? WHERE project_id = ?")
        .bind(&name)
        .bind(serde_json::to_string(&project).map_err(internal_json_error)?)
        .bind(&project_id)
        .execute(&mut *tx)
        .await?;
    let event = Store::append_event(&mut tx, &project_id, "project.updated", &project).await?;
    tx.commit().await?;
    state.store.publish(event);
    Ok(etag_response(project))
}

pub(super) async fn delete_project(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let project_id = project_id.to_string();
    let _project_lock = state.lock_project_operations(&project_id).await;
    ensure_project_exists_pool(state.store.pool(), &project_id).await?;
    let links = list_objects(state.store.pool(), &project_id, "link")
        .await?
        .0;
    let nodes = list_objects(state.store.pool(), &project_id, "node")
        .await?
        .0;
    for node in &nodes {
        stop_runtime_node(&state, &project_id, node).await?;
    }
    for link in &links {
        cleanup_link_runtime(&state, &project_id, link).await?;
    }
    for node in &nodes {
        cleanup_node_runtime(&state, node).await?;
    }
    let mut tx = state.store.begin_write().await?;
    let row = sqlx::query("SELECT project_json FROM projects WHERE project_id = ?")
        .bind(&project_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Project {project_id} does not exist")))?;
    let project = decode_json(row.get::<String, _>("project_json"))?;
    let event = Store::append_event(&mut tx, &project_id, "project.deleted", &project).await?;
    sqlx::query("DELETE FROM projects WHERE project_id = ?")
        .bind(&project_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    let link_engine = LinkEngine::new(state.ubridge.clone());
    for link in &links {
        link_engine.release_link_ports(link).await;
    }
    state.store.publish(event);
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn open_project(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(open_project_lifecycle(state, project_id).await?))
}

pub(super) async fn open_project_lifecycle(
    state: AppState,
    project_id: Uuid,
) -> Result<Value, ApiError> {
    let project = set_project_status(state.clone(), project_id, "opened")
        .await?
        .0;
    if project["auto_start"].as_bool().unwrap_or(false) {
        controller_routes::bulk_nodes(
            axum::extract::State(state),
            axum::extract::Path(project_id),
            NodeAction::Start,
        )
        .await?;
    }
    Ok(project)
}

pub(super) async fn close_project(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    set_project_status(state, project_id, "closed").await
}

pub(super) async fn set_project_status(
    state: AppState,
    project_id: Uuid,
    status: &str,
) -> Result<Json<Value>, ApiError> {
    let project_id = project_id.to_string();
    let _project_lock = state.lock_project_operations(&project_id).await;
    let mut tx = state.store.begin_write().await?;
    let row = sqlx::query("SELECT project_json FROM projects WHERE project_id = ?")
        .bind(&project_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Project {project_id} does not exist")))?;
    let mut project = decode_json(row.get::<String, _>("project_json"))?;
    let mut node_events = Vec::new();
    let auto_start = status == "opened"
        && project
            .get("auto_start")
            .and_then(Value::as_bool)
            .unwrap_or(false);
    if status == "closed" {
        let node_rows = sqlx::query(
            "SELECT object_id, object_json FROM topology_objects
             WHERE project_id = ? AND kind = 'node'",
        )
        .bind(&project_id)
        .fetch_all(&mut *tx)
        .await?;
        for node_row in node_rows {
            let node_id: String = node_row.get("object_id");
            let mut node = decode_json(node_row.get::<String, _>("object_json"))?;
            let node_object = node.as_object_mut().ok_or_else(|| {
                ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Stored node is invalid")
            })?;
            if node_object.get("status").and_then(Value::as_str) == Some("started")
                || node_object.get("status").and_then(Value::as_str) == Some("suspended")
                || node_object.get("node_type").and_then(Value::as_str) == Some("qemu")
            {
                let runtime_status =
                    apply_node_lifecycle(&state, &project_id, &mut node, NodeAction::Stop).await?;
                let node_object = node.as_object_mut().ok_or_else(|| {
                    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Stored node is invalid")
                })?;
                node_object.insert("status".to_owned(), json!(runtime_status));
                sqlx::query(
                    "UPDATE topology_objects SET object_json = ?
                     WHERE project_id = ? AND kind = 'node' AND object_id = ?",
                )
                .bind(serde_json::to_string(&node).map_err(internal_json_error)?)
                .bind(&project_id)
                .bind(&node_id)
                .execute(&mut *tx)
                .await?;
                node_events
                    .push(Store::append_event(&mut tx, &project_id, "node.updated", &node).await?);
            }
        }
    } else if auto_start {
        let node_rows = sqlx::query(
            "SELECT object_id, object_json FROM topology_objects
             WHERE project_id = ? AND kind = 'node' ORDER BY object_id",
        )
        .bind(&project_id)
        .fetch_all(&mut *tx)
        .await?;
        for node_row in node_rows {
            let node_id: String = node_row.get("object_id");
            let mut node = decode_json(node_row.get::<String, _>("object_json"))?;
            if !controller_routes::bulk_node_action_applies(&node, NodeAction::Start) {
                continue;
            }
            let runtime_status =
                apply_node_lifecycle(&state, &project_id, &mut node, NodeAction::Start).await?;
            node["status"] = json!(runtime_status);
            sqlx::query(
                "UPDATE topology_objects SET object_json = ?
                 WHERE project_id = ? AND kind = 'node' AND object_id = ?",
            )
            .bind(serde_json::to_string(&node).map_err(internal_json_error)?)
            .bind(&project_id)
            .bind(&node_id)
            .execute(&mut *tx)
            .await?;
            node_events
                .push(Store::append_event(&mut tx, &project_id, "node.updated", &node).await?);
        }
    }
    let object = as_object_mut(&mut project, "stored project is invalid")?;
    if object.get("status").and_then(Value::as_str) == Some(status) {
        tx.commit().await?;
        return Ok(Json(project));
    }
    object.insert("status".into(), json!(status));
    sqlx::query("UPDATE projects SET status = ?, project_json = ? WHERE project_id = ?")
        .bind(status)
        .bind(serde_json::to_string(&project).map_err(internal_json_error)?)
        .bind(&project_id)
        .execute(&mut *tx)
        .await?;
    let event = Store::append_event(&mut tx, &project_id, "project.updated", &project).await?;
    tx.commit().await?;
    for event in node_events {
        state.store.publish(event);
    }
    state.store.publish(event);
    Ok(Json(project))
}
