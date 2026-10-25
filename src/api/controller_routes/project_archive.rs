use super::*;

pub(super) async fn export_project(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<Uuid>,
    Query(query): Query<ProjectExportQuery>,
) -> Result<Response, ApiError> {
    let pid = project_id.to_string();
    let mut project = project_gns3file(State(state.clone()), AxumPath(project_id))
        .await?
        .0;
    strip_project_runtime_fields(&mut project);
    if sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM topology_objects
         WHERE project_id = ? AND kind = 'node'
           AND json_extract(object_json, '$.status') IN ('started', 'suspended')",
    )
    .bind(&pid)
    .fetch_one(state.store.pool())
    .await?
        > 0
    {
        return Err(ApiError::conflict(
            "Project must be stopped in order to export it",
        ));
    }
    let project_file_bytes: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(length(contents)), 0) FROM project_files WHERE project_id=?",
    )
    .bind(&pid)
    .fetch_one(state.store.pool())
    .await?;
    if project_file_bytes as u64 > MAX_PROJECT_ARCHIVE_FILES_BYTES {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Project files exceed the 128 MiB export size limit",
        ));
    }
    let rows = sqlx::query(
        "SELECT file_path, contents FROM project_files WHERE project_id=? ORDER BY file_path",
    )
    .bind(&pid)
    .fetch_all(state.store.pool())
    .await?;
    let mut files = Vec::with_capacity(rows.len());
    for row in rows {
        let file_path: String = row.get("file_path");
        files.push((
            format!("project-files/{file_path}"),
            row.get::<Vec<u8>, _>("contents"),
        ));
    }
    if query.include_images {
        let image_files = package_project_images(&mut project)?;
        files.extend(image_files);
    }
    let archive_files_size = files
        .iter()
        .map(|(_, contents)| contents.len() as u64)
        .sum::<u64>();
    if archive_files_size > MAX_PROJECT_ARCHIVE_FILES_BYTES {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Project files and images exceed the 128 MiB export size limit",
        ));
    }
    let topology = serde_json::to_vec(&project).map_err(internal_json_error)?;
    if topology.len() as u64 > MAX_PROJECT_FILE_BYTES {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Project topology exceeds the 64 MiB export size limit",
        ));
    }
    let bytes = write_project_archive(topology, files)?;
    let mut response = Response::new(Body::from(bytes));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/gns3project"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=project.gns3project"),
    );
    Ok(response)
}

#[derive(Default, serde::Deserialize)]
pub(super) struct ProjectExportQuery {
    #[serde(default)]
    include_images: bool,
}

pub(super) async fn import_project(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<Uuid>,
    Query(query): Query<HashMap<String, String>>,
    principal: Option<axum::Extension<auth::Principal>>,
    request: Request,
) -> Result<Response, ApiError> {
    let bytes = to_bytes(request.into_body(), 64 * 1024 * 1024)
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let ImportedProjectArchive {
        project: mut project_data,
        project_files,
        images,
    } = read_project_archive(&bytes)?;
    let name = query
        .get("name")
        .cloned()
        .or_else(|| project_data["name"].as_str().map(str::to_owned))
        .unwrap_or_else(|| "Imported project".into());
    let mut project = project_data.clone();
    project["project_id"] = json!(project_id);
    project["name"] = json!(name);
    project["path"] = Value::Null;
    project["status"] = json!("opened");
    project["locked"] = json!(false);
    if let Some(axum::Extension(principal)) = principal {
        let username: String = sqlx::query_scalar("SELECT username FROM auth_users WHERE user_id=?")
            .bind(principal.user_id)
            .fetch_one(state.store.pool())
            .await?;
        project["created_by"] = json!(username);
    }
    let image_map = install_imported_project_images(&images)?;
    remap_imported_project_images(&mut project, &image_map);
    regenerate_imported_project_ids(&mut project, &project_id.to_string());
    strip_project_runtime_fields(&mut project);
    project_data = project.clone();
    save_imported_project(state, project_id, &project_data, &project, &project_files).await?;
    Ok((StatusCode::CREATED, Json(project)).into_response())
}

pub(super) async fn load_project(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let path = body
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad_request("path is required"))?;
    let path_metadata = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|e| ApiError::not_found(format!("Could not read project file: {e}")))?;
    if !path_metadata.file_type().is_file() {
        return Err(ApiError::bad_request(
            "Project path must refer to a regular file",
        ));
    }
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| ApiError::not_found(format!("Could not read project file: {e}")))?;
    let metadata = file
        .metadata()
        .await
        .map_err(|e| ApiError::not_found(format!("Could not inspect project file: {e}")))?;
    if !metadata.is_file() {
        return Err(ApiError::bad_request(
            "Project path must refer to a regular file",
        ));
    }
    if metadata.len() > MAX_PROJECT_FILE_BYTES {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Project files must be 64 MiB or smaller",
        ));
    }
    let mut data = Vec::with_capacity(metadata.len() as usize);
    let bytes_read = file
        .take(MAX_PROJECT_FILE_BYTES + 1)
        .read_to_end(&mut data)
        .await
        .map_err(|e| ApiError::bad_request(format!("Could not read project file: {e}")))?;
    if bytes_read as u64 > MAX_PROJECT_FILE_BYTES {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Project files must be 64 MiB or smaller",
        ));
    }
    let ImportedProjectArchive {
        project: mut project,
        project_files,
        images,
    } = read_project_archive(&data)?;
    let project_id = Uuid::new_v4();
    let mut saved = project.clone();
    saved["project_id"] = json!(project_id);
    saved["path"] = Value::Null;
    saved["status"] = json!("opened");
    saved["locked"] = json!(false);
    let image_map = install_imported_project_images(&images)?;
    remap_imported_project_images(&mut saved, &image_map);
    regenerate_imported_project_ids(&mut saved, &project_id.to_string());
    strip_project_runtime_fields(&mut saved);
    project = saved.clone();
    save_imported_project(state, project_id, &project, &saved, &project_files).await?;
    Ok((StatusCode::CREATED, Json(saved)).into_response())
}

pub(super) async fn save_imported_project(
    state: AppState,
    project_id: Uuid,
    source: &Value,
    project: &Value,
    project_files: &[(String, Vec<u8>)],
) -> Result<(), ApiError> {
    let pid = project_id.to_string();
    let mut clean_project = project.clone();
    strip_project_runtime_fields(&mut clean_project);
    let name = required_string(
        clean_project
            .as_object()
            .ok_or_else(|| ApiError::bad_request("project file must contain an object"))?,
        "name",
    )?;
    let mut tx = state.store.pool().begin().await?;
    sqlx::query("INSERT INTO projects(project_id,name,status,project_json) VALUES(?,?,?,?)")
        .bind(&pid)
        .bind(name)
        .bind("opened")
        .bind(serde_json::to_string(&clean_project).map_err(internal_json_error)?)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    // Use the same node constructors as the HTTP create route. They allocate
    // QEMU overlays, Docker containers, and Dynamips runtime manifests that a
    // raw database insert would leave missing when a student opens the lab.
    let mut created_node_ids = Vec::new();
    let import_result: Result<(), ApiError> = async {
        if let Some(nodes) = source.get("nodes").and_then(Value::as_array) {
            for item in nodes {
                let node_type = item
                    .get("node_type")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ApiError::bad_request("Imported node has no node_type"))?;
                let node_id = item
                    .get("node_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ApiError::bad_request("Imported node has no node_id"))?
                    .to_owned();
                if matches!(node_type, "docker" | "qemu" | "vpcs" | "dynamips")
                    || crate::builtin::BuiltinRuntime::is_type(node_type)
                {
                    super::create_node(
                        State(state.clone()),
                        AxumPath(project_id),
                        Json(item.clone()),
                    )
                    .await?;
                } else {
                    // Retain unfamiliar GNS3 appliance records so the rest of
                    // a project remains portable even when this build has no
                    // runtime adapter for that node type.
                    let mut value = item.clone();
                    value["project_id"] = json!(pid);
                    value["status"] = json!("stopped");
                    strip_node_runtime_fields(&mut value);
                    let node_name = value.get("name").and_then(Value::as_str).map(str::to_owned);
                    sqlx::query("INSERT INTO topology_objects(project_id,kind,object_id,name,object_json) VALUES(?,?,?,?,?)")
                        .bind(&pid)
                        .bind("node")
                        .bind(&node_id)
                        .bind(node_name)
                        .bind(serde_json::to_string(&value).map_err(internal_json_error)?)
                        .execute(state.store.pool())
                        .await?;
                }
                created_node_ids.push(node_id);
            }
        }

        let mut tx = state.store.pool().begin().await?;
        for (key, kind, id_field) in [("links", "link", "link_id"), ("drawings", "drawing", "drawing_id")] {
            if let Some(items) = source.get(key).and_then(Value::as_array) {
                for item in items {
                    let mut value = item.clone();
                    let id = value[id_field]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| Uuid::new_v4().to_string());
                    value[id_field] = json!(id);
                    value["project_id"] = json!(pid);
                    if kind == "link" {
                        strip_link_runtime_fields(&mut value);
                    }
                    let object_name = value.get("name").and_then(Value::as_str).map(str::to_owned);
                    sqlx::query("INSERT INTO topology_objects(project_id,kind,object_id,name,object_json) VALUES(?,?,?,?,?)")
                        .bind(&pid)
                        .bind(kind)
                        .bind(&id)
                        .bind(object_name)
                        .bind(serde_json::to_string(&value).map_err(internal_json_error)?)
                        .execute(&mut *tx)
                        .await?;
                    if kind == "link" {
                        store_link_endpoints_in_tx(&mut tx, &pid, &id, &value).await?;
                    }
                }
            }
        }
        for (file_path, contents) in project_files {
            let file_path = clean_relative_path(file_path)?;
            sqlx::query("INSERT INTO project_files(project_id,file_path,contents) VALUES(?,?,?)")
                .bind(&pid)
                .bind(file_path)
                .bind(contents)
                .execute(&mut *tx)
                .await?;
        }
        let event = Store::append_event(&mut tx, &pid, "project.created", &clean_project).await?;
        tx.commit().await?;
        state.store.publish(event);
        Ok(())
    }
    .await;
    if let Err(error) = import_result {
        rollback_imported_project(&state, &pid, &created_node_ids).await;
        return Err(error);
    }
    Ok(())
}

pub(super) async fn rollback_imported_project(
    state: &AppState,
    project_id: &str,
    node_ids: &[String],
) {
    for node_id in node_ids {
        if let Err(error) = super::delete_node_and_connected_links(
            state.clone(),
            project_id.to_owned(),
            node_id.clone(),
        )
        .await
        {
            tracing::error!(%project_id, %node_id, error = %error.message, "could not clean up a partially imported node");
        }
    }
    if let Err(error) = sqlx::query("DELETE FROM projects WHERE project_id = ?")
        .bind(project_id)
        .execute(state.store.pool())
        .await
    {
        tracing::error!(%project_id, %error, "could not remove a partially imported project");
    }
}

pub(super) async fn duplicate_project(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<Uuid>,
    Json(body): Json<Value>,
) -> Result<Response, ApiError> {
    let pid = project_id.to_string();
    let old = fetch_project(state.store.pool(), &pid).await?;
    let mut new_project = old.clone();
    let new_id = Uuid::new_v4().to_string();
    new_project["project_id"] = json!(new_id);
    new_project["name"] = json!(
        body.get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{} copy", old["name"].as_str().unwrap_or("Project")))
    );
    new_project["path"] = Value::Null;
    new_project["status"] = json!("opened");
    new_project["locked"] = json!(false);
    strip_project_runtime_fields(&mut new_project);
    let rows=sqlx::query("SELECT kind,object_id,object_json FROM topology_objects WHERE project_id=? ORDER BY kind,object_id").bind(&pid).fetch_all(state.store.pool()).await?;
    let mut ids = HashMap::new();
    for row in &rows {
        ids.insert(
            row.get::<String, _>("object_id"),
            Uuid::new_v4().to_string(),
        );
    }
    let mut tx = state.store.pool().begin().await?;
    let name = new_project["name"].as_str().unwrap_or("Project");
    sqlx::query("INSERT INTO projects(project_id,name,status,project_json) VALUES(?,?,?,?)")
        .bind(&new_id)
        .bind(name)
        .bind("opened")
        .bind(serde_json::to_string(&new_project).map_err(internal_json_error)?)
        .execute(&mut *tx)
        .await?;
    for row in rows {
        let kind: String = row.get("kind");
        let old_id: String = row.get("object_id");
        let mut object = decode_json(row.get("object_json"))?;
        let new_object_id = ids[&old_id].clone();
        let id_key = match kind.as_str() {
            "node" => "node_id",
            "link" => "link_id",
            _ => "drawing_id",
        };
        object[id_key] = json!(new_object_id);
        object["project_id"] = json!(new_id);
        if kind == "node" {
            strip_node_runtime_fields(&mut object);
            object["status"] = json!("stopped");
            object["locked"] = json!(false);
        }
        if kind == "link" {
            if let Some(nodes) = object["nodes"].as_array_mut() {
                for endpoint in nodes {
                    if let Some(id) = endpoint
                        .get("node_id")
                        .and_then(Value::as_str)
                        .and_then(|id| ids.get(id))
                    {
                        endpoint["node_id"] = json!(id);
                    }
                }
                strip_link_runtime_fields(&mut object);
            }
        }
        let obj_name = object
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_owned);
        sqlx::query("INSERT INTO topology_objects(project_id,kind,object_id,name,object_json) VALUES(?,?,?,?,?)").bind(&new_id).bind(&kind).bind(&new_object_id).bind(obj_name).bind(serde_json::to_string(&object).map_err(internal_json_error)?).execute(&mut *tx).await?;
        if kind == "link" {
            store_link_endpoints_in_tx(&mut tx, &new_id, &new_object_id, &object).await?;
        }
    }
    sqlx::query("INSERT INTO project_files(project_id,file_path,contents) SELECT ?,file_path,contents FROM project_files WHERE project_id=?")
        .bind(&new_id).bind(&pid).execute(&mut *tx).await?;
    let event = Store::append_event(&mut tx, &new_id, "project.created", &new_project).await?;
    tx.commit().await?;
    state.store.publish(event);
    Ok((StatusCode::CREATED, Json(new_project)).into_response())
}
