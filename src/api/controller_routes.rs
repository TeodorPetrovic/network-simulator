use std::{
    collections::HashMap,
    io::{self, Cursor, Read, Write},
    path::Path,
};

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Path as AxumPath, Query, Request, State, ws::Message},
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use md5::Digest;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

use super::*;

const MAX_PROJECT_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PROJECT_ARCHIVE_FILES_BYTES: u64 = 128 * 1024 * 1024;

pub(super) fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/v3/templates",
            get(list_api_templates).post(create_template),
        )
        .route(
            "/v3/templates/{template_id}",
            get(get_template)
                .put(update_template)
                .delete(delete_template),
        )
        .route(
            "/v3/templates/{template_id}/duplicate",
            post(duplicate_template),
        )
        .route("/v3/statistics", get(controller_statistics))
        .route("/v3/notifications", get(controller_notifications))
        .route("/v3/notifications/ws", get(controller_notifications_ws))
        .route("/v3/settings", get(get_settings).put(update_settings))
        .route("/v3/computes", get(list_computes))
        .route("/v3/computes/{compute_id}", get(get_compute))
        .route("/v3/projects/load", post(load_project))
        .route("/v3/projects/{project_id}/stats", get(project_stats))
        .route(
            "/v3/projects/{project_id}/duplicate",
            post(duplicate_project),
        )
        .route("/v3/projects/{project_id}/locked", get(project_locked))
        .route("/v3/projects/{project_id}/lock", post(lock_project))
        .route("/v3/projects/{project_id}/unlock", post(unlock_project))
        .route("/v3/projects/{project_id}/gns3file", get(project_gns3file))
        .route("/v3/projects/{project_id}/export", get(export_project))
        .route("/v3/projects/{project_id}/import", post(import_project))
        .route(
            "/v3/projects/{project_id}/files/{*file_path}",
            get(get_project_file).post(write_project_file),
        )
        .route(
            "/v3/projects/{project_id}/notifications",
            get(project_notifications),
        )
        .route(
            "/v3/projects/{project_id}/snapshots",
            get(list_snapshots).post(create_snapshot),
        )
        .route(
            "/v3/projects/{project_id}/snapshots/{snapshot_id}",
            delete(delete_snapshot),
        )
        .route(
            "/v3/projects/{project_id}/snapshots/{snapshot_id}/restore",
            post(restore_snapshot),
        )
        .route(
            "/v3/projects/{project_id}/nodes/start",
            post(start_all_nodes),
        )
        .route("/v3/projects/{project_id}/nodes/stop", post(stop_all_nodes))
        .route(
            "/v3/projects/{project_id}/nodes/suspend",
            post(suspend_all_nodes),
        )
        .route(
            "/v3/projects/{project_id}/nodes/reload",
            post(reload_all_nodes),
        )
        .route(
            "/v3/projects/{project_id}/nodes/console/reset",
            post(reset_all_consoles),
        )
        .route(
            "/v3/projects/{project_id}/nodes/{node_id}/duplicate",
            post(duplicate_node),
        )
        .route(
            "/v3/projects/{project_id}/nodes/{node_id}/isolate",
            post(isolate_node),
        )
        .route(
            "/v3/projects/{project_id}/nodes/{node_id}/unisolate",
            post(unisolate_node),
        )
        .route(
            "/v3/projects/{project_id}/nodes/{node_id}/console/reset",
            post(reset_console),
        )
        .route(
            "/v3/projects/{project_id}/nodes/{node_id}/links",
            get(node_links),
        )
        .route(
            "/v3/projects/{project_id}/nodes/{node_id}/files",
            get(list_node_files),
        )
        .route(
            "/v3/projects/{project_id}/nodes/{node_id}/files/{*file_path}",
            get(get_node_file)
                .post(write_node_file)
                .delete(delete_node_file),
        )
        .route(
            "/v3/projects/{project_id}/nodes/{node_id}/qemu/disk_image/{disk_name}",
            post(create_qemu_disk)
                .put(resize_qemu_disk)
                .delete(delete_qemu_disk),
        )
        .route(
            "/v3/projects/{project_id}/nodes/{node_id}/dynamips/auto_idlepc",
            get(auto_idlepc),
        )
        .route(
            "/v3/projects/{project_id}/nodes/{node_id}/dynamips/idlepc_proposals",
            get(idlepc_proposals),
        )
        .route("/v3/images", get(list_images))
        .route("/v3/images/upload/{*image_path}", post(upload_image))
        .route(
            "/v3/images/{*image_path}",
            get(get_image).delete(delete_image),
        )
        .route("/v3/symbols", get(list_symbols))
        .route("/v3/symbols/default_symbols", get(default_symbols))
        .route(
            "/v3/symbols/{symbol_id}/raw",
            get(get_symbol).post(upload_symbol),
        )
        .route("/v3/symbols/{symbol_id}/dimensions", get(symbol_dimensions))
        .route("/v3/symbols/{symbol_id}", delete(delete_symbol))
}

async fn list_api_templates(State(state): State<AppState>) -> Result<Json<Vec<Value>>, ApiError> {
    let mut templates = available_templates(&state).await?;
    let rows = sqlx::query("SELECT template_json FROM controller_templates ORDER BY template_id")
        .fetch_all(state.store.pool())
        .await?;
    for row in rows {
        templates.push(decode_json(row.get("template_json"))?);
    }
    Ok(Json(templates))
}

pub(super) async fn check_version(Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    let requested = body
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad_request("version is required"))?;
    if requested != "3.1.0.dev7" {
        return Err(ApiError::conflict(format!(
            "Client version {requested} is not the same as server version 3.1.0.dev7"
        )));
    }
    Ok(Json(json!({"version":"3.1.0.dev7"})))
}

async fn create_template(
    State(state): State<AppState>,
    Json(mut template): Json<Value>,
) -> Result<Response, ApiError> {
    let obj = as_object_mut(&mut template, "template body must be an object")?;
    let template_id = parse_or_generate_uuid(obj.get("template_id"), "template_id")?;
    let name = required_string(obj, "name")?.to_owned();
    let template_type = required_string(obj, "template_type")?.to_owned();
    obj.insert("template_id".into(), json!(template_id));
    obj.insert("name".into(), json!(name));
    obj.insert("template_type".into(), json!(template_type));
    obj.entry("compute_id").or_insert(json!("local"));
    obj.entry("builtin").or_insert(json!(false));
    sqlx::query("INSERT INTO controller_templates (template_id, template_json) VALUES (?, ?)")
        .bind(&template_id)
        .bind(serde_json::to_string(&template).map_err(internal_json_error)?)
        .execute(state.store.pool())
        .await?;
    Ok((StatusCode::CREATED, Json(template)).into_response())
}

async fn get_template(
    State(state): State<AppState>,
    AxumPath(template_id): AxumPath<Uuid>,
) -> Result<Json<Value>, ApiError> {
    if let Some(row) =
        sqlx::query("SELECT template_json FROM controller_templates WHERE template_id = ?")
            .bind(template_id.to_string())
            .fetch_optional(state.store.pool())
            .await?
    {
        return Ok(Json(decode_json(row.get("template_json"))?));
    }
    let id = template_id.to_string();
    let template = available_templates(&state)
        .await?
        .into_iter()
        .find(|item| item["template_id"].as_str() == Some(id.as_str()))
        .ok_or_else(|| ApiError::not_found(format!("Template {id} does not exist")))?;
    Ok(Json(template))
}

async fn update_template(
    State(state): State<AppState>,
    AxumPath(template_id): AxumPath<Uuid>,
    Json(patch): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let template_id = template_id.to_string();
    let row = sqlx::query("SELECT template_json FROM controller_templates WHERE template_id = ?")
        .bind(&template_id)
        .fetch_optional(state.store.pool())
        .await?;
    let Some(row) = row else {
        return super::update_template_profile(
            State(state),
            AxumPath(Uuid::parse_str(&template_id).unwrap()),
            Json(patch),
        )
        .await;
    };
    let mut template = decode_json(row.get("template_json"))?;
    let patch = patch
        .as_object()
        .ok_or_else(|| ApiError::bad_request("template update body must be an object"))?;
    let target = as_object_mut(&mut template, "stored template must be an object")?;
    for (key, value) in patch {
        if !matches!(key.as_str(), "template_id" | "created_at" | "updated_at") {
            target.insert(key.clone(), value.clone());
        }
    }
    let serialized = serde_json::to_string(&template).map_err(internal_json_error)?;
    sqlx::query("UPDATE controller_templates SET template_json = ?, updated_at = CURRENT_TIMESTAMP WHERE template_id = ?")
        .bind(serialized)
        .bind(&template_id)
        .execute(state.store.pool())
        .await?;
    Ok(Json(template))
}

async fn delete_template(
    State(state): State<AppState>,
    AxumPath(template_id): AxumPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    let result = sqlx::query("DELETE FROM controller_templates WHERE template_id = ?")
        .bind(template_id.to_string())
        .execute(state.store.pool())
        .await?;
    if result.rows_affected() == 0 {
        return super::reset_template_profile(State(state), AxumPath(template_id))
            .await
            .map(|_| StatusCode::NO_CONTENT);
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn duplicate_template(
    State(state): State<AppState>,
    AxumPath(template_id): AxumPath<Uuid>,
) -> Result<Response, ApiError> {
    let original = get_template(State(state.clone()), AxumPath(template_id))
        .await?
        .0;
    let mut duplicate = original;
    let id = Uuid::new_v4().to_string();
    let name = duplicate["name"].as_str().unwrap_or("Template").to_owned();
    duplicate["template_id"] = json!(id);
    duplicate["name"] = json!(format!("{name} copy"));
    duplicate["builtin"] = json!(false);
    sqlx::query("INSERT INTO controller_templates (template_id, template_json) VALUES (?, ?)")
        .bind(&id)
        .bind(serde_json::to_string(&duplicate).map_err(internal_json_error)?)
        .execute(state.store.pool())
        .await?;
    Ok((StatusCode::CREATED, Json(duplicate)).into_response())
}

async fn controller_statistics(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
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

async fn controller_notifications(State(state): State<AppState>) -> Response {
    let stream = futures_util::stream::unfold(state.store.subscribe(), |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let mut value = serde_json::to_vec(&json!({"action": event.action, "event": event.event, "project_id": event.project_id})).unwrap_or_default();
                    value.push(b'\n');
                    return Some((Ok::<_, io::Error>(value), rx));
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });
    let mut response = Response::new(Body::from_stream(stream));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-ndjson"),
    );
    response
}

async fn controller_notifications_ws(
    State(state): State<AppState>,
    upgrade: WebSocketUpgrade,
) -> Response {
    upgrade.on_upgrade(move |mut socket| async move {
        let mut events = state.store.subscribe();
        loop {
            match events.recv().await {
                Ok(event) => {
                    let value = json!({"action":event.action,"event":event.event,"project_id":event.project_id}).to_string();
                    if socket.send(Message::Text(value.into())).await.is_err() { break; }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    }).into_response()
}

async fn project_notifications(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<Uuid>,
) -> Response {
    let project_id = project_id.to_string();
    let stream = futures_util::stream::unfold(
        (state.store.subscribe(), project_id),
        |(mut rx, project_id)| async move {
            loop {
                match rx.recv().await {
                    Ok(event) if event.project_id == project_id => {
                        let mut value = serde_json::to_vec(&json!({"action": event.action, "event": event.event, "project_id": event.project_id})).unwrap_or_default();
                        value.push(b'\n');
                        return Some((Ok::<_, io::Error>(value), (rx, project_id)));
                    }
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        },
    );
    let mut response = Response::new(Body::from_stream(stream));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-ndjson"),
    );
    response
}

async fn get_settings(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let json: String =
        sqlx::query_scalar("SELECT settings_json FROM controller_settings WHERE singleton = 1")
            .fetch_one(state.store.pool())
            .await?;
    Ok(Json(decode_json(json)?))
}

async fn update_settings(
    State(state): State<AppState>,
    Json(patch): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let patch = patch
        .as_object()
        .ok_or_else(|| ApiError::bad_request("settings body must be an object"))?;
    let row: String =
        sqlx::query_scalar("SELECT settings_json FROM controller_settings WHERE singleton = 1")
            .fetch_one(state.store.pool())
            .await?;
    let mut settings = decode_json(row)?;
    let target = as_object_mut(&mut settings, "stored settings must be an object")?;
    for (section, value) in patch {
        if !value.is_object() {
            return Err(ApiError::bad_request(format!(
                "settings section '{section}' must be an object"
            )));
        }
        let section_target = target.entry(section.clone()).or_insert(json!({}));
        for (key, value) in value.as_object().expect("validated settings section") {
            section_target[key] = value.clone();
        }
    }
    sqlx::query("UPDATE controller_settings SET settings_json = ? WHERE singleton = 1")
        .bind(serde_json::to_string(&settings).map_err(internal_json_error)?)
        .execute(state.store.pool())
        .await?;
    Ok(Json(settings))
}

fn local_compute() -> Value {
    json!({"compute_id":"local","name":"Local","host":"127.0.0.1","port":3080,"protocol":"http","auto_connect":true,"connected":true})
}
async fn list_computes() -> Json<Vec<Value>> {
    Json(vec![local_compute()])
}
async fn get_compute(AxumPath(compute_id): AxumPath<String>) -> Result<Json<Value>, ApiError> {
    if compute_id == "local" {
        Ok(Json(local_compute()))
    } else {
        Err(ApiError::not_found(format!(
            "Compute {compute_id} does not exist"
        )))
    }
}

async fn project_stats(
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

async fn project_locked(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<Uuid>,
) -> Result<Json<bool>, ApiError> {
    let project = fetch_project(state.store.pool(), &project_id.to_string()).await?;
    Ok(Json(project["locked"].as_bool().unwrap_or(false)))
}
async fn lock_project(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    set_project_lock(state, project_id, true).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn unlock_project(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    set_project_lock(state, project_id, false).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn set_project_lock(state: AppState, project_id: Uuid, locked: bool) -> Result<(), ApiError> {
    let pid = project_id.to_string();
    let mut tx = state.store.pool().begin().await?;
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

async fn project_gns3file(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let pid = project_id.to_string();
    let mut project = fetch_project(state.store.pool(), &pid).await?;
    for kind in ["node", "link", "drawing"] {
        let array = list_objects(state.store.pool(), &pid, kind).await?.0;
        let key = match kind {
            "node" => "nodes",
            "link" => "links",
            _ => "drawings",
        };
        project[key] = json!(array);
    }
    Ok(Json(project))
}

async fn get_project_file(
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
async fn write_project_file(
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

async fn export_project(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<Uuid>,
) -> Result<Response, ApiError> {
    let pid = project_id.to_string();
    let mut project = project_gns3file(State(state.clone()), AxumPath(project_id))
        .await?
        .0;
    strip_project_runtime_fields(&mut project);
    if project["status"].as_str()==Some("opened") && sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM topology_objects WHERE project_id=? AND kind='node' AND json_extract(object_json,'$.status')='started'").bind(&pid).fetch_one(state.store.pool()).await? > 0 { return Err(ApiError::conflict("Project must be stopped in order to export it")); }
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

async fn import_project(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<Uuid>,
    Query(query): Query<HashMap<String, String>>,
    request: Request,
) -> Result<Response, ApiError> {
    let bytes = to_bytes(request.into_body(), 64 * 1024 * 1024)
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let ImportedProjectArchive {
        project: project_data,
        project_files,
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
    strip_project_runtime_fields(&mut project);
    save_imported_project(state, project_id, &project_data, &project, &project_files).await?;
    Ok((StatusCode::CREATED, Json(project)).into_response())
}

async fn load_project(
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
        project,
        project_files,
    } = read_project_archive(&data)?;
    let project_id = Uuid::new_v4();
    let mut saved = project.clone();
    saved["project_id"] = json!(project_id);
    saved["path"] = Value::Null;
    saved["status"] = json!("opened");
    strip_project_runtime_fields(&mut saved);
    save_imported_project(state, project_id, &project, &saved, &project_files).await?;
    Ok((StatusCode::CREATED, Json(saved)).into_response())
}

async fn save_imported_project(
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
    for (key, kind, id_field) in [
        ("nodes", "node", "node_id"),
        ("links", "link", "link_id"),
        ("drawings", "drawing", "drawing_id"),
    ] {
        if let Some(items) = source.get(key).and_then(Value::as_array) {
            for item in items {
                let mut value = item.clone();
                let id = value[id_field]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| Uuid::new_v4().to_string());
                value[id_field] = json!(id);
                value["project_id"] = json!(pid);
                if kind == "node" {
                    strip_node_runtime_fields(&mut value);
                    value["status"] = json!("stopped");
                } else if kind == "link" {
                    strip_link_runtime_fields(&mut value);
                }
                let name = value.get("name").and_then(Value::as_str).map(str::to_owned);
                sqlx::query("INSERT INTO topology_objects(project_id,kind,object_id,name,object_json) VALUES(?,?,?,?,?)").bind(&pid).bind(kind).bind(&id).bind(name).bind(serde_json::to_string(&value).map_err(internal_json_error)?).execute(&mut *tx).await?;
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

async fn duplicate_project(
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

async fn create_snapshot(
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
    let answer = json!({"snapshot_id":id,"project_id":pid,"name":name,"description":description,"filename":format!("{id}.snapshot"),"created_at":row.get::<String,_>("epoch").parse::<i64>().unwrap_or(0),"created":row.get::<String,_>("created_at")});
    Ok((StatusCode::CREATED, Json(answer)).into_response())
}
async fn list_snapshots(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<Uuid>,
) -> Result<Json<Vec<Value>>, ApiError> {
    let pid = project_id.to_string();
    ensure_project_exists_pool(state.store.pool(), &pid).await?;
    let rows=sqlx::query("SELECT snapshot_id,name,description,created_at,strftime('%s',created_at) AS epoch FROM project_snapshots WHERE project_id=? ORDER BY created_at,name").bind(&pid).fetch_all(state.store.pool()).await?;
    Ok(Json(rows.into_iter().map(|r|{let id:String=r.get("snapshot_id");json!({"snapshot_id":id,"project_id":pid,"name":r.get::<String,_>("name"),"description":r.get::<String,_>("description"),"filename":format!("{id}.snapshot"),"created_at":r.get::<String,_>("epoch").parse::<i64>().unwrap_or(0),"created":r.get::<String,_>("created_at")})}).collect()))
}
async fn delete_snapshot(
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
async fn restore_snapshot(
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

async fn bulk_nodes(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<Uuid>,
    action: NodeAction,
) -> Result<StatusCode, ApiError> {
    let pid = project_id.to_string();
    let nodes = list_objects(state.store.pool(), &pid, "node").await?.0;
    for node in nodes {
        let id = node["node_id"]
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or_else(|| ApiError::bad_request("node has invalid ID"))?;
        change_node_runtime_state(state.clone(), project_id, id, action).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn start_all_nodes(
    State(s): State<AppState>,
    AxumPath(p): AxumPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    bulk_nodes(State(s), AxumPath(p), NodeAction::Start).await
}
async fn stop_all_nodes(
    State(s): State<AppState>,
    AxumPath(p): AxumPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    bulk_nodes(State(s), AxumPath(p), NodeAction::Stop).await
}
async fn suspend_all_nodes(
    State(s): State<AppState>,
    AxumPath(p): AxumPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    bulk_nodes(State(s), AxumPath(p), NodeAction::Suspend).await
}
async fn reload_all_nodes(
    State(s): State<AppState>,
    AxumPath(p): AxumPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    bulk_nodes(State(s), AxumPath(p), NodeAction::Reload).await
}
async fn reset_all_consoles(
    State(state): State<AppState>,
    AxumPath(project_id): AxumPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    let pid = project_id.to_string();
    ensure_project_exists_pool(state.store.pool(), &pid).await?;
    Err(ApiError::new(
        StatusCode::NOT_IMPLEMENTED,
        "Resetting runtime consoles is not supported",
    ))
}

async fn duplicate_node(
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
async fn set_node_isolated(
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
async fn isolate_node(
    State(s): State<AppState>,
    AxumPath((p, n)): AxumPath<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    set_node_isolated(State(s), p, n, true).await
}
async fn unisolate_node(
    State(s): State<AppState>,
    AxumPath((p, n)): AxumPath<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    set_node_isolated(State(s), p, n, false).await
}
async fn reset_console(
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
async fn node_links(
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

fn clean_relative_path(path: &str) -> Result<String, ApiError> {
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
struct FileQuery {
    path: Option<String>,
    recursive: Option<bool>,
}
async fn list_node_files(
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
    let files=rows.into_iter().filter_map(|r|{let path:String=r.get("file_path");if !prefix.is_empty()&&!path.starts_with(&format!("{prefix}/")){return None;}if !recursive&&path.strip_prefix(&prefix).unwrap_or(&path).trim_start_matches('/').contains('/'){return None;}Some(json!({"path":path,"size":r.get::<i64,_>("size"),"created_at":r.get::<String,_>("updated_at"),"modified_at":r.get::<String,_>("updated_at"),"file_type":"file"}))}).collect();
    Ok(Json(files))
}
async fn get_node_file(
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
async fn write_node_file(
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
async fn delete_node_file(
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

async fn create_qemu_disk(
    State(state): State<AppState>,
    AxumPath((p, n, d)): AxumPath<(Uuid, Uuid, String)>,
    Json(body): Json<Value>,
) -> Result<StatusCode, ApiError> {
    qemu_disk_op(state, p, n, d, body, "create").await
}
async fn resize_qemu_disk(
    State(state): State<AppState>,
    AxumPath((p, n, d)): AxumPath<(Uuid, Uuid, String)>,
    Json(body): Json<Value>,
) -> Result<StatusCode, ApiError> {
    qemu_disk_op(state, p, n, d, body, "resize").await
}
async fn delete_qemu_disk(
    State(state): State<AppState>,
    AxumPath((p, n, d)): AxumPath<(Uuid, Uuid, String)>,
) -> Result<StatusCode, ApiError> {
    qemu_disk_op(state, p, n, d, json!({}), "delete").await
}
async fn qemu_disk_op(
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
async fn auto_idlepc(
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
    Ok(Json(json!({"idlepc":idlepc})))
}
async fn idlepc_proposals(
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

#[derive(Deserialize)]
struct ImageQuery {
    image_type: Option<String>,
    name: Option<String>,
}
async fn list_images(
    State(state): State<AppState>,
    Query(query): Query<ImageQuery>,
) -> Result<Json<Vec<Value>>, ApiError> {
    let mut dirs = Vec::new();
    if query.image_type.as_deref().is_none_or(|t| t == "qemu") {
        dirs.push(("qemu", state.qemu.info().await.image_directory));
    }
    if query.image_type.as_deref().is_none_or(|t| t == "ios") {
        dirs.push(("ios", state.dynamips.info().await.image_directory));
    }
    if query
        .image_type
        .as_deref()
        .is_some_and(|t| !matches!(t, "qemu" | "ios" | "iou"))
    {
        return Err(ApiError::bad_request(
            "image_type must be qemu, ios, or iou",
        ));
    }
    let mut items = Vec::new();
    for (kind, dir) in dirs {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    error.to_string(),
                ));
            }
        };
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        {
            if !entry
                .file_type()
                .await
                .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
                .is_file()
            {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_string();
            if query.name.as_deref().is_some_and(|name| name != file_name) {
                continue;
            }
            let bytes = tokio::fs::read(entry.path())
                .await
                .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            let md5 = format!("{:x}", md5::Md5::digest(&bytes));
            items.push(json!({"filename":file_name,"path":entry.path().to_string_lossy(),"image_type":kind,"image_size":bytes.len(),"checksum":md5,"checksum_algorithm":"md5","availability":"available"}));
        }
    }
    Ok(Json(items))
}
async fn upload_image(
    State(state): State<AppState>,
    AxumPath(image_path): AxumPath<String>,
    request: Request,
) -> Result<Response, ApiError> {
    let name = Path::new(&image_path)
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| ApiError::bad_request("image path must end in a filename"))?;
    if is_qemu_image_upload(name) {
        let (tmp, dest) = state
            .qemu
            .prepare_upload_name(name)
            .map_err(runtime_api_error)?;
        let (size, checksum) =
            write_image_upload(request.into_body(), &tmp, MAX_QEMU_IMAGE_BYTES as u64).await?;
        if let Err(error) = state.qemu.finalize_uploaded_image(&tmp, &dest).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(runtime_api_error(error));
        }
        Ok((StatusCode::CREATED, Json(json!({"filename":name,"path":dest.to_string_lossy(),"image_type":"qemu","image_size":size,"checksum":checksum,"checksum_algorithm":"md5","availability":"available"}))).into_response())
    } else {
        let (tmp, dest) = state
            .dynamips
            .prepare_upload_name(name)
            .map_err(runtime_api_error)?;
        let (size, checksum) =
            write_image_upload(request.into_body(), &tmp, MAX_DYNAMIPS_IMAGE_BYTES as u64).await?;
        if let Err(error) = state.dynamips.finalize_uploaded_image(&tmp, &dest) {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(runtime_api_error(error));
        }
        Ok((StatusCode::CREATED, Json(json!({"filename":name,"path":dest.to_string_lossy(),"image_type":"ios","image_size":size,"checksum":checksum,"checksum_algorithm":"md5","availability":"available"}))).into_response())
    }
}

fn is_qemu_image_upload(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "qcow2" | "qcow" | "img" | "raw" | "vmdk" | "vdi"
            )
        })
}

async fn write_image_upload(
    body: Body,
    temporary_path: &Path,
    max_bytes: u64,
) -> Result<(u64, String), ApiError> {
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary_path)
        .await
        .map_err(|error| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Could not create temporary image upload file: {error}"),
            )
        })?;
    let mut stream = body.into_data_stream();
    let mut size = 0_u64;
    let mut checksum = md5::Md5::new();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                drop(file);
                let _ = tokio::fs::remove_file(temporary_path).await;
                return Err(ApiError::bad_request(format!(
                    "Image upload failed: {error}"
                )));
            }
        };
        size = size.saturating_add(chunk.len() as u64);
        if size > max_bytes {
            drop(file);
            let _ = tokio::fs::remove_file(temporary_path).await;
            return Err(ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "Image upload exceeds the size limit",
            ));
        }
        if let Err(error) = file.write_all(&chunk).await {
            drop(file);
            let _ = tokio::fs::remove_file(temporary_path).await;
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Could not write image upload: {error}"),
            ));
        }
        checksum.update(&chunk);
    }
    if let Err(error) = file.flush().await {
        drop(file);
        let _ = tokio::fs::remove_file(temporary_path).await;
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not flush image upload: {error}"),
        ));
    }
    drop(file);
    if size == 0 {
        let _ = tokio::fs::remove_file(temporary_path).await;
        return Err(ApiError::bad_request("image upload is empty"));
    }
    Ok((size, format!("{:x}", checksum.finalize())))
}
async fn get_image(
    State(state): State<AppState>,
    AxumPath(image_path): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    let needle = Path::new(&image_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&image_path);
    let images = list_images(
        State(state),
        Query(ImageQuery {
            image_type: None,
            name: None,
        }),
    )
    .await?
    .0;
    images
        .into_iter()
        .find(|image| {
            image["filename"].as_str() == Some(needle)
                || image["path"].as_str() == Some(image_path.as_str())
        })
        .map(Json)
        .ok_or_else(|| ApiError::not_found("Image does not exist"))
}
async fn delete_image(
    State(state): State<AppState>,
    AxumPath(image_path): AxumPath<String>,
) -> Result<StatusCode, ApiError> {
    let path = Path::new(&image_path);
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(ApiError::bad_request(
            "image path must not contain parent references",
        ));
    }
    let roots = [
        Path::new(&state.qemu.info().await.image_directory).to_path_buf(),
        Path::new(&state.dynamips.info().await.image_directory).to_path_buf(),
    ];
    let target = if path.is_absolute() {
        path.to_path_buf()
    } else {
        let mut found = None;
        for root in &roots {
            let p = root.join(path);
            if p.is_file() {
                found = Some(p);
                break;
            }
        }
        found.ok_or_else(|| ApiError::not_found("Image does not exist"))?
    };
    if !roots.iter().any(|root| target.starts_with(root)) {
        return Err(ApiError::bad_request(
            "image path is outside the configured image directories",
        ));
    }
    let target_string = target.to_string_lossy().to_string();
    let rows = sqlx::query("SELECT object_json FROM topology_objects WHERE kind='node'")
        .fetch_all(state.store.pool())
        .await?;
    for row in rows {
        let v = decode_json(row.get("object_json"))?;
        for key in ["image", "hda_disk_image"] {
            if v[key].as_str() == Some(target_string.as_str()) {
                return Err(ApiError::conflict("Image is used by a node"));
            }
        }
    }
    tokio::fs::remove_file(target)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_symbols(State(state): State<AppState>) -> Result<Json<Vec<Value>>, ApiError> {
    let rows=sqlx::query("SELECT symbol_id,length(contents) AS size,updated_at FROM controller_symbols ORDER BY symbol_id").fetch_all(state.store.pool()).await?;
    Ok(Json(rows.into_iter().map(|r|json!({"symbol_id":r.get::<String,_>("symbol_id"),"filename":r.get::<String,_>("symbol_id"),"size":r.get::<i64,_>("size"),"builtin":false})).collect()))
}
async fn default_symbols() -> Json<Value> {
    Json(
        json!({"default_symbol":":/symbols/router.svg","symbols":{"router":"router.svg","switch":"switch.svg","cloud":"cloud.svg","pc":"pc.svg"}}),
    )
}
fn safe_symbol_id(id: &str) -> Result<String, ApiError> {
    let name = Path::new(id)
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| ApiError::bad_request("symbol ID must be a filename"))?;
    if name != id
        || name.starts_with('.')
        || name.len() > 128
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(ApiError::bad_request("symbol ID must be a safe filename"));
    }
    Ok(name.to_owned())
}
async fn get_symbol(
    State(state): State<AppState>,
    AxumPath(symbol_id): AxumPath<String>,
) -> Result<Response, ApiError> {
    let id = safe_symbol_id(&symbol_id)?;
    let row = sqlx::query("SELECT contents FROM controller_symbols WHERE symbol_id=?")
        .bind(&id)
        .fetch_optional(state.store.pool())
        .await?
        .ok_or_else(|| ApiError::not_found("Symbol does not exist"))?;
    let mut response = (
        [(header::CONTENT_TYPE, content_type(&symbol_id))],
        Body::from(row.get::<Vec<u8>, _>("contents")),
    )
        .into_response();
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{id}\""))
            .expect("safe symbol IDs produce valid disposition headers"),
    );
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "sandbox; default-src 'none'; img-src data:; style-src 'unsafe-inline'",
        ),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}
async fn upload_symbol(
    State(state): State<AppState>,
    AxumPath(symbol_id): AxumPath<String>,
    request: Request,
) -> Result<StatusCode, ApiError> {
    let id = safe_symbol_id(&symbol_id)?;
    let bytes = to_bytes(request.into_body(), 16 * 1024 * 1024)
        .await
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    sqlx::query("INSERT INTO controller_symbols(symbol_id,contents) VALUES(?,?) ON CONFLICT(symbol_id) DO UPDATE SET contents=excluded.contents,updated_at=CURRENT_TIMESTAMP").bind(id).bind(bytes.to_vec()).execute(state.store.pool()).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn delete_symbol(
    State(state): State<AppState>,
    AxumPath(symbol_id): AxumPath<String>,
) -> Result<StatusCode, ApiError> {
    let id = safe_symbol_id(&symbol_id)?;
    let r = sqlx::query("DELETE FROM controller_symbols WHERE symbol_id=?")
        .bind(id)
        .execute(state.store.pool())
        .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::not_found("Symbol does not exist"));
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn symbol_dimensions(
    State(state): State<AppState>,
    AxumPath(symbol_id): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    let id = safe_symbol_id(&symbol_id)?;
    let row = sqlx::query("SELECT contents FROM controller_symbols WHERE symbol_id=?")
        .bind(id)
        .fetch_optional(state.store.pool())
        .await?
        .ok_or_else(|| ApiError::not_found("Symbol does not exist"))?;
    let bytes: Vec<u8> = row.get("contents");
    let (width, height) = image_dimensions(&bytes).ok_or_else(|| {
        ApiError::bad_request("symbol dimensions are not supported for this file format")
    })?;
    Ok(Json(json!({"width":width,"height":height})))
}
fn content_type(path: &str) -> &'static str {
    if path.ends_with(".svg") {
        "image/svg+xml"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg"
    } else {
        "application/octet-stream"
    }
}
fn image_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    if data.starts_with(b"\x89PNG\r\n\x1a\n") && data.len() >= 24 {
        return Some((
            u32::from_be_bytes(data[16..20].try_into().ok()?),
            u32::from_be_bytes(data[20..24].try_into().ok()?),
        ));
    }
    let s = std::str::from_utf8(data).ok()?;
    let tag = s.find("<svg")?;
    let end = s[tag..].find('>')? + tag;
    let root = &s[tag..end];
    let dimension = |key: &str| -> Option<u32> {
        let start = root.find(key)? + key.len();
        let rest = root[start..].trim_start_matches([' ', '=', '"', '\'']);
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse().ok()
    };
    Some((dimension("width")?, dimension("height")?))
}

#[derive(Debug)]
struct ImportedProjectArchive {
    project: Value,
    project_files: Vec<(String, Vec<u8>)>,
}

fn write_project_archive(
    topology: Vec<u8>,
    project_files: Vec<(String, Vec<u8>)>,
) -> Result<Vec<u8>, ApiError> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    writer
        .start_file("project.gns3", options)
        .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    writer
        .write_all(&topology)
        .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    for (file_path, contents) in project_files {
        let file_path = clean_relative_path(&file_path)?;
        writer
            .start_file(file_path, options)
            .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        writer
            .write_all(&contents)
            .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    }
    writer
        .finish()
        .map(|cursor| cursor.into_inner())
        .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

fn read_project_archive(bytes: &[u8]) -> Result<ImportedProjectArchive, ApiError> {
    if !bytes.starts_with(b"PK") {
        return serde_json::from_slice(bytes)
            .map(|project| ImportedProjectArchive {
                project,
                project_files: Vec::new(),
            })
            .map_err(|error| {
                archive_bad_request(format!("Invalid .gns3project archive or JSON: {error}"))
            });
    }

    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| archive_bad_request(format!("Invalid .gns3project ZIP: {error}")))?;
    let mut topology = None;
    let mut project_files = Vec::new();
    let mut expanded_size = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            archive_bad_request(format!("Invalid .gns3project ZIP entry: {error}"))
        })?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_owned();
        let project_file_path = archive_project_file_path(&name)?;
        let is_topology = name == "project.gns3" || (topology.is_none() && name.ends_with(".gns3"));
        if !is_topology && project_file_path.is_none() {
            continue;
        }
        let advertised_size = entry.size();
        let remaining = MAX_PROJECT_ARCHIVE_FILES_BYTES.saturating_sub(expanded_size);
        let max_size = if is_topology {
            MAX_PROJECT_FILE_BYTES.min(remaining)
        } else {
            MAX_PROJECT_FILE_BYTES.min(remaining)
        };
        let contents = read_bounded_archive_entry(&mut entry, advertised_size, max_size)?;
        expanded_size = expanded_size.saturating_add(contents.len() as u64);
        if is_topology {
            if topology.is_some() && name == "project.gns3" {
                return Err(archive_bad_request(
                    "Project archive contains more than one project.gns3 topology file",
                ));
            }
            topology = Some(contents);
        } else if let Some(path) = project_file_path {
            project_files.push((path, contents));
        }
    }
    let topology = topology
        .ok_or_else(|| archive_bad_request("Project archive contains no .gns3 topology file"))?;
    let project = serde_json::from_slice(&topology).map_err(|error| {
        archive_bad_request(format!("Project archive topology is invalid: {error}"))
    })?;
    Ok(ImportedProjectArchive {
        project,
        project_files,
    })
}

fn read_bounded_archive_entry(
    entry: &mut impl Read,
    advertised_size: u64,
    max_size: u64,
) -> Result<Vec<u8>, ApiError> {
    if advertised_size > max_size {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Project archive contents exceed the 128 MiB expanded size limit",
        ));
    }
    let mut contents = Vec::with_capacity(advertised_size as usize);
    entry
        .take(max_size.saturating_add(1))
        .read_to_end(&mut contents)
        .map_err(|error| {
            archive_bad_request(format!("Could not read project archive entry: {error}"))
        })?;
    if contents.len() as u64 > max_size || contents.len() as u64 != advertised_size {
        return Err(archive_bad_request("Project archive entry size is invalid"));
    }
    Ok(contents)
}

fn archive_project_file_path(name: &str) -> Result<Option<String>, ApiError> {
    let Some(path) = name
        .strip_prefix("project-files/")
        .or_else(|| name.strip_prefix("project_files/"))
    else {
        return Ok(None);
    };
    if path.is_empty() || path.starts_with('/') || name.ends_with('/') || path.contains('\\') {
        return Err(archive_bad_request(
            "Project archive contains an invalid project file path",
        ));
    }
    clean_relative_path(path)
        .map(Some)
        .map_err(|error| archive_bad_request(error.message))
}

fn archive_bad_request(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, message)
}

fn strip_node_runtime_fields(node: &mut Value) {
    let Some(object) = node.as_object_mut() else {
        return;
    };
    object.retain(|key, _| {
        let key = key.to_ascii_lowercase();
        !key.starts_with("runtime_")
            && !matches!(
                key.as_str(),
                "container_id"
                    | "container_name"
                    | "container_pid"
                    | "image_digest"
                    | "dynamips_id"
                    | "pid"
                    | "pid_file"
                    | "qemu_pid"
                    | "qmp"
                    | "qmp_socket"
                    | "qmp_path"
                    | "serial"
                    | "serial_socket"
                    | "serial_path"
                    | "overlay"
                    | "overlay_path"
                    | "log"
                    | "log_file"
                    | "log_path"
                    | "working_dir"
                    | "working_directory"
                    | "hypervisor_port"
                    | "console"
                    | "aux"
                    | "console_host"
                    | "console_port"
                    | "aux_port"
            )
            && !key.ends_with("_pid")
            && !key.ends_with("_pid_file")
            && !key.ends_with("_qmp_socket")
            && !key.ends_with("_qmp_path")
            && !key.ends_with("_qmp")
            && !key.ends_with("_serial")
            && !key.ends_with("_serial_socket")
            && !key.ends_with("_serial_path")
            && !key.ends_with("_overlay")
            && !key.ends_with("_overlay_path")
            && !key.ends_with("_log_file")
            && !key.ends_with("_log_path")
            && !key.ends_with("_working_dir")
            && !key.ends_with("_working_directory")
            && !key.ends_with("_hypervisor_port")
            && !key.ends_with("_container_id")
    });
}

fn strip_link_runtime_fields(link: &mut Value) {
    if let Some(endpoints) = link.get_mut("nodes").and_then(Value::as_array_mut) {
        for endpoint in endpoints {
            if let Some(endpoint) = endpoint.as_object_mut() {
                endpoint.retain(|key, _| !key.starts_with("runtime_"));
            }
        }
    }
    if let Some(object) = link.as_object_mut() {
        object.insert("capturing".into(), json!(false));
        object.insert("capture_file_name".into(), Value::Null);
        object.insert("capture_file_path".into(), Value::Null);
        object.insert("capture_endpoint_id".into(), Value::Null);
        object.insert("wireshark".into(), json!(false));
    }
}

fn strip_project_runtime_fields(project: &mut Value) {
    if let Some(nodes) = project.get_mut("nodes").and_then(Value::as_array_mut) {
        for node in nodes {
            strip_node_runtime_fields(node);
            if node.get("status").is_some() {
                node["status"] = json!("stopped");
            }
        }
    }
    if let Some(links) = project.get_mut("links").and_then(Value::as_array_mut) {
        for link in links {
            strip_link_runtime_fields(link);
        }
    }
}

#[cfg(test)]
mod archive_tests {
    use std::io::{Cursor, Write};

    use super::{
        archive_project_file_path, image_dimensions, is_qemu_image_upload, read_project_archive,
        strip_node_runtime_fields, write_image_upload, write_project_archive,
    };
    use axum::{body::Body, http::StatusCode};
    use serde_json::json;
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    fn compressed_project_archive(method: CompressionMethod) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(
                "project.gns3",
                SimpleFileOptions::default().compression_method(method),
            )
            .unwrap();
        writer
            .write_all(br#"{"name":"Compressed","nodes":[],"links":[],"drawings":[]}"#)
            .unwrap();
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn stored_project_archive_round_trips() {
        let value = br#"{"name":"Lab","nodes":[],"links":[],"drawings":[]}"#;
        let archive = write_project_archive(value.to_vec(), Vec::new())
            .ok()
            .unwrap();
        assert_eq!(
            read_project_archive(&archive).ok().unwrap().project["name"],
            "Lab"
        );
    }

    #[test]
    fn deflate_and_upstream_zstandard_archives_are_supported() {
        for method in [CompressionMethod::Deflated, CompressionMethod::Zstd] {
            let imported = read_project_archive(&compressed_project_archive(method))
                .ok()
                .unwrap();
            assert_eq!(imported.project["name"], "Compressed");
        }
    }

    #[test]
    fn project_files_round_trip_and_reject_unsafe_archive_paths() {
        let archive = write_project_archive(
            br#"{"name":"Files","nodes":[],"links":[],"drawings":[]}"#.to_vec(),
            vec![("project-files/readme.md".into(), b"hello".to_vec())],
        )
        .ok()
        .unwrap();
        let imported = read_project_archive(&archive).ok().unwrap();
        assert_eq!(
            imported.project_files,
            vec![("readme.md".to_owned(), b"hello".to_vec())]
        );
        assert!(archive_project_file_path("project-files/../../outside").is_err());
        assert!(archive_project_file_path("project-files/../outside").is_err());
    }

    #[test]
    fn malformed_zip_headers_return_errors_without_panicking() {
        let mut oversized_name_length = vec![0_u8; 30];
        oversized_name_length[..4].copy_from_slice(b"PK\x03\x04");
        oversized_name_length[26..28].copy_from_slice(&u16::MAX.to_le_bytes());
        assert_eq!(
            read_project_archive(&oversized_name_length)
                .err()
                .unwrap()
                .status,
            StatusCode::BAD_REQUEST
        );

        let valid_archive = compressed_project_archive(CompressionMethod::Stored);
        let central_header = valid_archive
            .windows(4)
            .position(|window| window == b"PK\x01\x02")
            .unwrap();
        for field_offset in [26, 28] {
            let mut malformed = valid_archive.clone();
            malformed[field_offset..field_offset + 2].copy_from_slice(&u16::MAX.to_le_bytes());
            assert_eq!(
                read_project_archive(&malformed).err().unwrap().status,
                StatusCode::BAD_REQUEST,
                "malformed local-header length at {field_offset}"
            );
        }
        for field_offset in [28, 30, 32] {
            let mut malformed = valid_archive.clone();
            let field = central_header + field_offset;
            malformed[field..field + 2].copy_from_slice(&u16::MAX.to_le_bytes());
            assert_eq!(
                read_project_archive(&malformed).err().unwrap().status,
                StatusCode::BAD_REQUEST,
                "malformed central-header length at {field_offset}"
            );
        }

        let mut truncated_central_header = compressed_project_archive(CompressionMethod::Stored);
        truncated_central_header.truncate(truncated_central_header.len() - 10);
        assert_eq!(
            read_project_archive(&truncated_central_header)
                .err()
                .unwrap()
                .status,
            StatusCode::BAD_REQUEST
        );

        let mut truncated_local_header = vec![0_u8; 29];
        truncated_local_header[..4].copy_from_slice(b"PK\x03\x04");
        assert_eq!(
            read_project_archive(&truncated_local_header)
                .err()
                .unwrap()
                .status,
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn imported_runtime_fields_are_removed_from_node_objects() {
        let mut node = json!({
            "node_id":"source",
            "container_id":"abc",
            "runtime_overlay":"/source/overlay.qcow2",
            "runtime_qmp_socket":"/source/qmp.sock",
            "runtime_serial_socket":"/source/serial.sock",
            "runtime_pid_file":"/source/vm.pid",
            "runtime_log_file":"/source/vm.log",
            "runtime_working_dir":"/source",
            "runtime_hypervisor_port":7200,
            "runtime_hdb_overlay":"/source/disk.qcow2",
            "runtime_ghost_file":"/source/ghost.iso",
            "image_digest":"sha256:source",
            "dynamips_id":1,
            "qmp_path":"/source/qmp",
            "serial_path":"/source/serial",
            "overlay_path":"/source/overlay",
            "log_path":"/source/log",
            "working_directory":"/source",
            "qemu_pid":123,
            "pid_file":"/source/legacy.pid",
            "console":1234,
            "name":"Router"
        });
        strip_node_runtime_fields(&mut node);
        assert_eq!(node["name"], "Router");
        for key in [
            "container_id",
            "image_digest",
            "dynamips_id",
            "runtime_overlay",
            "runtime_qmp_socket",
            "runtime_serial_socket",
            "runtime_pid_file",
            "runtime_log_file",
            "runtime_working_dir",
            "runtime_hypervisor_port",
            "runtime_hdb_overlay",
            "runtime_ghost_file",
            "qmp_path",
            "serial_path",
            "overlay_path",
            "log_path",
            "working_directory",
            "qemu_pid",
            "pid_file",
            "console",
        ] {
            assert!(node.get(key).is_none(), "{key} was retained");
        }
    }

    #[tokio::test]
    async fn image_upload_stream_is_bounded_and_cleans_temporary_files() {
        let directory = tempfile::tempdir().unwrap();
        let temporary_path = directory.path().join(".test.upload");
        let error = write_image_upload(Body::from(vec![1, 2, 3, 4]), &temporary_path, 3)
            .await
            .unwrap_err();
        assert_eq!(error.status, StatusCode::PAYLOAD_TOO_LARGE);
        assert!(!temporary_path.exists());

        let failed_body = Body::from_stream(futures_util::stream::iter([
            Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"partial")),
            Err(std::io::Error::other("client disconnected")),
        ]));
        let error = write_image_upload(failed_body, &temporary_path, 64)
            .await
            .unwrap_err();
        assert_eq!(error.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!temporary_path.exists());
        assert!(is_qemu_image_upload("router.vmdk"));
        assert!(is_qemu_image_upload("disk.vdi"));
        assert!(is_qemu_image_upload("disk.QCOW2"));
        assert!(!is_qemu_image_upload("c7200.bin"));
    }
    #[test]
    fn png_dimensions_are_read_from_ihdr() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(image_dimensions(&png), Some((640, 480)));
    }
}

#[cfg(test)]
mod route_tests {
    use axum::{
        body::Body,
        http::{Method, Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use sqlx::Row;
    use tempfile::TempDir;
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
        net::TcpListener,
    };
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::super::{AppState, router};
    use crate::storage::Store;

    async fn app() -> (axum::Router, TempDir, Store) {
        let dir = TempDir::new().expect("test directory");
        let store = Store::connect(dir.path().join("api.sqlite").to_str().unwrap())
            .await
            .unwrap();
        (router(AppState::new(store.clone())), dir, store)
    }

    async fn mock_dynamips(commands: Vec<(&'static str, &'static str)>) -> u16 {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            for (expected, response) in commands {
                let (stream, _) = listener.accept().await.unwrap();
                let (read, mut write) = stream.into_split();
                let mut reader = BufReader::new(read);
                let mut command = String::new();
                reader.read_line(&mut command).await.unwrap();
                assert_eq!(command.trim(), expected);
                write.write_all(response.as_bytes()).await.unwrap();
            }
        });
        port
    }

    async fn send(
        app: &axum::Router,
        method: Method,
        uri: &str,
        body: Body,
        content_type: Option<&str>,
    ) -> axum::response::Response {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(value) = content_type {
            builder = builder.header("content-type", value);
        }
        app.clone()
            .oneshot(builder.body(body).unwrap())
            .await
            .unwrap()
    }

    async fn json_body(response: axum::response::Response) -> Value {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }

    fn assert_node_has_no_runtime_state(node: &Value) {
        if let Some(fields) = node.as_object() {
            for key in fields.keys() {
                assert!(
                    !key.to_ascii_lowercase().starts_with("runtime_"),
                    "{key} was retained"
                );
            }
        }
        for key in [
            "container_id",
            "container_name",
            "container_pid",
            "image_digest",
            "dynamips_id",
            "pid",
            "pid_file",
            "qemu_pid",
            "qmp",
            "qmp_socket",
            "qmp_path",
            "serial",
            "serial_socket",
            "serial_path",
            "overlay",
            "overlay_path",
            "log",
            "log_file",
            "log_path",
            "working_dir",
            "working_directory",
            "hypervisor_port",
            "console",
            "aux",
            "console_host",
            "console_port",
            "aux_port",
            "runtime_overlay",
            "runtime_qmp_socket",
            "runtime_serial_socket",
            "runtime_pid_file",
            "runtime_log_file",
            "runtime_working_dir",
            "runtime_hypervisor_port",
            "runtime_hdb_overlay",
        ] {
            assert!(node.get(key).is_none(), "{key} was retained");
        }
    }

    async fn websocket_status(app: &axum::Router, uri: &str) -> StatusCode {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::{TcpListener, TcpStream},
            time::{Duration, timeout},
        };

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_app = app.clone();
        let server = tokio::spawn(async move { axum::serve(listener, server_app).await.unwrap() });
        let mut client = TcpStream::connect(address).await.unwrap();
        client
            .write_all(
                format!(
                    "GET {uri} HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = [0_u8; 512];
        let length = timeout(Duration::from_secs(2), client.read(&mut response))
            .await
            .unwrap()
            .unwrap();
        server.abort();
        let first_line = String::from_utf8_lossy(&response[..length]);
        StatusCode::from_u16(
            first_line
                .split_whitespace()
                .nth(1)
                .unwrap()
                .parse()
                .unwrap(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn controller_statistics_reports_live_node_link_and_capture_counts() {
        let (app, _directory, store) = app().await;
        let project = json_body(
            send(
                &app,
                Method::POST,
                "/v3/projects",
                Body::from(r#"{"name":"Statistics lab"}"#),
                Some("application/json"),
            )
            .await,
        )
        .await;
        let project_id = project["project_id"].as_str().unwrap();
        let started_id = Uuid::new_v4().to_string();
        let stopped_id = Uuid::new_v4().to_string();
        let link_id = Uuid::new_v4().to_string();
        for (id, node_type, status) in [
            (&started_id, "qemu", "started"),
            (&stopped_id, "docker", "stopped"),
        ] {
            let object = json!({
                "node_id":id,
                "project_id":project_id,
                "node_type":node_type,
                "status":status,
                "console_host":"192.0.2.10"
            });
            sqlx::query("INSERT INTO topology_objects(project_id,kind,object_id,name,object_json) VALUES(?, 'node', ?, ?, ?)")
                .bind(project_id)
                .bind(id)
                .bind(node_type)
                .bind(object.to_string())
                .execute(store.pool())
                .await
                .unwrap();
        }
        let link = json!({
            "link_id":link_id,
            "project_id":project_id,
            "nodes":[],
            "capturing":true,
            "wireshark":true
        });
        sqlx::query("INSERT INTO topology_objects(project_id,kind,object_id,object_json) VALUES(?, 'link', ?, ?)")
            .bind(project_id)
            .bind(&link_id)
            .bind(link.to_string())
            .execute(store.pool())
            .await
            .unwrap();

        let stats =
            json_body(send(&app, Method::GET, "/v3/statistics", Body::empty(), None).await).await;
        assert!(stats["uptime_seconds"].as_u64().is_some());
        assert_eq!(stats["nodes"]["total"], 2);
        assert_eq!(stats["nodes"]["by_type"]["qemu"], 1);
        assert_eq!(stats["nodes"]["by_type"]["docker"], 1);
        assert_eq!(stats["nodes"]["by_status"]["started"], 1);
        assert_eq!(stats["nodes"]["by_status"]["stopped"], 1);
        assert_eq!(stats["links"]["total"], 1);
        assert_eq!(stats["links"]["capturing"], 1);
        assert_eq!(stats["webwireshark"]["containers"], 1);

        let reset = send(
            &app,
            Method::POST,
            &format!("/v3/projects/{project_id}/nodes/console/reset"),
            Body::empty(),
            None,
        )
        .await;
        assert_eq!(reset.status(), StatusCode::NOT_IMPLEMENTED);
        assert_eq!(
            json_body(reset).await["message"],
            "Resetting runtime consoles is not supported"
        );
        for id in [&started_id, &stopped_id] {
            let row = sqlx::query("SELECT object_json FROM topology_objects WHERE project_id=? AND kind='node' AND object_id=?")
                .bind(project_id)
                .bind(id)
                .fetch_one(store.pool())
                .await
                .unwrap();
            let node: Value =
                serde_json::from_str(row.get::<String, _>("object_json").as_str()).unwrap();
            assert_eq!(node["console_host"], "192.0.2.10");
        }
    }

    #[tokio::test]
    async fn imported_and_duplicated_runtime_nodes_cannot_remove_source_files() {
        let (app, directory, store) = app().await;
        let runtime_dir = directory.path().join("runtime-source");
        tokio::fs::create_dir_all(&runtime_dir).await.unwrap();
        let runtime_paths = [
            "overlay.qcow2",
            "qmp.sock",
            "serial.sock",
            "vm.pid",
            "vm.log",
            "disk.qcow2",
            "legacy.pid",
        ]
        .into_iter()
        .map(|name| (name, runtime_dir.join(name)))
        .collect::<Vec<_>>();
        for (name, path) in &runtime_paths {
            tokio::fs::write(path, format!("source runtime: {name}"))
                .await
                .unwrap();
        }
        let source_node_id = Uuid::new_v4();
        let source_project_id = Uuid::new_v4();
        let source_project = json!({
            "project_id":source_project_id,
            "name":"Source lab",
            "status":"opened",
            "nodes":[{
                "node_id":source_node_id,
                "node_type":"qemu",
                "status":"started",
                "runtime_overlay":runtime_paths[0].1,
                "runtime_working_dir":runtime_dir.display().to_string(),
                "container_id":"0123456789abcdef",
                "console":5001
            }]
        });
        sqlx::query("INSERT INTO projects(project_id,name,status,project_json) VALUES(?,?,?,?)")
            .bind(source_project_id.to_string())
            .bind("Source lab")
            .bind("opened")
            .bind(source_project.to_string())
            .execute(store.pool())
            .await
            .unwrap();
        let source_node = json!({
            "node_id":source_node_id,
            "project_id":source_project_id,
            "node_type":"qemu",
            "name":"Source router",
            "status":"stopped",
            "hda_disk_image":"/images/router.qcow2",
            "runtime_overlay":runtime_paths[0].1,
            "runtime_qmp_socket":runtime_paths[1].1,
            "runtime_serial_socket":runtime_paths[2].1,
            "runtime_pid_file":runtime_paths[3].1,
            "runtime_log_file":runtime_paths[4].1,
            "runtime_hdb_overlay":runtime_paths[5].1,
            "runtime_working_dir":runtime_dir.display().to_string(),
            "runtime_hypervisor_port":7200,
            "container_id":"0123456789abcdef",
            "image_digest":"sha256:source",
            "dynamips_id":1,
            "qemu_pid":1234,
            "pid_file":runtime_paths[6].1,
            "console":5001
        });
        sqlx::query("INSERT INTO topology_objects(project_id,kind,object_id,name,object_json) VALUES(?, 'node', ?, ?, ?)")
            .bind(source_project_id.to_string())
            .bind(source_node_id.to_string())
            .bind("Source router")
            .bind(source_node.to_string())
            .execute(store.pool())
            .await
            .unwrap();

        let duplicate_project = json_body(
            send(
                &app,
                Method::POST,
                &format!("/v3/projects/{source_project_id}/duplicate"),
                Body::from("{}"),
                Some("application/json"),
            )
            .await,
        )
        .await;
        let duplicate_project_id = duplicate_project["project_id"].as_str().unwrap();
        assert_node_has_no_runtime_state(&duplicate_project["nodes"][0]);
        let duplicate_project_node = sqlx::query(
            "SELECT object_json FROM topology_objects WHERE project_id=? AND kind='node'",
        )
        .bind(duplicate_project_id)
        .fetch_one(store.pool())
        .await
        .unwrap();
        let duplicate_project_node: Value = serde_json::from_str(
            duplicate_project_node
                .get::<String, _>("object_json")
                .as_str(),
        )
        .unwrap();
        assert_node_has_no_runtime_state(&duplicate_project_node);
        assert_eq!(
            send(
                &app,
                Method::DELETE,
                &format!("/v3/projects/{duplicate_project_id}"),
                Body::empty(),
                None,
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );

        let duplicate_node = json_body(
            send(
                &app,
                Method::POST,
                &format!("/v3/projects/{source_project_id}/nodes/{source_node_id}/duplicate"),
                Body::from("{}"),
                Some("application/json"),
            )
            .await,
        )
        .await;
        assert_node_has_no_runtime_state(&duplicate_node);
        let duplicate_node_id = duplicate_node["node_id"].as_str().unwrap();
        assert_eq!(
            send(
                &app,
                Method::DELETE,
                &format!("/v3/projects/{source_project_id}/nodes/{duplicate_node_id}"),
                Body::empty(),
                None,
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        for (name, path) in &runtime_paths {
            assert_eq!(
                tokio::fs::read(path).await.unwrap(),
                format!("source runtime: {name}").as_bytes(),
                "duplicate deletion changed {name}"
            );
        }
        let source_node: Value = sqlx::query_scalar::<_, String>(
            "SELECT object_json FROM topology_objects WHERE project_id=? AND kind='node' AND object_id=?",
        )
        .bind(source_project_id.to_string())
        .bind(source_node_id.to_string())
        .fetch_one(store.pool())
        .await
        .map(|json| serde_json::from_str(&json).unwrap())
        .unwrap();
        assert_eq!(
            source_node["runtime_overlay"],
            runtime_paths[0].1.to_string_lossy().as_ref()
        );
        assert_eq!(source_node["container_id"], "0123456789abcdef");
        assert_eq!(source_node["dynamips_id"], 1);

        let import_node_id = Uuid::new_v4();
        let unsafe_project_path = directory.path().join("unsafe-project.gns3");
        let unsafe_project = json!({
            "name":"Imported unsafe topology",
            "nodes":[{
                "node_id":import_node_id,
                "node_type":"qemu",
                "name":"Imported router",
                "status":"started",
                "hda_disk_image":"/images/router.qcow2",
                "runtime_overlay":runtime_paths[0].1,
                "runtime_qmp_socket":runtime_paths[1].1,
                "runtime_serial_socket":runtime_paths[2].1,
                "runtime_pid_file":runtime_paths[3].1,
                "runtime_log_file":runtime_paths[4].1,
                "runtime_hdb_overlay":runtime_paths[5].1,
                "runtime_working_dir":runtime_dir.display().to_string(),
                "runtime_hypervisor_port":7200,
                "container_id":"0123456789abcdef",
                "image_digest":"sha256:source",
                "dynamips_id":1,
                "qemu_pid":1234,
                "pid_file":runtime_paths[6].1
            }],
            "links":[],
            "drawings":[]
        });
        tokio::fs::write(&unsafe_project_path, unsafe_project.to_string())
            .await
            .unwrap();
        let loaded = json_body(
            send(
                &app,
                Method::POST,
                "/v3/projects/load",
                Body::from(json!({"path":unsafe_project_path}).to_string()),
                Some("application/json"),
            )
            .await,
        )
        .await;
        assert_eq!(loaded["nodes"][0]["status"], "stopped");
        assert_node_has_no_runtime_state(&loaded["nodes"][0]);
        let loaded_project_id = loaded["project_id"].as_str().unwrap();
        let loaded_node: Value = sqlx::query_scalar::<_, String>(
            "SELECT object_json FROM topology_objects WHERE project_id=? AND kind='node'",
        )
        .bind(loaded_project_id)
        .fetch_one(store.pool())
        .await
        .map(|json| serde_json::from_str(&json).unwrap())
        .unwrap();
        assert_node_has_no_runtime_state(&loaded_node);
        assert_eq!(
            send(
                &app,
                Method::DELETE,
                &format!("/v3/projects/{loaded_project_id}"),
                Body::empty(),
                None,
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        for (_, path) in &runtime_paths {
            assert!(path.exists(), "import deletion removed {}", path.display());
        }
    }

    #[tokio::test]
    async fn load_project_rejects_non_regular_and_oversized_files() {
        let (app, directory, _store) = app().await;
        let non_regular = send(
            &app,
            Method::POST,
            "/v3/projects/load",
            Body::from(json!({"path":directory.path().to_string_lossy()}).to_string()),
            Some("application/json"),
        )
        .await;
        assert_eq!(non_regular.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            json_body(non_regular).await["message"]
                .as_str()
                .unwrap()
                .contains("regular file")
        );

        let oversized_path = directory.path().join("oversized.gns3");
        let oversized = std::fs::File::create(&oversized_path).unwrap();
        oversized.set_len(64 * 1024 * 1024 + 1).unwrap();
        let too_large = send(
            &app,
            Method::POST,
            "/v3/projects/load",
            Body::from(json!({"path":oversized_path.to_string_lossy()}).to_string()),
            Some("application/json"),
        )
        .await;
        assert_eq!(too_large.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn malformed_zip_import_returns_http_400() {
        let (app, _directory, _store) = app().await;
        let mut malformed = vec![0_u8; 30];
        malformed[..4].copy_from_slice(b"PK\x03\x04");
        malformed[26..28].copy_from_slice(&u16::MAX.to_le_bytes());
        let response = send(
            &app,
            Method::POST,
            &format!("/v3/projects/{}/import", Uuid::new_v4()),
            Body::from(malformed),
            Some("application/gns3project"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn node_deletion_cleans_file_rows_and_database_trigger_covers_direct_deletes() {
        let (app, _directory, store) = app().await;
        let project = json_body(
            send(
                &app,
                Method::POST,
                "/v3/projects",
                Body::from(r#"{"name":"Node file cleanup"}"#),
                Some("application/json"),
            )
            .await,
        )
        .await;
        let project_id = project["project_id"].as_str().unwrap();
        let node_id = Uuid::new_v4();
        let node = json!({"node_id":node_id,"project_id":project_id,"node_type":"ethernet_switch","name":"Switch","status":"stopped"});
        sqlx::query("INSERT INTO topology_objects(project_id,kind,object_id,name,object_json) VALUES(?, 'node', ?, 'Switch', ?)")
            .bind(project_id)
            .bind(node_id.to_string())
            .bind(node.to_string())
            .execute(store.pool())
            .await
            .unwrap();
        assert_eq!(
            send(
                &app,
                Method::POST,
                &format!("/v3/projects/{project_id}/nodes/{node_id}/files/config.cfg"),
                Body::from("switch config"),
                None,
            )
            .await
            .status(),
            StatusCode::CREATED
        );
        assert_eq!(
            send(
                &app,
                Method::DELETE,
                &format!("/v3/projects/{project_id}/nodes/{node_id}"),
                Body::empty(),
                None,
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM node_files WHERE project_id=? AND node_id=?")
                .bind(project_id)
                .bind(node_id.to_string())
                .fetch_one(store.pool())
                .await
                .unwrap();
        assert_eq!(count, 0);

        let direct_delete_id = Uuid::new_v4();
        let direct_node = json!({"node_id":direct_delete_id,"project_id":project_id,"node_type":"ethernet_switch","name":"Direct switch"});
        sqlx::query("INSERT INTO topology_objects(project_id,kind,object_id,name,object_json) VALUES(?, 'node', ?, 'Direct switch', ?)")
            .bind(project_id)
            .bind(direct_delete_id.to_string())
            .bind(direct_node.to_string())
            .execute(store.pool())
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO node_files(project_id,node_id,file_path,contents) VALUES(?,?,?,?)",
        )
        .bind(project_id)
        .bind(direct_delete_id.to_string())
        .bind("startup.cfg")
        .bind(b"config".as_slice())
        .execute(store.pool())
        .await
        .unwrap();
        sqlx::query(
            "DELETE FROM topology_objects WHERE project_id=? AND kind='node' AND object_id=?",
        )
        .bind(project_id)
        .bind(direct_delete_id.to_string())
        .execute(store.pool())
        .await
        .unwrap();
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM node_files WHERE project_id=? AND node_id=?")
                .bind(project_id)
                .bind(direct_delete_id.to_string())
                .fetch_one(store.pool())
                .await
                .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn template_crud_and_duplicate_are_http_accessible() {
        let (app, _dir, _store) = app().await;
        let response = send(
            &app,
            Method::POST,
            "/v3/templates",
            Body::from(
                json!({"name":"Test VPCS","template_type":"vpcs","startup_script":""}).to_string(),
            ),
            Some("application/json"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let template = json_body(response).await;
        let id = template["template_id"].as_str().unwrap();
        assert_eq!(
            send(
                &app,
                Method::GET,
                &format!("/v3/templates/{id}"),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::OK
        );
        let response = send(
            &app,
            Method::PUT,
            &format!("/v3/templates/{id}"),
            Body::from(r#"{"name":"Updated VPCS"}"#),
            Some("application/json"),
        )
        .await;
        assert_eq!(json_body(response).await["name"], "Updated VPCS");
        let response = send(
            &app,
            Method::POST,
            &format!("/v3/templates/{id}/duplicate"),
            Body::empty(),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let duplicate = json_body(response).await;
        assert_eq!(duplicate["name"], "Updated VPCS copy");
        let response = send(&app, Method::GET, "/v3/templates", Body::empty(), None).await;
        assert!(
            json_body(response)
                .await
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["template_id"] == id)
        );
        assert_eq!(
            send(
                &app,
                Method::DELETE,
                &format!(
                    "/v3/templates/{}",
                    duplicate["template_id"].as_str().unwrap()
                ),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
    }

    #[tokio::test]
    async fn controller_version_matches_the_upstream_shape_and_check_route() {
        let (app, _dir, _store) = app().await;
        let response = send(&app, Method::GET, "/v3/version", Body::empty(), None).await;
        assert_eq!(
            json_body(response).await,
            json!({"controller_host":null,"version":"3.1.0.dev7","local":true})
        );
        let response = send(
            &app,
            Method::POST,
            "/v3/version",
            Body::from(r#"{"version":"3.1.0.dev7"}"#),
            Some("application/json"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(json_body(response).await, json!({"version":"3.1.0.dev7"}));
        assert_eq!(
            send(
                &app,
                Method::POST,
                "/v3/version",
                Body::from(r#"{"version":"different"}"#),
                Some("application/json"),
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
    }

    #[tokio::test]
    async fn project_stats_lock_snapshots_files_export_import_and_duplicate_work_over_http() {
        let (app, _dir, _store) = app().await;
        let response = send(
            &app,
            Method::POST,
            "/v3/projects",
            Body::from(r#"{"name":"API parity lab"}"#),
            Some("application/json"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let project = json_body(response).await;
        let id = project["project_id"].as_str().unwrap();
        assert_eq!(
            json_body(
                send(
                    &app,
                    Method::GET,
                    &format!("/v3/projects/{id}/stats"),
                    Body::empty(),
                    None
                )
                .await
            )
            .await["nodes"],
            0
        );
        assert_eq!(
            json_body(
                send(
                    &app,
                    Method::GET,
                    &format!("/v3/projects/{id}/locked"),
                    Body::empty(),
                    None
                )
                .await
            )
            .await,
            false
        );
        assert_eq!(
            send(
                &app,
                Method::POST,
                &format!("/v3/projects/{id}/lock"),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        let project_stream = send(
            &app,
            Method::GET,
            &format!("/v3/projects/{id}/notifications"),
            Body::empty(),
            None,
        )
        .await;
        assert_eq!(project_stream.status(), StatusCode::OK);
        assert_eq!(
            project_stream.headers()["content-type"],
            "application/x-ndjson"
        );
        assert_eq!(
            json_body(
                send(
                    &app,
                    Method::GET,
                    &format!("/v3/projects/{id}/locked"),
                    Body::empty(),
                    None
                )
                .await
            )
            .await,
            true
        );
        assert_eq!(
            send(
                &app,
                Method::POST,
                &format!("/v3/projects/{id}/unlock"),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        let snapshot = json_body(
            send(
                &app,
                Method::POST,
                &format!("/v3/projects/{id}/snapshots"),
                Body::from(r#"{"name":"initial","description":"test"}"#),
                Some("application/json"),
            )
            .await,
        )
        .await;
        let snapshot_id = snapshot["snapshot_id"].as_str().unwrap();
        assert_eq!(
            json_body(
                send(
                    &app,
                    Method::GET,
                    &format!("/v3/projects/{id}/snapshots"),
                    Body::empty(),
                    None
                )
                .await
            )
            .await
            .as_array()
            .unwrap()
            .len(),
            1
        );
        assert_eq!(
            send(
                &app,
                Method::POST,
                &format!("/v3/projects/{id}/snapshots/{snapshot_id}/restore"),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            send(
                &app,
                Method::DELETE,
                &format!("/v3/projects/{id}/snapshots/{snapshot_id}"),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            send(
                &app,
                Method::POST,
                &format!("/v3/projects/{id}/nodes/start"),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            send(
                &app,
                Method::POST,
                &format!("/v3/projects/{id}/nodes/stop"),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            send(
                &app,
                Method::POST,
                &format!("/v3/projects/{id}/nodes/suspend"),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            send(
                &app,
                Method::POST,
                &format!("/v3/projects/{id}/nodes/reload"),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            send(
                &app,
                Method::POST,
                &format!("/v3/projects/{id}/nodes/console/reset"),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::NOT_IMPLEMENTED
        );
        let file_response = send(
            &app,
            Method::POST,
            &format!("/v3/projects/{id}/files/readme.txt"),
            Body::from("lab"),
            None,
        )
        .await;
        assert_eq!(file_response.status(), StatusCode::NO_CONTENT);
        let file = send(
            &app,
            Method::GET,
            &format!("/v3/projects/{id}/files/readme.txt"),
            Body::empty(),
            None,
        )
        .await
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
        assert_eq!(&file[..], b"lab");
        assert_eq!(
            send(
                &app,
                Method::GET,
                &format!("/v3/projects/{id}/gns3file"),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::OK
        );
        let archive = send(
            &app,
            Method::GET,
            &format!("/v3/projects/{id}/export"),
            Body::empty(),
            None,
        )
        .await
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
        assert_eq!(&archive[..2], b"PK");
        let new_id = uuid::Uuid::new_v4();
        let response = send(
            &app,
            Method::POST,
            &format!("/v3/projects/{new_id}/import?name=Imported"),
            Body::from(archive),
            Some("application/gns3project"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(json_body(response).await["name"], "Imported");
        let imported_file = send(
            &app,
            Method::GET,
            &format!("/v3/projects/{new_id}/files/readme.txt"),
            Body::empty(),
            None,
        )
        .await
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes();
        assert_eq!(&imported_file[..], b"lab");
        let response = send(
            &app,
            Method::POST,
            &format!("/v3/projects/{id}/duplicate"),
            Body::from(r#"{"name":"Cloned lab"}"#),
            Some("application/json"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(json_body(response).await["name"], "Cloned lab");
    }

    #[tokio::test]
    async fn settings_computes_symbols_and_node_file_actions_are_http_accessible() {
        let (app, _dir, store) = app().await;
        let response = send(
            &app,
            Method::PUT,
            "/v3/settings",
            Body::from(r#"{"Server":{"console_start_port_range":5000}}"#),
            Some("application/json"),
        )
        .await;
        assert_eq!(
            json_body(response).await["Server"]["console_start_port_range"],
            5000
        );
        assert_eq!(
            json_body(send(&app, Method::GET, "/v3/settings", Body::empty(), None).await).await["Server"]
                ["console_start_port_range"],
            5000
        );
        assert_eq!(
            json_body(send(&app, Method::GET, "/v3/computes", Body::empty(), None).await).await[0]
                ["compute_id"],
            "local"
        );
        assert_eq!(
            json_body(send(&app, Method::GET, "/v3/computes/local", Body::empty(), None).await)
                .await["connected"],
            true
        );
        assert!(
            json_body(
                send(
                    &app,
                    Method::GET,
                    "/v3/symbols/default_symbols",
                    Body::empty(),
                    None
                )
                .await
            )
            .await
            .is_object()
        );
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&320u32.to_be_bytes());
        png.extend_from_slice(&200u32.to_be_bytes());
        assert_eq!(
            send(
                &app,
                Method::POST,
                "/v3/symbols/custom.png/raw",
                Body::from(png.clone()),
                Some("application/octet-stream")
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            json_body(
                send(
                    &app,
                    Method::GET,
                    "/v3/symbols/custom.png/dimensions",
                    Body::empty(),
                    None
                )
                .await
            )
            .await,
            json!({"width":320,"height":200})
        );
        let raw_symbol = send(
            &app,
            Method::GET,
            "/v3/symbols/custom.png/raw",
            Body::empty(),
            None,
        )
        .await;
        assert_eq!(
            raw_symbol.headers()["content-disposition"],
            "attachment; filename=\"custom.png\""
        );
        assert!(
            raw_symbol.headers()["content-security-policy"]
                .to_str()
                .unwrap()
                .contains("sandbox")
        );
        assert_eq!(raw_symbol.headers()["x-content-type-options"], "nosniff");
        assert_eq!(
            raw_symbol.into_body().collect().await.unwrap().to_bytes()[..],
            png[..]
        );
        assert_eq!(
            send(
                &app,
                Method::DELETE,
                "/v3/symbols/custom.png",
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        assert!(
            json_body(send(&app, Method::GET, "/v3/symbols", Body::empty(), None).await)
                .await
                .as_array()
                .unwrap()
                .is_empty()
        );
        let active_svg =
            br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#;
        assert_eq!(
            send(
                &app,
                Method::POST,
                "/v3/symbols/active.svg/raw",
                Body::from(active_svg.as_slice()),
                Some("image/svg+xml"),
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        let svg_response = send(
            &app,
            Method::GET,
            "/v3/symbols/active.svg/raw",
            Body::empty(),
            None,
        )
        .await;
        assert_eq!(svg_response.headers()["content-type"], "image/svg+xml");
        assert!(
            svg_response.headers()["content-disposition"]
                .to_str()
                .unwrap()
                .starts_with("attachment;")
        );
        assert!(
            svg_response.headers()["content-security-policy"]
                .to_str()
                .unwrap()
                .contains("sandbox")
        );
        assert_eq!(svg_response.headers()["x-content-type-options"], "nosniff");
        assert_eq!(
            svg_response.into_body().collect().await.unwrap().to_bytes()[..],
            active_svg[..]
        );
        assert_eq!(
            send(
                &app,
                Method::DELETE,
                "/v3/symbols/active.svg",
                Body::empty(),
                None,
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            send(&app, Method::GET, "/v3/statistics", Body::empty(), None)
                .await
                .status(),
            StatusCode::OK
        );
        let notifications = send(&app, Method::GET, "/v3/notifications", Body::empty(), None).await;
        assert_eq!(notifications.status(), StatusCode::OK);
        assert_eq!(
            notifications.headers()["content-type"],
            "application/x-ndjson"
        );
        let images = send(&app, Method::GET, "/v3/images", Body::empty(), None).await;
        assert_eq!(images.status(), StatusCode::OK);
        assert!(json_body(images).await.is_array());
        assert_eq!(
            send(
                &app,
                Method::GET,
                "/v3/images?image_type=bogus",
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            send(
                &app,
                Method::GET,
                "/v3/images/no-such-image",
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send(
                &app,
                Method::DELETE,
                "/v3/images/no-such-image",
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send(
                &app,
                Method::POST,
                "/v3/images/upload/empty.qcow2",
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        let project_id = uuid::Uuid::new_v4();
        let node_id = uuid::Uuid::new_v4();
        let create = send(
            &app,
            Method::POST,
            "/v3/projects",
            Body::from(format!(
                r#"{{"project_id":"{project_id}","name":"node files"}}"#
            )),
            Some("application/json"),
        )
        .await;
        assert_eq!(create.status(), StatusCode::CREATED);
        let node = json!({"node_id":node_id,"project_id":project_id,"name":"host","node_type":"vpcs","status":"stopped","x":0,"y":0,"locked":false});
        sqlx::query("INSERT INTO topology_objects(project_id,kind,object_id,name,object_json) VALUES(?,?,?,?,?)")
            .bind(project_id.to_string()).bind("node").bind(node_id.to_string()).bind("host").bind(node.to_string()).execute(store.pool()).await.unwrap();
        assert_eq!(
            json_body(
                send(
                    &app,
                    Method::GET,
                    &format!("/v3/projects/{project_id}/nodes/{node_id}/files"),
                    Body::empty(),
                    None
                )
                .await
            )
            .await
            .as_array()
            .unwrap()
            .len(),
            0
        );
        assert_eq!(
            send(
                &app,
                Method::POST,
                &format!("/v3/projects/{project_id}/nodes/{node_id}/files/config/startup.cfg"),
                Body::from("hostname lab"),
                None
            )
            .await
            .status(),
            StatusCode::CREATED
        );
        assert_eq!(
            send(
                &app,
                Method::GET,
                &format!("/v3/projects/{project_id}/nodes/{node_id}/files/config/startup.cfg"),
                Body::empty(),
                None
            )
            .await
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()[..],
            b"hostname lab"[..]
        );
        assert_eq!(
            send(
                &app,
                Method::POST,
                &format!("/v3/projects/{project_id}/nodes/{node_id}/isolate"),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            json_body(
                send(
                    &app,
                    Method::GET,
                    &format!("/v3/projects/{project_id}/nodes/{node_id}"),
                    Body::empty(),
                    None
                )
                .await
            )
            .await["isolated"],
            true
        );
        assert_eq!(
            send(
                &app,
                Method::POST,
                &format!("/v3/projects/{project_id}/nodes/{node_id}/unisolate"),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            send(
                &app,
                Method::POST,
                &format!("/v3/projects/{project_id}/nodes/{node_id}/console/reset"),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::NOT_IMPLEMENTED
        );
        assert!(
            json_body(
                send(
                    &app,
                    Method::GET,
                    &format!("/v3/projects/{project_id}/nodes/{node_id}/links"),
                    Body::empty(),
                    None
                )
                .await
            )
            .await
            .as_array()
            .unwrap()
            .is_empty()
        );
        assert_eq!(
            send(
                &app,
                Method::GET,
                &format!("/v3/projects/{project_id}/nodes/{node_id}/dynamips/auto_idlepc"),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            send(
                &app,
                Method::GET,
                &format!("/v3/projects/{project_id}/nodes/{node_id}/dynamips/idlepc_proposals"),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        for method in [Method::POST, Method::PUT, Method::DELETE] {
            assert_eq!(
                send(
                    &app,
                    method,
                    &format!(
                        "/v3/projects/{project_id}/nodes/{node_id}/qemu/disk_image/extra.qcow2"
                    ),
                    Body::from(r#"{"size":"1G"}"#),
                    Some("application/json")
                )
                .await
                .status(),
                StatusCode::UNPROCESSABLE_ENTITY
            );
        }
        let duplicate = send(
            &app,
            Method::POST,
            &format!("/v3/projects/{project_id}/nodes/{node_id}/duplicate"),
            Body::from(r#"{"x":50,"y":50,"z":1}"#),
            Some("application/json"),
        )
        .await;
        assert_eq!(duplicate.status(), StatusCode::CREATED);
        assert_eq!(
            send(
                &app,
                Method::DELETE,
                &format!("/v3/projects/{project_id}/nodes/{node_id}/files/config/startup.cfg"),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
    }

    #[tokio::test]
    async fn dynamips_idlepc_routes_use_the_hypervisor_protocol() {
        let dir = TempDir::new().expect("test directory");
        let store = Store::connect(dir.path().join("api.sqlite").to_str().unwrap())
            .await
            .unwrap();
        let image_dir = dir.path().join("ios");
        let state_dir = dir.path().join("dynamips-state");
        std::fs::create_dir_all(&image_dir).unwrap();
        std::fs::create_dir_all(&state_dir).unwrap();
        let image = image_dir.join("router.image");
        std::fs::write(&image, [0x7f, b'E', b'L', b'F', 1, 2, 1, 0]).unwrap();
        let mut state = AppState::new(store.clone());
        state.dynamips =
            crate::dynamips::DynamipsRuntime::with_paths("dynamips", &image_dir, &state_dir);
        let app = router(state);
        let project_id = uuid::Uuid::new_v4();
        let project = send(
            &app,
            Method::POST,
            "/v3/projects",
            Body::from(format!(
                r#"{{"project_id":"{project_id}","name":"IOS lab"}}"#
            )),
            Some("application/json"),
        )
        .await;
        assert_eq!(project.status(), StatusCode::CREATED);

        let proposals_node = uuid::Uuid::new_v4();
        let auto_node = uuid::Uuid::new_v4();
        let proposals_port = mock_dynamips(vec![
            ("vm get_status \"R1\"", "100 2\r\n100-OK\r\n"),
            (
                "vm get_idle_pc_prop \"R1\" 0",
                "100 0x1234abcd 90%\r\n100-OK\r\n",
            ),
        ])
        .await;
        let auto_port = mock_dynamips(vec![
            ("vm get_status \"R2\"", "100 2\r\n100-OK\r\n"),
            ("vm set_idle_pc_online \"R2\" 0 0x0", "100-OK\r\n"),
            (
                "vm get_idle_pc_prop \"R2\" 0",
                "100 0x1234abcd 90%\r\n100-OK\r\n",
            ),
            ("vm set_idle_pc_online \"R2\" 0 0x1234abcd", "100-OK\r\n"),
            ("vm cpu_usage \"R2\" 0", "100 100\r\n100-OK\r\n"),
            ("vm cpu_usage \"R2\" 0", "100 102\r\n100-OK\r\n"),
        ])
        .await;
        for (node_id, name, port) in [
            (proposals_node, "R1", proposals_port),
            (auto_node, "R2", auto_port),
        ] {
            let working_dir = state_dir
                .join(project_id.to_string())
                .join(node_id.to_string());
            std::fs::create_dir_all(&working_dir).unwrap();
            std::fs::write(
                working_dir.join(".dynamips-runtime"),
                format!("{project_id}\n{node_id}\n{port}\n0\n0\n"),
            )
            .unwrap();
            let node = json!({
                "node_id":node_id,
                "project_id":project_id,
                "name":name,
                "node_type":"dynamips",
                "platform":"c7200",
                "slot0":"C7200-IO-FE",
                "image":image.display().to_string(),
                "ram":512,
                "runtime_working_dir":working_dir.display().to_string(),
                "runtime_log_file":working_dir.join("dynamips_stdout.log").display().to_string(),
                "runtime_hypervisor_port":port,
                "status":"started",
                "x":0,
                "y":0,
                "locked":false
            });
            sqlx::query("INSERT INTO topology_objects(project_id,kind,object_id,name,object_json) VALUES(?,?,?,?,?)")
                .bind(project_id.to_string())
                .bind("node")
                .bind(node_id.to_string())
                .bind(name)
                .bind(node.to_string())
                .execute(store.pool())
                .await
                .unwrap();
        }

        let proposals = send(
            &app,
            Method::GET,
            &format!("/v3/projects/{project_id}/nodes/{proposals_node}/dynamips/idlepc_proposals"),
            Body::empty(),
            None,
        )
        .await;
        assert_eq!(proposals.status(), StatusCode::OK);
        assert_eq!(json_body(proposals).await, json!(["0x1234abcd 90%"]));

        let auto = send(
            &app,
            Method::GET,
            &format!("/v3/projects/{project_id}/nodes/{auto_node}/dynamips/auto_idlepc"),
            Body::empty(),
            None,
        )
        .await;
        assert_eq!(auto.status(), StatusCode::OK);
        assert_eq!(json_body(auto).await, json!({"idlepc":"0x1234abcd"}));
    }

    #[tokio::test]
    async fn project_archive_loading_reads_a_local_gns3_file() {
        let (app, dir, _store) = app().await;
        let path = dir.path().join("local.gns3");
        tokio::fs::write(
            &path,
            r#"{"name":"Loaded lab","nodes":[],"links":[],"drawings":[]}"#,
        )
        .await
        .unwrap();
        let response = send(
            &app,
            Method::POST,
            "/v3/projects/load",
            Body::from(json!({"path":path}).to_string()),
            Some("application/json"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(json_body(response).await["name"], "Loaded lab");
    }

    #[tokio::test]
    async fn controller_and_project_notification_websockets_upgrade_over_http() {
        let (app, _dir, _store) = app().await;
        let response = send(
            &app,
            Method::POST,
            "/v3/projects",
            Body::from(r#"{"name":"WS lab"}"#),
            Some("application/json"),
        )
        .await;
        let project = json_body(response).await;
        let project_id = project["project_id"].as_str().unwrap();
        assert_eq!(
            websocket_status(&app, "/v3/notifications/ws").await,
            StatusCode::SWITCHING_PROTOCOLS
        );
        assert_eq!(
            websocket_status(&app, &format!("/v3/projects/{project_id}/notifications/ws")).await,
            StatusCode::SWITCHING_PROTOCOLS
        );
    }
}
