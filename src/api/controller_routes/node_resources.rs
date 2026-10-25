use super::*;

pub(super) async fn duplicate_node(
    State(state): State<AppState>,
    AxumPath((project_id, node_id)): AxumPath<(Uuid, Uuid)>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let mut node = get_topology_object(
        state.store.pool(),
        &project_id.to_string(),
        "node",
        &node_id.to_string(),
    )
    .await?
    .0;
    if !node.is_object() {
        return Err(ApiError::bad_request("stored node is invalid"));
    }
    strip_node_runtime_fields(&mut node);
    node["status"] = json!("stopped");
    node["x"] = body
        .get("x")
        .cloned()
        .unwrap_or_else(|| json!(node["x"].as_i64().unwrap_or(0) + 50));
    node["y"] = body
        .get("y")
        .cloned()
        .unwrap_or_else(|| json!(node["y"].as_i64().unwrap_or(0) + 50));
    if let Some(z) = body.get("z") {
        node["z"] = z.clone();
    }
    let old_name = node["name"].as_str().unwrap_or("Node").to_owned();
    node["name"] = json!(format!("{old_name} copy"));
    as_object_mut(&mut node, "duplicated node must be an object")?
        .insert("node_id".into(), json!(Uuid::new_v4()));
    create_topology_object(state, &project_id.to_string(), "node", "node_id", node).await
}
pub(super) async fn set_node_isolated(
    State(state): State<AppState>,
    project_id: Uuid,
    node_id: Uuid,
    isolated: bool,
) -> Result<Json<Value>, ApiError> {
    get_topology_object(
        state.store.pool(),
        &project_id.to_string(),
        "node",
        &node_id.to_string(),
    )
    .await?;
    update_topology_object(
        state,
        project_id.to_string(),
        node_id.to_string(),
        "node",
        "node_id",
        json!({"isolated":isolated}),
    )
    .await
}
pub(super) async fn isolate_node(
    State(s): State<AppState>,
    AxumPath((p, n)): AxumPath<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    set_node_isolated(State(s), p, n, true).await
}
pub(super) async fn unisolate_node(
    State(s): State<AppState>,
    AxumPath((p, n)): AxumPath<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    set_node_isolated(State(s), p, n, false).await
}
pub(super) async fn reset_console(
    State(state): State<AppState>,
    AxumPath((project_id, node_id)): AxumPath<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    get_topology_object(
        state.store.pool(),
        &project_id.to_string(),
        "node",
        &node_id.to_string(),
    )
    .await?;
    Err(ApiError::new(
        StatusCode::NOT_IMPLEMENTED,
        "Resetting runtime consoles is not supported",
    ))
}
pub(super) async fn node_links(
    State(state): State<AppState>,
    AxumPath((project_id, node_id)): AxumPath<(Uuid, Uuid)>,
) -> Result<Json<Vec<Value>>, ApiError> {
    get_topology_object(
        state.store.pool(),
        &project_id.to_string(),
        "node",
        &node_id.to_string(),
    )
    .await?;
    let rows=sqlx::query("SELECT DISTINCT object_json FROM topology_objects l JOIN link_endpoints e ON e.project_id=l.project_id AND e.link_id=l.object_id WHERE l.project_id=? AND l.kind='link' AND e.node_id=? ORDER BY l.object_id").bind(project_id.to_string()).bind(node_id.to_string()).fetch_all(state.store.pool()).await?;
    rows.into_iter()
        .map(|row| decode_json(row.get("object_json")))
        .collect::<Result<Vec<_>, _>>()
        .map(Json)
}

pub(super) fn clean_relative_path(path: &str) -> Result<String, ApiError> {
    let path = path.trim_start_matches('/');
    if path.is_empty()
        || path
            .split('/')
            .any(|s| s.is_empty() || s == "." || s == "..")
        || path.contains('\0')
    {
        return Err(ApiError::bad_request(
            "file path must be a safe relative path",
        ));
    }
    Ok(path.to_owned())
}
#[derive(Deserialize)]
pub(super) struct FileQuery {
    path: Option<String>,
    recursive: Option<bool>,
}
pub(super) async fn list_node_files(
    State(state): State<AppState>,
    AxumPath((project_id, node_id)): AxumPath<(Uuid, Uuid)>,
    Query(query): Query<FileQuery>,
) -> Result<Json<Vec<Value>>, ApiError> {
    get_topology_object(
        state.store.pool(),
        &project_id.to_string(),
        "node",
        &node_id.to_string(),
    )
    .await?;
    let rows=sqlx::query("SELECT file_path,length(contents) AS size,updated_at FROM node_files WHERE project_id=? AND node_id=? ORDER BY file_path").bind(project_id.to_string()).bind(node_id.to_string()).fetch_all(state.store.pool()).await?;
    let prefix = query.path.unwrap_or_default().trim_matches('/').to_owned();
    let recursive = query.recursive.unwrap_or(false);
    let files=rows.into_iter().filter_map(|r|{let path:String=r.get("file_path");if !prefix.is_empty()&&!path.starts_with(&format!("{prefix}/")){return None;}if !recursive&&path.strip_prefix(&prefix).unwrap_or(&path).trim_start_matches('/').contains('/'){return None;}let timestamp=date_time(r.get::<String,_>("updated_at"));Some(json!({"path":path,"size":r.get::<i64,_>("size"),"created_at":timestamp,"modified_at":timestamp,"file_type":"file"}))}).collect();
    Ok(Json(files))
}

fn date_time(timestamp: String) -> String {
    if timestamp.ends_with('Z') || timestamp.contains('+') { return timestamp; }
    format!("{}Z", timestamp.replace(' ', "T"))
}

#[cfg(test)]
mod timestamp_tests {
    use super::date_time;

    #[test]
    fn node_file_timestamps_use_rfc3339_shape() {
        assert_eq!(date_time("2026-10-25 12:34:56".into()), "2026-10-25T12:34:56Z");
    }
}
pub(super) async fn get_node_file(
    State(state): State<AppState>,
    AxumPath((project_id, node_id, file_path)): AxumPath<(Uuid, Uuid, String)>,
) -> Result<Response, ApiError> {
    get_topology_object(
        state.store.pool(),
        &project_id.to_string(),
        "node",
        &node_id.to_string(),
    )
    .await?;
    let file = clean_relative_path(&file_path)?;
    let row = sqlx::query(
        "SELECT contents FROM node_files WHERE project_id=? AND node_id=? AND file_path=?",
    )
    .bind(project_id.to_string())
    .bind(node_id.to_string())
    .bind(file)
    .fetch_optional(state.store.pool())
    .await?
    .ok_or_else(|| ApiError::not_found("Node file does not exist"))?;
    Ok((
        [(header::CONTENT_TYPE, "application/octet-stream")],
        Body::from(row.get::<Vec<u8>, _>("contents")),
    )
        .into_response())
}
pub(super) async fn write_node_file(
    State(state): State<AppState>,
    AxumPath((project_id, node_id, file_path)): AxumPath<(Uuid, Uuid, String)>,
    request: Request,
) -> Result<Response, ApiError> {
    get_topology_object(
        state.store.pool(),
        &project_id.to_string(),
        "node",
        &node_id.to_string(),
    )
    .await?;
    let file = clean_relative_path(&file_path)?;
    let bytes = to_bytes(request.into_body(), 64 * 1024 * 1024)
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    sqlx::query("INSERT INTO node_files(project_id,node_id,file_path,contents) VALUES(?,?,?,?) ON CONFLICT(project_id,node_id,file_path) DO UPDATE SET contents=excluded.contents,updated_at=CURRENT_TIMESTAMP").bind(project_id.to_string()).bind(node_id.to_string()).bind(file).bind(bytes.to_vec()).execute(state.store.pool()).await?;
    Ok((StatusCode::CREATED, Json(Value::Null)).into_response())
}
pub(super) async fn delete_node_file(
    State(state): State<AppState>,
    AxumPath((project_id, node_id, file_path)): AxumPath<(Uuid, Uuid, String)>,
) -> Result<StatusCode, ApiError> {
    let file = clean_relative_path(&file_path)?;
    let r = sqlx::query("DELETE FROM node_files WHERE project_id=? AND node_id=? AND file_path=?")
        .bind(project_id.to_string())
        .bind(node_id.to_string())
        .bind(file)
        .execute(state.store.pool())
        .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found("Node file does not exist"));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn create_qemu_disk(
    State(state): State<AppState>,
    AxumPath((p, n, d)): AxumPath<(Uuid, Uuid, String)>,
    Json(body): Json<Value>,
) -> Result<StatusCode, ApiError> {
    qemu_disk_op(state, p, n, d, body, "create").await
}
pub(super) async fn resize_qemu_disk(
    State(state): State<AppState>,
    AxumPath((p, n, d)): AxumPath<(Uuid, Uuid, String)>,
    Json(body): Json<Value>,
) -> Result<StatusCode, ApiError> {
    qemu_disk_op(state, p, n, d, body, "resize").await
}
pub(super) async fn delete_qemu_disk(
    State(state): State<AppState>,
    AxumPath((p, n, d)): AxumPath<(Uuid, Uuid, String)>,
) -> Result<StatusCode, ApiError> {
    qemu_disk_op(state, p, n, d, json!({}), "delete").await
}
pub(super) async fn qemu_disk_op(
    state: AppState,
    project_id: Uuid,
    node_id: Uuid,
    disk_name: String,
    body: Value,
    operation: &str,
) -> Result<StatusCode, ApiError> {
    let node = get_topology_object(
        state.store.pool(),
        &project_id.to_string(),
        "node",
        &node_id.to_string(),
    )
    .await?
    .0;
    if node["node_type"].as_str() != Some("qemu") {
        return Err(ApiError::bad_request(
            "QEMU disk operations require a QEMU node",
        ));
    }
    if disk_name.is_empty()
        || disk_name.contains('/')
        || disk_name.contains('\\')
        || disk_name.starts_with('.')
    {
        return Err(ApiError::bad_request("disk_name must be a filename"));
    }
    if operation == "delete" {
        state
            .qemu
            .delete_disk_image(&project_id.to_string(), &node_id.to_string(), &disk_name)
            .map_err(runtime_api_error)?;
        return Ok(StatusCode::NO_CONTENT);
    }
    let size = body
        .get("size")
        .or_else(|| body.get("size_bytes"))
        .ok_or_else(|| ApiError::bad_request("size is required"))?;
    let size_string = if let Some(s) = size.as_str() {
        s.to_owned()
    } else if let Some(n) = size.as_u64() {
        n.to_string()
    } else {
        return Err(ApiError::bad_request(
            "size must be a string or nonnegative integer",
        ));
    };
    if operation == "create" {
        let image_options: std::collections::BTreeMap<String, String> = body
            .get("options")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|_| ApiError::bad_request("options must be a string-to-string object"))?
            .unwrap_or_default();
        state
            .qemu
            .create_disk_image_with_size(
                &project_id.to_string(),
                &node_id.to_string(),
                &disk_name,
                body["format"].as_str().unwrap_or("qcow2"),
                &size_string,
                &image_options,
            )
            .await
            .map_err(runtime_api_error)?;
    } else {
        state
            .qemu
            .resize_disk_image_to_size(
                &project_id.to_string(),
                &node_id.to_string(),
                &disk_name,
                &size_string,
                body["allow_shrink"].as_bool() == Some(true),
            )
            .await
            .map_err(runtime_api_error)?;
    }
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn auto_idlepc(
    State(state): State<AppState>,
    AxumPath((project_id, node_id)): AxumPath<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    let node = get_topology_object(
        state.store.pool(),
        &project_id.to_string(),
        "node",
        &node_id.to_string(),
    )
    .await?
    .0;
    if node["node_type"].as_str() != Some("dynamips") {
        return Err(ApiError::bad_request(
            "Dynamips operations require a Dynamips node",
        ));
    }
    let vm = state
        .dynamips
        .vm_from_node(&node)
        .map_err(runtime_api_error)?;
    let idlepc = state
        .dynamips
        .auto_idlepc(&vm)
        .await
        .map_err(runtime_api_error)?;
    let _ = update_topology_object(
        state,
        project_id.to_string(),
        node_id.to_string(),
        "node",
        "node_id",
        json!({"idlepc": idlepc.clone()}),
    )
    .await?;
    Ok(Json(json!({"idlepc":idlepc})))
}
pub(super) async fn idlepc_proposals(
    State(state): State<AppState>,
    AxumPath((project_id, node_id)): AxumPath<(Uuid, Uuid)>,
) -> Result<Json<Vec<String>>, ApiError> {
    let node = get_topology_object(
        state.store.pool(),
        &project_id.to_string(),
        "node",
        &node_id.to_string(),
    )
    .await?
    .0;
    if node["node_type"].as_str() != Some("dynamips") {
        return Err(ApiError::bad_request(
            "Dynamips operations require a Dynamips node",
        ));
    }
    let vm = state
        .dynamips
        .vm_from_node(&node)
        .map_err(runtime_api_error)?;
    let proposals = state
        .dynamips
        .idlepc_proposals(&vm)
        .await
        .map_err(runtime_api_error)?;
    Ok(Json(proposals))
}
