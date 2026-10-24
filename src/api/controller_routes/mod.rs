use std::{
    collections::{HashMap, HashSet},
    io::{self, Cursor, Read, Write},
    path::{Path, PathBuf},
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

fn local_compute(state: &AppState) -> Value {
    json!({
        "compute_id": "local",
        "name": "Local",
        "host": state.bind_address.ip().to_string(),
        "port": state.bind_address.port(),
        "protocol": "http",
        "auto_connect": true,
        "connected": true
    })
}
async fn list_computes(State(state): State<AppState>) -> Json<Vec<Value>> {
    Json(vec![local_compute(&state)])
}
async fn get_compute(
    State(state): State<AppState>,
    AxumPath(compute_id): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    if compute_id == "local" {
        Ok(Json(local_compute(&state)))
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
    let project = fetch_project(state.store.pool(), &pid).await?;
    let nodes = list_objects(state.store.pool(), &pid, "node").await?.0;
    let links = list_objects(state.store.pool(), &pid, "link").await?.0;
    let drawings = list_objects(state.store.pool(), &pid, "drawing").await?.0;
    Ok(Json(project_api_to_gns3(project, nodes, links, drawings)))
}

fn project_api_to_gns3(
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

fn api_node_to_gns3(mut node: Value) -> Value {
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

fn normalize_imported_project(mut project: Value) -> Value {
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

fn package_project_images(project: &mut Value) -> Result<Vec<(String, Vec<u8>)>, ApiError> {
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

fn unique_project_image_name(
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
struct ProjectExportQuery {
    #[serde(default)]
    include_images: bool,
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
    let image_map = install_imported_project_images(&images)?;
    remap_imported_project_images(&mut project, &image_map);
    regenerate_imported_project_ids(&mut project, &project_id.to_string());
    strip_project_runtime_fields(&mut project);
    project_data = project.clone();
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

async fn rollback_imported_project(state: &AppState, project_id: &str, node_ids: &[String]) {
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
    let mut first_error = None;
    for node in nodes {
        if !bulk_node_action_applies(&node, action) {
            continue;
        }
        let id = node["node_id"]
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or_else(|| ApiError::bad_request("node has invalid ID"))?;
        if let Err(error) = change_node_runtime_state(state.clone(), project_id, id, action).await {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(super) fn bulk_node_action_applies(node: &Value, action: NodeAction) -> bool {
    let node_type = node
        .get("node_type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let status = node
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("stopped");

    match node_type {
        "ethernet_switch" | "ethernet_hub" => false,
        "cloud" | "nat" if !matches!(action, NodeAction::Start) => false,
        "vpcs" if matches!(action, NodeAction::Suspend) => false,
        "docker" | "qemu" | "vpcs" | "dynamips" | "cloud" | "nat" => match action {
            NodeAction::Start => status != "started",
            NodeAction::Stop => status == "started" || status == "suspended",
            NodeAction::Suspend | NodeAction::Reload => status == "started",
        },
        _ => false,
    }
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
    images: Vec<ImportedProjectImage>,
}

#[derive(Debug)]
struct ImportedProjectImage {
    image_type: String,
    file_name: String,
    contents: Vec<u8>,
}

fn write_project_archive(
    topology: Vec<u8>,
    project_files: Vec<(String, Vec<u8>)>,
) -> Result<Vec<u8>, ApiError> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
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
                project: normalize_imported_project(project),
                project_files: Vec::new(),
                images: Vec::new(),
            })
            .map_err(|error| {
                archive_bad_request(format!("Invalid .gns3project archive or JSON: {error}"))
            });
    }

    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| archive_bad_request(format!("Invalid .gns3project ZIP: {error}")))?;
    let mut topology = None;
    let mut project_files = Vec::new();
    let mut images = Vec::new();
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
        let image_path = archive_project_image_path(&name)?;
        let is_topology = name == "project.gns3" || (topology.is_none() && name.ends_with(".gns3"));
        if !is_topology && project_file_path.is_none() && image_path.is_none() {
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
        } else if let Some((image_type, file_name)) = image_path {
            images.push(ImportedProjectImage {
                image_type,
                file_name,
                contents,
            });
        }
    }
    let topology = topology
        .ok_or_else(|| archive_bad_request("Project archive contains no .gns3 topology file"))?;
    let project = serde_json::from_slice(&topology).map_err(|error| {
        archive_bad_request(format!("Project archive topology is invalid: {error}"))
    })?;
    Ok(ImportedProjectArchive {
        project: normalize_imported_project(project),
        project_files,
        images,
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

fn archive_project_image_path(name: &str) -> Result<Option<(String, String)>, ApiError> {
    let Some(path) = name.strip_prefix("images/") else {
        return Ok(None);
    };
    let Some((directory, file_name)) = path.split_once('/') else {
        return Ok(None);
    };
    let image_type = match directory.to_ascii_lowercase().as_str() {
        "qemu" => "qemu",
        "ios" | "dynamips" => "dynamips",
        _ => return Ok(None),
    };
    if file_name.contains('/') || file_name.is_empty() {
        return Err(archive_bad_request(
            "Project archive image entries must use a simple file name",
        ));
    }
    let safe_name =
        clean_relative_path(file_name).map_err(|error| archive_bad_request(error.message))?;
    if safe_name.contains('/') {
        return Err(archive_bad_request(
            "Project archive image entries must use a simple file name",
        ));
    }
    Ok(Some((image_type.to_owned(), safe_name)))
}

fn install_imported_project_images(
    images: &[ImportedProjectImage],
) -> Result<HashMap<(String, String), String>, ApiError> {
    let mut installed = HashMap::new();
    for image in images {
        let directory = match image.image_type.as_str() {
            "qemu" => crate::image_paths::qemu_images(),
            "dynamips" => crate::image_paths::dynamips_images(),
            _ => continue,
        };
        std::fs::create_dir_all(&directory).map_err(|error| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Could not create imported image directory: {error}"),
            )
        })?;
        let destination_name =
            imported_image_destination_name(&directory, &image.file_name, &image.contents)?;
        let destination = directory.join(&destination_name);
        if !destination.exists() {
            let temporary = directory.join(format!(".import-{}.tmp", Uuid::new_v4()));
            let result = (|| -> std::io::Result<()> {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temporary)?;
                file.write_all(&image.contents)?;
                file.sync_all()?;
                std::fs::rename(&temporary, &destination)
            })();
            if let Err(error) = result {
                let _ = std::fs::remove_file(&temporary);
                if !destination.is_file() {
                    return Err(ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!(
                            "Could not install imported image '{}': {error}",
                            image.file_name
                        ),
                    ));
                }
            }
        }
        installed.insert(
            (image.image_type.clone(), image.file_name.clone()),
            destination_name,
        );
    }
    Ok(installed)
}

fn imported_image_destination_name(
    directory: &Path,
    file_name: &str,
    contents: &[u8],
) -> Result<String, ApiError> {
    let safe_name = clean_relative_path(file_name)?;
    if safe_name.contains('/') {
        return Err(archive_bad_request(
            "Imported image file names must be simple file names",
        ));
    }
    let requested = directory.join(&safe_name);
    if !requested.exists() || std::fs::read(&requested).ok().as_deref() == Some(contents) {
        return Ok(safe_name);
    }
    let path = Path::new(&safe_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("image");
    let extension = path.extension().and_then(|value| value.to_str());
    let digest = format!("{:x}", md5::Md5::digest(contents));
    for index in 0..1000 {
        let suffix = if index == 0 {
            format!("-import-{}", &digest[..8])
        } else {
            format!("-import-{}-{index}", &digest[..8])
        };
        let candidate = match extension {
            Some(extension) => format!("{stem}{suffix}.{extension}"),
            None => format!("{stem}{suffix}"),
        };
        let path = directory.join(&candidate);
        if !path.exists() || std::fs::read(&path).ok().as_deref() == Some(contents) {
            return Ok(candidate);
        }
    }
    Err(ApiError::conflict(
        "Could not allocate a unique name for an imported image",
    ))
}

fn remap_imported_project_images(
    project: &mut Value,
    image_map: &HashMap<(String, String), String>,
) {
    let Some(nodes) = project.get_mut("nodes").and_then(Value::as_array_mut) else {
        return;
    };
    for node in nodes {
        let image_type = match node.get("node_type").and_then(Value::as_str) {
            Some("qemu") => "qemu",
            Some("dynamips") => "dynamips",
            _ => continue,
        };
        let fields: &[&str] = if image_type == "qemu" {
            &[
                "hda_disk_image",
                "hdb_disk_image",
                "hdc_disk_image",
                "hdd_disk_image",
                "cdrom_image",
            ]
        } else {
            &["image"]
        };
        for field in fields {
            let Some(reference) = node.get(*field).and_then(Value::as_str) else {
                continue;
            };
            let file_name = Path::new(reference)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(reference)
                .to_owned();
            if let Some(imported_name) = image_map.get(&(image_type.to_owned(), file_name)) {
                node[*field] = json!(imported_name);
            }
        }
    }
}

fn regenerate_imported_project_ids(project: &mut Value, project_id: &str) {
    project["project_id"] = json!(project_id);
    let mut node_ids = HashMap::new();
    if let Some(nodes) = project.get_mut("nodes").and_then(Value::as_array_mut) {
        for node in nodes {
            let old_id = node
                .get("node_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let new_id = Uuid::new_v4().to_string();
            if let Some(old_id) = old_id {
                node_ids.insert(old_id, new_id.clone());
            }
            node["node_id"] = json!(new_id);
            node["project_id"] = json!(project_id);
            node["compute_id"] = json!("local");
            node["status"] = json!("stopped");
            strip_node_runtime_fields(node);
        }
    }
    if let Some(links) = project.get_mut("links").and_then(Value::as_array_mut) {
        for link in links {
            link["link_id"] = json!(Uuid::new_v4());
            link["project_id"] = json!(project_id);
            if let Some(endpoints) = link.get_mut("nodes").and_then(Value::as_array_mut) {
                for endpoint in endpoints {
                    if let Some(old_id) = endpoint.get("node_id").and_then(Value::as_str)
                        && let Some(new_id) = node_ids.get(old_id)
                    {
                        endpoint["node_id"] = json!(new_id);
                    }
                }
            }
            strip_link_runtime_fields(link);
        }
    }
    if let Some(drawings) = project.get_mut("drawings").and_then(Value::as_array_mut) {
        for drawing in drawings {
            drawing["drawing_id"] = json!(Uuid::new_v4());
            drawing["project_id"] = json!(project_id);
        }
    }
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
    let nodes = if project.get("topology").is_some() {
        project
            .get_mut("topology")
            .and_then(|topology| topology.get_mut("nodes"))
    } else {
        project.get_mut("nodes")
    };
    if let Some(nodes) = nodes.and_then(Value::as_array_mut) {
        for node in nodes {
            strip_node_runtime_fields(node);
            if node.get("status").is_some() {
                node["status"] = json!("stopped");
            }
        }
    }
    let links = if project.get("topology").is_some() {
        project
            .get_mut("topology")
            .and_then(|topology| topology.get_mut("links"))
    } else {
        project.get_mut("links")
    };
    if let Some(links) = links.and_then(Value::as_array_mut) {
        for link in links {
            strip_link_runtime_fields(link);
        }
    }
}

#[cfg(test)]
#[path = "tests/archive_tests.rs"]
mod archive_tests;

#[cfg(test)]
#[path = "tests/route_tests.rs"]
mod route_tests;
