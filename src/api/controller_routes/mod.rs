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

mod templates;
pub(crate) use templates::check_version;
use templates::*;
mod statistics;
use statistics::*;
mod project_operations;
use project_operations::*;
mod project_archive;
use project_archive::*;
mod snapshots;
use snapshots::*;
mod bulk_lifecycle;
pub(crate) use bulk_lifecycle::bulk_node_action_applies;
pub(crate) use bulk_lifecycle::bulk_nodes;
use bulk_lifecycle::*;
mod node_resources;
use node_resources::*;
mod images;
pub(crate) use images::is_qemu_image_upload;
use images::*;
mod symbols;
use symbols::*;
mod archive;
use archive::*;

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
        .route("/v3/computes", get(list_computes).post(create_compute))
        .route(
            "/v3/computes/{compute_id}",
            get(get_compute).put(update_compute).delete(delete_compute),
        )
        .route(
            "/v3/computes/{compute_id}/disconnect",
            post(disconnect_compute),
        )
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
        .route(
            "/v3/projects/{project_id}/symbols",
            post(upload_project_symbol),
        )
        .route("/v3/symbols/default_symbols", get(default_symbols))
        .route(
            "/v3/symbols/{symbol_id}/raw",
            get(get_symbol).post(upload_symbol),
        )
        .route("/v3/symbols/{symbol_id}/dimensions", get(symbol_dimensions))
        .route("/v3/symbols/{symbol_id}", delete(delete_symbol))
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
    let mut settings = decode_json(json)?;
    if settings.get("WebWireshark").is_none() {
        settings["WebWireshark"] = json!({"enabled":true,"image":"gns3/web-wireshark:latest","network_subnet":"172.31.0.0/22","memory":"2g","cpus":1.0,"pids_limit":1000});
    }
    Ok(Json(settings))
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

async fn local_compute(state: &AppState) -> Result<Value, ApiError> {
    let stored: String =
        sqlx::query_scalar("SELECT settings_json FROM controller_settings WHERE singleton = 1")
            .fetch_one(state.store.pool())
            .await?;
    let settings = decode_json(stored)?;
    Ok(json!({
        "compute_id": "local",
        "name": "Local",
        "host": state.bind_address.ip().to_string(),
        "port": state.bind_address.port(),
        "protocol": "http",
        "auto_connect": settings.pointer("/computes/local/auto_connect").and_then(Value::as_bool).unwrap_or(true),
        "connected": settings.pointer("/computes/local/connected").and_then(Value::as_bool).unwrap_or(true)
    }))
}
async fn list_computes(State(state): State<AppState>) -> Result<Json<Vec<Value>>, ApiError> {
    let mut computes = vec![local_compute(&state).await?];
    let stored: String =
        sqlx::query_scalar("SELECT settings_json FROM controller_settings WHERE singleton = 1")
            .fetch_one(state.store.pool())
            .await?;
    if let Some(items) = decode_json(stored)?
        .get("computes")
        .and_then(Value::as_object)
    {
        computes.extend(
            items
                .iter()
                .filter(|(id, _)| id.as_str() != "local")
                .map(|(_, value)| value.clone()),
        );
    }
    Ok(Json(computes))
}

fn validate_compute(body: &Value) -> Result<(), ApiError> {
    let host = body
        .get("host")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if host.is_empty() {
        return Err(ApiError::bad_request("compute host must not be empty"));
    }
    let port = body.get("port").and_then(Value::as_u64).unwrap_or(0);
    if !(1..=65535).contains(&port) {
        return Err(ApiError::bad_request(
            "compute port must be between 1 and 65535",
        ));
    }
    for key in ["name", "protocol", "compute_id"] {
        if body
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .is_empty()
        {
            return Err(ApiError::bad_request(format!(
                "compute {key} must not be empty"
            )));
        }
    }
    Ok(())
}
async fn mutate_remote_compute(
    state: AppState,
    id: String,
    body: Value,
    create: bool,
) -> Result<Json<Value>, ApiError> {
    validate_compute(&body)?;
    if id == "local" {
        return Err(ApiError::bad_request(
            "the local compute cannot be modified",
        ));
    }
    let row: String =
        sqlx::query_scalar("SELECT settings_json FROM controller_settings WHERE singleton = 1")
            .fetch_one(state.store.pool())
            .await?;
    let mut settings = decode_json(row)?;
    let computes = settings
        .as_object_mut()
        .ok_or_else(|| ApiError::bad_request("settings must be an object"))?
        .entry("computes")
        .or_insert(json!({}))
        .as_object_mut()
        .ok_or_else(|| ApiError::bad_request("computes settings must be an object"))?;
    if create && computes.contains_key(&id) {
        return Err(ApiError::bad_request("compute already exists"));
    }
    let mut compute = body;
    compute["compute_id"] = json!(id);
    compute["connected"] = json!(false);
    computes.insert(id, compute.clone());
    sqlx::query("UPDATE controller_settings SET settings_json = ? WHERE singleton = 1")
        .bind(serde_json::to_string(&settings).map_err(internal_json_error)?)
        .execute(state.store.pool())
        .await?;
    Ok(Json(compute))
}
async fn create_compute(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let id = body
        .get("compute_id")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    mutate_remote_compute(state, id, body, true).await
}
async fn update_compute(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    if id == "local" {
        return Err(ApiError::bad_request(
            "the local compute cannot be modified",
        ));
    }
    if !decode_json(
        sqlx::query_scalar::<_, String>(
            "SELECT settings_json FROM controller_settings WHERE singleton = 1",
        )
        .fetch_one(state.store.pool())
        .await?,
    )?
    .pointer(&format!("/computes/{id}"))
    .is_some()
    {
        return Err(ApiError::not_found("compute does not exist"));
    }
    mutate_remote_compute(state, id, body, false).await
}
async fn delete_compute(
    State(state): State<AppState>,
    AxumPath(id): AxumPath<String>,
) -> Result<StatusCode, ApiError> {
    if id == "local" {
        return Err(ApiError::bad_request("the local compute cannot be deleted"));
    }
    let row: String =
        sqlx::query_scalar("SELECT settings_json FROM controller_settings WHERE singleton = 1")
            .fetch_one(state.store.pool())
            .await?;
    let mut settings = decode_json(row)?;
    let computes = settings["computes"]
        .as_object_mut()
        .ok_or_else(|| ApiError::not_found("compute does not exist"))?;
    if computes.remove(&id).is_none() {
        return Err(ApiError::not_found("compute does not exist"));
    }
    sqlx::query("UPDATE controller_settings SET settings_json = ? WHERE singleton = 1")
        .bind(serde_json::to_string(&settings).map_err(internal_json_error)?)
        .execute(state.store.pool())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn get_compute(
    State(state): State<AppState>,
    AxumPath(compute_id): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    if compute_id == "local" {
        Ok(Json(local_compute(&state).await?))
    } else {
        Err(ApiError::not_found(format!(
            "Compute {compute_id} does not exist"
        )))
    }
}

async fn disconnect_compute(
    State(state): State<AppState>,
    AxumPath(compute_id): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    if compute_id != "local" {
        return Err(ApiError::not_found(format!(
            "Compute {compute_id} does not exist"
        )));
    }
    let row: String =
        sqlx::query_scalar("SELECT settings_json FROM controller_settings WHERE singleton = 1")
            .fetch_one(state.store.pool())
            .await?;
    let mut settings = decode_json(row)?;
    let obj = as_object_mut(&mut settings, "stored settings must be an object")?;
    let computes = obj.entry("computes").or_insert(json!({}));
    let local = computes
        .as_object_mut()
        .ok_or_else(|| ApiError::bad_request("computes settings must be an object"))?
        .entry("local")
        .or_insert(json!({}));
    let local = as_object_mut(local, "local compute settings must be an object")?;
    local.insert("connected".into(), json!(false));
    local.entry("auto_connect").or_insert(json!(true));
    sqlx::query("UPDATE controller_settings SET settings_json = ? WHERE singleton = 1")
        .bind(serde_json::to_string(&settings).map_err(internal_json_error)?)
        .execute(state.store.pool())
        .await?;
    Ok(Json(local_compute(&state).await?))
}

#[cfg(test)]
#[path = "tests/archive_tests.rs"]
mod archive_tests;

#[cfg(test)]
#[path = "tests/project_routes.rs"]
mod project_routes;
#[cfg(test)]
#[path = "tests/resource_routes.rs"]
mod resource_routes;
#[cfg(test)]
#[path = "tests/runtime_and_notifications.rs"]
mod runtime_and_notifications;
#[cfg(test)]
#[path = "tests/stats_and_imports.rs"]
mod stats_and_imports;
#[cfg(test)]
#[path = "tests/templates_and_version.rs"]
mod templates_and_version;
