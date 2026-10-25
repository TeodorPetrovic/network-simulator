use super::*;

pub(super) async fn project_stats(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let project_id = project_id.to_string();
    ensure_project_exists_pool(state.store.pool(), &project_id).await?;
    let mut counts = HashMap::<String, i64>::new();
    for row in sqlx::query(
        "SELECT kind, COUNT(*) AS total FROM topology_objects WHERE project_id = ? GROUP BY kind",
    )
    .bind(&project_id)
    .fetch_all(state.store.pool())
    .await?
    {
        counts.insert(row.get("kind"), row.get("total"));
    }
    let snapshots: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM project_snapshots WHERE project_id = ?")
            .bind(&project_id)
            .fetch_one(state.store.pool())
            .await?;
    Ok(Json(
        json!({"nodes":counts.get("node").copied().unwrap_or(0),"links":counts.get("link").copied().unwrap_or(0),"drawings":counts.get("drawing").copied().unwrap_or(0),"snapshots":snapshots,"markers":0}),
    ))
}

pub(super) async fn project_locked(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<Uuid>,
) -> Result<Json<bool>, ApiError> {
    let project = fetch_project(state.store.pool(), &project_id.to_string()).await?;
    Ok(Json(project["locked"].as_bool().unwrap_or(false)))
}
pub(super) async fn lock_project(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    set_project_lock(state, project_id, true).await?;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn unlock_project(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    set_project_lock(state, project_id, false).await?;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn set_project_lock(
    state: AppState,
    project_id: Uuid,
    locked: bool,
) -> Result<(), ApiError> {
    let pid = project_id.to_string();
    let mut tx = state.store.begin_write().await?;
    let row = sqlx::query("SELECT project_json FROM projects WHERE project_id=?")
        .bind(&pid)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::not_found("Project does not exist"))?;
    let mut project = decode_json(row.get("project_json"))?;
    project["locked"] = json!(locked);
    sqlx::query("UPDATE projects SET project_json=? WHERE project_id=?")
        .bind(serde_json::to_string(&project).map_err(internal_json_error)?)
        .bind(&pid)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE topology_objects SET object_json=json_set(object_json,'$.locked',?) WHERE project_id=? AND kind IN ('node','drawing')").bind(i64::from(locked)).bind(&pid).execute(&mut *tx).await?;
    let event = Store::append_event(&mut tx, &pid, "project.updated", &project).await?;
    tx.commit().await?;
    state.store.publish(event);
    Ok(())
}

pub(super) async fn project_gns3file(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let pid = project_id.to_string();
    let project = fetch_project(state.store.pool(), &pid).await?;
    let nodes = list_objects(state.store.pool(), &pid, "node").await?.0;
    let links = list_objects(state.store.pool(), &pid, "link").await?.0;
    let drawings = list_objects(state.store.pool(), &pid, "drawing").await?.0;
    Ok(Json(project_api_to_gns3(project, nodes, links, drawings)))
}

pub(super) fn project_api_to_gns3(
    mut project: Value,
    nodes: Vec<Value>,
    links: Vec<Value>,
    drawings: Vec<Value>,
) -> Value {
    if let Some(object) = project.as_object_mut() {
        for key in [
            "project_id",
            "nodes",
            "links",
            "drawings",
            "path",
            "status",
            "filename",
        ] {
            object.remove(key);
        }
    }
    let version = project
        .get("version")
        .cloned()
        .unwrap_or_else(|| json!("3.1.0"));
    project["version"] = version;
    project["topology"] = json!({
        "computes": [],
        "nodes": nodes.into_iter().map(api_node_to_gns3).collect::<Vec<_>>(),
        "links": links.into_iter().map(|mut link| {
            strip_link_runtime_fields(&mut link);
            if let Some(object) = link.as_object_mut() {
                object.remove("project_id");
            }
            link
        }).collect::<Vec<_>>(),
        "drawings": drawings.into_iter().map(|mut drawing| {
            if let Some(object) = drawing.as_object_mut() {
                object.remove("project_id");
            }
            drawing
        }).collect::<Vec<_>>()
    });
    strip_project_runtime_fields(&mut project);
    project
}

pub(super) fn api_node_to_gns3(mut node: Value) -> Value {
    strip_node_runtime_fields(&mut node);
    node["status"] = json!("stopped");
    node["compute_id"] = json!("local");
    let mut properties = node
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(object) = node.as_object_mut() {
        object.remove("properties");
        object.remove("project_id");
        object.remove("node_directory");
        object.remove("runtime_node_directory");
        let common = [
            "node_id",
            "node_type",
            "name",
            "compute_id",
            "x",
            "y",
            "z",
            "status",
            "console",
            "console_type",
            "console_auto_start",
            "console_resolution",
            "console_http_port",
            "console_http_path",
            "aux",
            "aux_type",
            "locked",
            "template_id",
            "usage",
        ];
        object.retain(|key, value| {
            if common.contains(&key.as_str()) {
                true
            } else {
                properties.insert(key.clone(), value.clone());
                false
            }
        });
        object.insert("properties".into(), Value::Object(properties));
    }
    node
}

pub(super) fn normalize_imported_project(mut project: Value) -> Value {
    if let Some(topology) = project.get("topology").cloned() {
        project["nodes"] = topology.get("nodes").cloned().unwrap_or_else(|| json!([]));
        project["links"] = topology.get("links").cloned().unwrap_or_else(|| json!([]));
        project["drawings"] = topology
            .get("drawings")
            .cloned()
            .unwrap_or_else(|| json!([]));
        project
            .as_object_mut()
            .map(|object| object.remove("topology"));
    }
    if let Some(nodes) = project.get_mut("nodes").and_then(Value::as_array_mut) {
        for node in nodes {
            node["status"] = json!("stopped");
            let properties = node.get("properties").and_then(Value::as_object).cloned();
            if let Some(properties) = properties
                && let Some(node) = node.as_object_mut()
            {
                for (key, value) in properties {
                    node.entry(key).or_insert(value);
                }
                node.insert("properties".into(), json!({}));
            }
        }
    }
    project
}

pub(super) fn package_project_images(
    project: &mut Value,
) -> Result<Vec<(String, Vec<u8>)>, ApiError> {
    let Some(nodes) = project
        .get_mut("topology")
        .and_then(|topology| topology.get_mut("nodes"))
        .and_then(Value::as_array_mut)
    else {
        return Ok(Vec::new());
    };
    let mut files = Vec::new();
    let mut seen = HashMap::<(String, PathBuf), String>::new();
    let mut archive_names = HashSet::<(String, String)>::new();
    let mut expanded_size = 0_u64;
    for node in nodes {
        let node_type = node
            .get("node_type")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let Some(properties) = node.get_mut("properties").and_then(Value::as_object_mut) else {
            continue;
        };
        let (image_type, archive_directory, image_fields, image_root) = match node_type.as_str() {
            "qemu" => (
                "qemu",
                "QEMU",
                &[
                    "hda_disk_image",
                    "hdb_disk_image",
                    "hdc_disk_image",
                    "hdd_disk_image",
                    "cdrom_image",
                ][..],
                crate::image_paths::qemu_images(),
            ),
            "dynamips" => (
                "dynamips",
                "IOS",
                &["image"][..],
                crate::image_paths::dynamips_images(),
            ),
            _ => continue,
        };
        let root = image_root.canonicalize().map_err(|error| {
            ApiError::not_found(format!(
                "Cannot include {image_type} images because image directory '{}' is unavailable: {error}",
                image_root.display()
            ))
        })?;
        for field in image_fields {
            let Some(reference) = properties.get(*field).and_then(Value::as_str) else {
                continue;
            };
            if reference.trim().is_empty() {
                continue;
            }
            let candidate = if Path::new(reference).is_absolute() {
                PathBuf::from(reference)
            } else {
                root.join(reference)
            };
            let source = candidate.canonicalize().map_err(|error| {
                ApiError::not_found(format!(
                    "Cannot include {image_type} image '{reference}': {error}"
                ))
            })?;
            if !source.starts_with(&root) || !source.is_file() {
                return Err(ApiError::bad_request(format!(
                    "Cannot include image '{reference}' outside the configured {image_type} image directory"
                )));
            }
            let file_name = source
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| ApiError::bad_request("Image file name is not valid UTF-8"))?
                .to_owned();
            let key = (image_type.to_owned(), source.clone());
            let archive_name = if let Some(existing) = seen.get(&key) {
                existing.clone()
            } else {
                let archive_name =
                    unique_project_image_name(&file_name, &image_type, &mut archive_names);
                let contents = std::fs::read(&source).map_err(|error| {
                    ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Could not read image '{}': {error}", source.display()),
                    )
                })?;
                expanded_size = expanded_size.saturating_add(contents.len() as u64);
                if expanded_size > MAX_PROJECT_ARCHIVE_FILES_BYTES {
                    return Err(ApiError::new(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        "Project images exceed the 128 MiB export size limit",
                    ));
                }
                files.push((
                    format!("images/{archive_directory}/{archive_name}"),
                    contents,
                ));
                seen.insert(key, archive_name.clone());
                archive_name
            };
            properties.insert((*field).to_owned(), json!(archive_name));
        }
    }
    Ok(files)
}

pub(super) fn unique_project_image_name(
    file_name: &str,
    image_type: &str,
    used: &mut HashSet<(String, String)>,
) -> String {
    let key = (image_type.to_owned(), file_name.to_owned());
    if used.insert(key) {
        return file_name.to_owned();
    }
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("image");
    let extension = path.extension().and_then(|value| value.to_str());
    for index in 2..=1000 {
        let candidate = match extension {
            Some(extension) => format!("{stem}-{index}.{extension}"),
            None => format!("{stem}-{index}"),
        };
        if used.insert((image_type.to_owned(), candidate.clone())) {
            return candidate;
        }
    }
    // A topology cannot practically reach this many same-name collisions, but
    // the suffix remains deterministic if it does.
    format!("{}-{}", stem, Uuid::new_v4())
}

pub(super) async fn get_project_file(
    State(state): State<AppState>,
    AxumPath((project_id, file_path)): AxumPath<(Uuid, String)>,
) -> Result<Response, ApiError> {
    let pid = project_id.to_string();
    ensure_project_exists_pool(state.store.pool(), &pid).await?;
    let file = clean_relative_path(&file_path)?;
    let data = sqlx::query("SELECT contents FROM project_files WHERE project_id=? AND file_path=?")
        .bind(pid)
        .bind(file)
        .fetch_optional(state.store.pool())
        .await?
        .ok_or_else(|| ApiError::not_found("Project file does not exist"))?;
    Ok((
        [(header::CONTENT_TYPE, "application/octet-stream")],
        Body::from(data.get::<Vec<u8>, _>("contents")),
    )
        .into_response())
}
pub(super) async fn write_project_file(
    State(state): State<AppState>,
    AxumPath((project_id, file_path)): AxumPath<(Uuid, String)>,
    request: Request,
) -> Result<StatusCode, ApiError> {
    let pid = project_id.to_string();
    ensure_project_exists_pool(state.store.pool(), &pid).await?;
    let file = clean_relative_path(&file_path)?;
    let bytes = to_bytes(request.into_body(), 64 * 1024 * 1024)
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    sqlx::query("INSERT INTO project_files(project_id,file_path,contents) VALUES(?,?,?) ON CONFLICT(project_id,file_path) DO UPDATE SET contents=excluded.contents, updated_at=CURRENT_TIMESTAMP").bind(pid).bind(file).bind(bytes.to_vec()).execute(state.store.pool()).await?;
    Ok(StatusCode::NO_CONTENT)
}
