use std::collections::HashSet;

use axum::{
    Json, Router,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{any, get, post},
};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::{Map, Value, json};
use sqlx::{Row, SqlitePool};
use tokio::sync::broadcast;
use tower_http::{
    limit::RequestBodyLimitLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use uuid::Uuid;

use crate::{
    qemu::{QemuLink, QemuRuntime},
    runtime::{DockerRuntime, NodeAction, RuntimeError, parse_start_command},
    storage::{ProjectEvent, Store},
};

const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const EVENT_PAGE_SIZE: i64 = 512;

#[derive(Clone)]
pub struct AppState {
    store: Store,
    runtime: DockerRuntime,
    qemu: QemuRuntime,
}

impl AppState {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            runtime: DockerRuntime::new(),
            qemu: QemuRuntime::new(),
        }
    }

    #[cfg(test)]
    fn with_runtimes(store: Store, runtime: DockerRuntime, qemu: QemuRuntime) -> Self {
        Self {
            store,
            runtime,
            qemu,
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/v3/version", get(version))
        .route("/v3/runtimes", get(runtimes))
        .route("/v3/templates", get(list_templates))
        .route("/v3/projects", get(list_projects).post(create_project))
        .route(
            "/v3/projects/{project_id}",
            get(get_project).put(update_project).delete(delete_project),
        )
        .route("/v3/projects/{project_id}/open", post(open_project))
        .route("/v3/projects/{project_id}/close", post(close_project))
        .route(
            "/v3/projects/{project_id}/nodes",
            get(list_nodes).post(create_node),
        )
        .route(
            "/v3/projects/{project_id}/templates/{template_id}",
            post(create_node_from_template),
        )
        .route(
            "/v3/projects/{project_id}/nodes/{node_id}",
            get(get_node).put(update_node).delete(delete_node),
        )
        .route(
            "/v3/projects/{project_id}/nodes/{node_id}/start",
            post(start_node),
        )
        .route(
            "/v3/projects/{project_id}/nodes/{node_id}/stop",
            post(stop_node),
        )
        .route(
            "/v3/projects/{project_id}/nodes/{node_id}/suspend",
            post(suspend_node),
        )
        .route(
            "/v3/projects/{project_id}/nodes/{node_id}/reload",
            post(reload_node),
        )
        .route(
            "/v3/projects/{project_id}/links",
            get(list_links).post(create_link),
        )
        .route(
            "/v3/projects/{project_id}/links/{link_id}",
            get(get_link).put(update_link).delete(delete_link),
        )
        .route(
            "/v3/projects/{project_id}/drawings",
            get(list_drawings).post(create_drawing),
        )
        .route(
            "/v3/projects/{project_id}/drawings/{drawing_id}",
            get(get_drawing).put(update_drawing).delete(delete_drawing),
        )
        .route(
            "/v3/projects/{project_id}/topology",
            get(get_topology_snapshot),
        )
        .route(
            "/v3/projects/{project_id}/notifications/ws",
            get(project_notifications),
        )
        .route("/v3", any(|| async { StatusCode::NOT_FOUND }))
        .route("/v3/{*path}", any(|| async { StatusCode::NOT_FOUND }))
        .route_service("/", ServeFile::new("frontend/dist/client/index.html"))
        .route_service(
            "/{*path}",
            ServeFile::new("frontend/dist/client/index.html"),
        )
        .nest_service("/assets", ServeDir::new("frontend/dist/client/assets"))
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(state.store.pool())
        .await?;
    Ok(Json(json!({ "status": "ok" })))
}

#[derive(Serialize)]
struct ErrorMessage {
    message: String,
}

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, message)
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }

    fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorMessage {
                message: self.message,
            }),
        )
            .into_response()
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(error: sqlx::Error) -> Self {
        if let sqlx::Error::Database(database_error) = &error {
            if database_error.is_unique_violation() {
                return Self::conflict(database_error.message());
            }
        }
        tracing::error!(%error, "database operation failed");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Database operation failed",
        )
    }
}

async fn version() -> Json<Value> {
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "local": true
    }))
}

async fn runtimes(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "docker": state.runtime.info().await,
        "qemu": state.qemu.info().await
    }))
}

async fn list_templates(State(state): State<AppState>) -> Result<Json<Vec<Value>>, ApiError> {
    let mut templates = Vec::new();
    match state.runtime.templates().await {
        Ok(available) => templates.extend(available),
        Err(error) => tracing::debug!(error = %error.message(), "Docker templates are unavailable"),
    }
    match state.qemu.templates().await {
        Ok(available) => templates.extend(available),
        Err(error) => tracing::debug!(error = %error.message(), "QEMU templates are unavailable"),
    }
    templates.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
    Ok(Json(templates))
}

fn runtime_api_error(error: RuntimeError) -> ApiError {
    let status = match &error {
        RuntimeError::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        RuntimeError::NotFound(_) => StatusCode::NOT_FOUND,
        RuntimeError::Failed(_) => StatusCode::CONFLICT,
    };
    ApiError::new(status, error.message())
}

async fn list_projects(State(state): State<AppState>) -> Result<Json<Vec<Value>>, ApiError> {
    let rows = sqlx::query("SELECT project_json FROM projects ORDER BY name, project_id")
        .fetch_all(state.store.pool())
        .await?;
    let projects = rows
        .into_iter()
        .map(|row| decode_json(row.get::<String, _>("project_json")))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(projects))
}

async fn create_project(
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
    let mut tx = state.store.pool().begin().await?;
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

async fn get_project(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        fetch_project(state.store.pool(), &project_id.to_string()).await?,
    ))
}

async fn update_project(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Json(patch): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let project_id = project_id.to_string();
    let mut tx = state.store.pool().begin().await?;
    let row = sqlx::query("SELECT project_json FROM projects WHERE project_id = ?")
        .bind(&project_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Project {project_id} does not exist")))?;
    let mut project = decode_json(row.get::<String, _>("project_json"))?;
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
    Ok(Json(project))
}

async fn delete_project(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let project_id = project_id.to_string();
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
    let mut tx = state.store.pool().begin().await?;
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
    state.store.publish(event);
    Ok(StatusCode::NO_CONTENT)
}

async fn open_project(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    set_project_status(state, project_id, "opened").await
}

async fn close_project(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    set_project_status(state, project_id, "closed").await
}

async fn set_project_status(
    state: AppState,
    project_id: Uuid,
    status: &str,
) -> Result<Json<Value>, ApiError> {
    let project_id = project_id.to_string();
    let mut tx = state.store.pool().begin().await?;
    let row = sqlx::query("SELECT project_json FROM projects WHERE project_id = ?")
        .bind(&project_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Project {project_id} does not exist")))?;
    let mut project = decode_json(row.get::<String, _>("project_json"))?;
    let mut node_events = Vec::new();
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

async fn list_nodes(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Vec<Value>>, ApiError> {
    let project_id = project_id.to_string();
    let nodes = list_objects(state.store.pool(), &project_id, "node")
        .await?
        .0;
    Ok(Json(
        refresh_node_statuses(&state, &project_id, nodes).await,
    ))
}

async fn list_links(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Vec<Value>>, ApiError> {
    list_objects(state.store.pool(), &project_id.to_string(), "link").await
}

async fn list_drawings(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Vec<Value>>, ApiError> {
    list_objects(state.store.pool(), &project_id.to_string(), "drawing").await
}

async fn create_node(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Json(node): Json<Value>,
) -> Result<impl IntoResponse, ApiError> {
    match node.get("node_type").and_then(Value::as_str) {
        Some("docker") => create_docker_node(state, project_id.to_string(), node).await,
        Some("qemu") => create_qemu_node(state, project_id.to_string(), node).await,
        Some(node_type) => Err(ApiError::bad_request(format!(
            "Unsupported node type '{node_type}'. Executable node types are Docker and QEMU."
        ))),
        None => Err(ApiError::bad_request("node_type is required")),
    }
}

async fn create_node_from_template(
    State(state): State<AppState>,
    Path((project_id, template_id)): Path<(Uuid, Uuid)>,
    Json(usage): Json<Value>,
) -> Result<impl IntoResponse, ApiError> {
    let mut templates = Vec::new();
    if let Ok(docker_templates) = state.runtime.templates().await {
        templates.extend(docker_templates);
    }
    if let Ok(qemu_templates) = state.qemu.templates().await {
        templates.extend(qemu_templates);
    }
    let template = templates
        .into_iter()
        .find(|template| {
            template.get("template_id").and_then(Value::as_str) == Some(&template_id.to_string())
        })
        .ok_or_else(|| ApiError::not_found(format!("Template {template_id} does not exist")))?;
    let name = usage
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .or_else(|| template.get("name").and_then(Value::as_str))
        .ok_or_else(|| ApiError::bad_request("template has no name"))?;
    let node_type = template
        .get("template_type")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad_request("template has no runtime type"))?;
    let mut node = if node_type == "docker" {
        json!({
            "name": name,
            "node_type": "docker",
            "image": template["image"],
            "image_digest": template["image_digest"],
            "template_id": template_id,
            "adapters": template["adapters"],
            "memory": template["memory"],
            "cpus": template["cpus"],
            "x": usage.get("x").cloned().unwrap_or(json!(0)),
            "y": usage.get("y").cloned().unwrap_or(json!(0))
        })
    } else if node_type == "qemu" {
        json!({
            "name": name,
            "node_type": "qemu",
            "hda_disk_image": template["hda_disk_image"],
            "template_id": template_id,
            "adapters": template["adapters"],
            "memory": template["memory"],
            "cpus": template["cpus"],
            "x": usage.get("x").cloned().unwrap_or(json!(0)),
            "y": usage.get("y").cloned().unwrap_or(json!(0))
        })
    } else {
        return Err(ApiError::bad_request(format!(
            "Template runtime '{node_type}' is not supported"
        )));
    };
    if node_type == "docker"
        && let Some(start_command) = usage
            .get("start_command")
            .and_then(Value::as_str)
            .or_else(|| template.get("start_command").and_then(Value::as_str))
    {
        node["start_command"] = json!(start_command);
    }
    if let Some(compute_id) = usage.get("compute_id") {
        node["compute_id"] = compute_id.clone();
    }
    match node_type {
        "docker" => create_docker_node(state, project_id.to_string(), node).await,
        "qemu" => create_qemu_node(state, project_id.to_string(), node).await,
        _ => unreachable!("template runtime was validated"),
    }
}

async fn create_docker_node(
    state: AppState,
    project_id: String,
    mut node: Value,
) -> Result<Response, ApiError> {
    let object = as_object_mut(&mut node, "node body must be an object")?;
    let name = required_string(object, "name")?.to_owned();
    let node_type = required_string(object, "node_type")?;
    if node_type != "docker" {
        return Err(ApiError::bad_request(format!(
            "Unsupported Docker node type '{node_type}'"
        )));
    }
    let image = required_string(object, "image")?.to_owned();
    let start_command = object
        .get("start_command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .map(str::to_owned);
    if let Some(start_command) = start_command.as_deref() {
        parse_start_command(start_command)
            .map_err(|error| ApiError::bad_request(error.message()))?;
    }
    if object
        .get("compute_id")
        .and_then(Value::as_str)
        .is_some_and(|compute_id| compute_id != "local")
    {
        return Err(ApiError::bad_request(
            "This runtime build can execute nodes only on the local Docker compute",
        ));
    }
    let node_id = parse_or_generate_uuid(object.get("node_id"), "node_id")?;
    ensure_project_exists_pool(state.store.pool(), &project_id).await?;
    if let Some(row) = sqlx::query(
        "SELECT object_json FROM topology_objects
         WHERE project_id = ? AND kind = 'node' AND object_id = ?",
    )
    .bind(&project_id)
    .bind(&node_id)
    .fetch_optional(state.store.pool())
    .await?
    {
        return Ok(Json(decode_json(row.get::<String, _>("object_json"))?).into_response());
    }

    object.insert("name".into(), json!(name));
    object.insert("node_id".into(), json!(node_id));
    object.insert("project_id".into(), json!(project_id));
    object.insert("node_type".into(), json!("docker"));
    object.insert("image".into(), json!(image));
    set_default(object, "compute_id", json!("local"));
    set_default(object, "status", json!("stopped"));
    set_default(object, "x", json!(0));
    set_default(object, "y", json!(0));
    set_default(object, "z", json!(1));
    set_default(object, "locked", json!(false));
    set_default(object, "adapters", json!(8));
    set_default(object, "memory", json!(0));
    set_default(object, "cpus", json!(0));
    set_default(object, "properties", json!({}));

    let memory_mb = object
        .get("memory")
        .map(|value| {
            value
                .as_i64()
                .ok_or_else(|| ApiError::bad_request("memory must be an integer number of MB"))
        })
        .transpose()?
        .unwrap_or(0);
    if !(0..=1_048_576).contains(&memory_mb) {
        return Err(ApiError::bad_request(
            "memory must be between 0 and 1048576 MB",
        ));
    }
    let cpus = object
        .get("cpus")
        .map(|value| {
            value
                .as_f64()
                .ok_or_else(|| ApiError::bad_request("cpus must be a number"))
        })
        .transpose()?
        .unwrap_or(0.0);
    if !(0.0..=256.0).contains(&cpus) {
        return Err(ApiError::bad_request("cpus must be between 0 and 256"));
    }
    let adapters = object
        .get("adapters")
        .map(|value| {
            value
                .as_i64()
                .ok_or_else(|| ApiError::bad_request("adapters must be an integer"))
        })
        .transpose()?
        .unwrap_or(8);
    if !(1..=99).contains(&adapters) {
        return Err(ApiError::bad_request("adapters must be between 1 and 99"));
    }
    let (container_id, image_digest) = state
        .runtime
        .create_container(
            &project_id,
            &node_id,
            &image,
            &name,
            start_command.as_deref(),
            u64::try_from(memory_mb).ok(),
            Some(cpus),
        )
        .await
        .map_err(runtime_api_error)?;
    object.insert("container_id".into(), json!(container_id));
    object.insert("image_digest".into(), json!(image_digest));

    match create_topology_object(state.clone(), &project_id, "node", "node_id", node).await {
        Ok(response) => Ok(response),
        Err(error) => {
            if let Err(cleanup_error) = state
                .runtime
                .remove_node_resources(&node_id, Some(&container_id))
                .await
            {
                tracing::error!(error = %cleanup_error.message(), %container_id, "could not roll back created container");
            }
            Err(error)
        }
    }
}

async fn create_qemu_node(
    state: AppState,
    project_id: String,
    mut node: Value,
) -> Result<Response, ApiError> {
    let object = as_object_mut(&mut node, "node body must be an object")?;
    let name = required_string(object, "name")?.to_owned();
    let node_type = required_string(object, "node_type")?;
    if node_type != "qemu" {
        return Err(ApiError::bad_request(format!(
            "Unsupported QEMU node type '{node_type}'"
        )));
    }
    let image = required_string(object, "hda_disk_image")?.to_owned();
    if object
        .get("compute_id")
        .and_then(Value::as_str)
        .is_some_and(|compute_id| compute_id != "local")
    {
        return Err(ApiError::bad_request(
            "This runtime build can execute nodes only on the local QEMU compute",
        ));
    }
    let node_id = parse_or_generate_uuid(object.get("node_id"), "node_id")?;
    ensure_project_exists_pool(state.store.pool(), &project_id).await?;
    if let Some(row) = sqlx::query(
        "SELECT object_json FROM topology_objects
         WHERE project_id = ? AND kind = 'node' AND object_id = ?",
    )
    .bind(&project_id)
    .bind(&node_id)
    .fetch_optional(state.store.pool())
    .await?
    {
        return Ok(Json(decode_json(row.get::<String, _>("object_json"))?).into_response());
    }

    object.insert("name".into(), json!(name));
    object.insert("node_id".into(), json!(node_id));
    object.insert("project_id".into(), json!(project_id));
    object.insert("node_type".into(), json!("qemu"));
    object.insert("hda_disk_image".into(), json!(image));
    set_default(object, "compute_id", json!("local"));
    set_default(object, "status", json!("stopped"));
    set_default(object, "x", json!(0));
    set_default(object, "y", json!(0));
    set_default(object, "z", json!(1));
    set_default(object, "locked", json!(false));
    set_default(object, "adapters", json!(8));
    set_default(object, "memory", json!(512));
    set_default(object, "cpus", json!(1));
    set_default(object, "properties", json!({}));

    let memory_mb = object
        .get("memory")
        .map(|value| {
            value
                .as_i64()
                .ok_or_else(|| ApiError::bad_request("memory must be an integer number of MB"))
        })
        .transpose()?
        .unwrap_or(512);
    if !(16..=1_048_576).contains(&memory_mb) {
        return Err(ApiError::bad_request(
            "QEMU memory must be between 16 and 1048576 MB",
        ));
    }
    let cpus = object
        .get("cpus")
        .map(|value| {
            value
                .as_i64()
                .ok_or_else(|| ApiError::bad_request("QEMU cpus must be an integer"))
        })
        .transpose()?
        .unwrap_or(1);
    if !(1..=256).contains(&cpus) {
        return Err(ApiError::bad_request("QEMU cpus must be between 1 and 256"));
    }
    let adapters = object
        .get("adapters")
        .map(|value| {
            value
                .as_i64()
                .ok_or_else(|| ApiError::bad_request("adapters must be an integer"))
        })
        .transpose()?
        .unwrap_or(8);
    if !(1..=99).contains(&adapters) {
        return Err(ApiError::bad_request("adapters must be between 1 and 99"));
    }
    let vm = state
        .qemu
        .create_vm(
            &project_id,
            &node_id,
            &image,
            memory_mb as u64,
            cpus as u32,
            adapters as u32,
        )
        .await
        .map_err(runtime_api_error)?;
    for (key, value) in state
        .qemu
        .runtime_fields(&vm)
        .as_object()
        .expect("runtime fields are an object")
    {
        object.insert(key.clone(), value.clone());
    }

    match create_topology_object(state.clone(), &project_id, "node", "node_id", node).await {
        Ok(response) => Ok(response),
        Err(error) => {
            if let Err(cleanup_error) = state.qemu.remove_vm(&vm).await {
                tracing::error!(error = %cleanup_error.message(), node_id, "could not roll back created QEMU disk overlay");
            }
            Err(error)
        }
    }
}

async fn create_link(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Json(mut link): Json<Value>,
) -> Result<impl IntoResponse, ApiError> {
    let project_id = project_id.to_string();
    let object = as_object_mut(&mut link, "link body must be an object")?;
    let link_id = parse_or_generate_uuid(object.get("link_id"), "link_id")?;
    let endpoints = object
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::bad_request("link must contain a nodes array"))?;
    if endpoints.len() < 2 {
        return Err(ApiError::bad_request(
            "link must connect at least two endpoints",
        ));
    }
    validate_link_shape(endpoints)?;
    let link_type = object
        .get("link_type")
        .and_then(Value::as_str)
        .unwrap_or("ethernet");
    if link_type != "ethernet" {
        return Err(ApiError::bad_request(
            "Only Ethernet links are supported by the active runtime",
        ));
    }
    object.insert("link_id".into(), json!(link_id));
    object.insert("project_id".into(), json!(project_id));
    set_default(object, "link_type", json!("ethernet"));
    set_default(object, "suspend", json!(false));
    if let Some(row) = sqlx::query(
        "SELECT object_json FROM topology_objects
         WHERE project_id = ? AND kind = 'link' AND object_id = ?",
    )
    .bind(&project_id)
    .bind(&link_id)
    .fetch_optional(state.store.pool())
    .await?
    {
        return Ok(Json(decode_json(row.get::<String, _>("object_json"))?).into_response());
    }
    let mut validation = state.store.pool().begin().await?;
    ensure_project_exists(&mut validation, &project_id).await?;
    validate_link_in_tx(&mut validation, &project_id, &link).await?;
    validation.rollback().await?;
    let runtime_endpoints = link_runtime_endpoints(state.store.pool(), &project_id, &link).await?;
    let runtime_type = runtime_endpoints
        .first()
        .and_then(|endpoint| endpoint["node_type"].as_str())
        .unwrap_or_default();
    if runtime_endpoints
        .iter()
        .any(|endpoint| endpoint["node_type"].as_str() != Some(runtime_type))
    {
        return Err(ApiError::bad_request(
            "Ethernet links cannot mix Docker and QEMU nodes",
        ));
    }
    if runtime_type == "docker" {
        state
            .runtime
            .create_link(&project_id, &link_id, &runtime_endpoints)
            .await
            .map_err(runtime_api_error)?;
    } else if runtime_type == "qemu" {
        if runtime_endpoints.len() != 2 {
            return Err(ApiError::bad_request(
                "QEMU Ethernet links connect exactly two virtual machines",
            ));
        }
        for endpoint in &runtime_endpoints {
            let row = sqlx::query(
                "SELECT object_json FROM topology_objects
                 WHERE project_id = ? AND kind = 'node' AND object_id = ?",
            )
            .bind(&project_id)
            .bind(endpoint["node_id"].as_str().unwrap_or_default())
            .fetch_one(state.store.pool())
            .await?;
            let node = decode_json(row.get::<String, _>("object_json"))?;
            let vm = state.qemu.vm_from_node(&node).map_err(runtime_api_error)?;
            let status = state.qemu.status(&vm).await.map_err(runtime_api_error)?;
            if status != "stopped" {
                return Err(ApiError::conflict(
                    "Stop both QEMU nodes before changing their Ethernet links",
                ));
            }
        }
        let (port_a, port_b) = state
            .qemu
            .allocate_link_ports()
            .map_err(runtime_api_error)?;
        let endpoints = link["nodes"]
            .as_array_mut()
            .ok_or_else(|| ApiError::bad_request("link must contain a nodes array"))?;
        for (index, endpoint) in endpoints.iter_mut().enumerate() {
            let (local_port, remote_port) = if index == 0 {
                (port_a, port_b)
            } else {
                (port_b, port_a)
            };
            let endpoint_object = endpoint
                .as_object_mut()
                .ok_or_else(|| ApiError::bad_request("each link endpoint must be an object"))?;
            endpoint_object.insert("runtime_local_port".into(), json!(local_port));
            endpoint_object.insert("runtime_remote_port".into(), json!(remote_port));
        }
    } else {
        return Err(ApiError::bad_request(format!(
            "Ethernet links are not available for node type '{runtime_type}'"
        )));
    }

    match create_topology_object(state.clone(), &project_id, "link", "link_id", link).await {
        Ok(response) => Ok(response),
        Err(error) => {
            if runtime_type == "docker"
                && let Err(cleanup_error) = state
                    .runtime
                    .remove_link(&link_id, &runtime_endpoints)
                    .await
            {
                tracing::error!(error = %cleanup_error.message(), %link_id, "could not roll back created Docker network");
            }
            Err(error)
        }
    }
}

async fn link_runtime_endpoints(
    pool: &SqlitePool,
    project_id: &str,
    link: &Value,
) -> Result<Vec<Value>, ApiError> {
    let endpoints = link
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::bad_request("link must contain a nodes array"))?;
    let mut runtime_endpoints = Vec::with_capacity(endpoints.len());
    for endpoint in endpoints {
        let node_id = endpoint
            .get("node_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::bad_request("each link endpoint requires node_id"))?;
        let row = sqlx::query(
            "SELECT object_json FROM topology_objects
             WHERE project_id = ? AND kind = 'node' AND object_id = ?",
        )
        .bind(project_id)
        .bind(node_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| {
            ApiError::bad_request(format!("Node {node_id} does not exist in this project"))
        })?;
        let node = decode_json(row.get::<String, _>("object_json"))?;
        let node_type = node
            .get("node_type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let adapter = endpoint["adapter_number"].as_u64().unwrap_or_default();
        let port = endpoint["port_number"].as_u64().unwrap_or_default();
        let adapters = node.get("adapters").and_then(Value::as_u64).unwrap_or(8);
        if node_type != "docker" && node_type != "qemu" {
            return Err(ApiError::bad_request(format!(
                "Node type '{node_type}' has no Ethernet runtime"
            )));
        }
        if (node_type == "docker" && (adapter > 0 || port >= adapters))
            || (node_type == "qemu" && (adapter >= adapters || port != 0))
        {
            return Err(ApiError::bad_request(format!(
                "Port {adapter}/{port} is outside the configured {adapters} {node_type} adapters"
            )));
        }
        let mut runtime_endpoint = json!({
            "node_id": node_id,
            "node_type": node_type,
            "adapter_number": adapter,
            "port_number": port
        });
        if node_type == "docker" {
            runtime_endpoint["container_id"] =
                node.get("container_id").cloned().ok_or_else(|| {
                    ApiError::conflict(format!("Node {node_id} has no Docker container"))
                })?;
        }
        runtime_endpoints.push(runtime_endpoint);
    }
    Ok(runtime_endpoints)
}

async fn apply_node_lifecycle(
    state: &AppState,
    project_id: &str,
    node: &mut Value,
    action: NodeAction,
) -> Result<String, ApiError> {
    let node_type = node
        .get("node_type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match node_type {
        "docker" => {
            let container_id = node
                .get("container_id")
                .and_then(Value::as_str)
                .ok_or_else(|| ApiError::conflict("Docker node has no runtime container"))?;
            state
                .runtime
                .lifecycle(container_id, action)
                .await
                .map_err(runtime_api_error)
        }
        "qemu" => {
            let vm = state.qemu.vm_from_node(node).map_err(runtime_api_error)?;
            let links = if matches!(action, NodeAction::Start) {
                qemu_links_for_node(state.store.pool(), project_id, &vm.node_id).await?
            } else {
                Vec::new()
            };
            state
                .qemu
                .lifecycle(&vm, action, &links)
                .await
                .map_err(runtime_api_error)
        }
        _ => Err(ApiError::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "This node has no implemented runtime adapter",
        )),
    }
}

async fn stop_runtime_node(
    state: &AppState,
    project_id: &str,
    node: &Value,
) -> Result<(), ApiError> {
    let Some(node_type) = node.get("node_type").and_then(Value::as_str) else {
        return Ok(());
    };
    if node_type != "docker" && node_type != "qemu" {
        return Ok(());
    }
    let mut node = node.clone();
    apply_node_lifecycle(state, project_id, &mut node, NodeAction::Stop).await?;
    Ok(())
}

async fn cleanup_node_runtime(state: &AppState, node: &Value) -> Result<(), ApiError> {
    let node_type = node
        .get("node_type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let node_id = node
        .get("node_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match node_type {
        "docker" => state
            .runtime
            .remove_node_resources(node_id, node.get("container_id").and_then(Value::as_str))
            .await
            .map_err(runtime_api_error),
        "qemu" => {
            let vm = state.qemu.vm_from_node(node).map_err(runtime_api_error)?;
            state.qemu.remove_vm(&vm).await.map_err(runtime_api_error)
        }
        _ => Ok(()),
    }
}

async fn qemu_links_for_node(
    pool: &SqlitePool,
    project_id: &str,
    node_id: &str,
) -> Result<Vec<QemuLink>, ApiError> {
    let rows = sqlx::query(
        "SELECT DISTINCT o.object_id, o.object_json
         FROM topology_objects AS o
         JOIN link_endpoints AS e
           ON e.project_id = o.project_id AND e.link_id = o.object_id
         WHERE o.project_id = ? AND o.kind = 'link' AND e.node_id = ?",
    )
    .bind(project_id)
    .bind(node_id)
    .fetch_all(pool)
    .await?;
    let mut links = Vec::new();
    for row in rows {
        let link_id: String = row.get("object_id");
        let link = decode_json(row.get::<String, _>("object_json"))?;
        let Some(endpoints) = link.get("nodes").and_then(Value::as_array) else {
            continue;
        };
        if endpoints.len() != 2 {
            continue;
        }
        let Some(endpoint) = endpoints
            .iter()
            .find(|endpoint| endpoint["node_id"].as_str() == Some(node_id))
        else {
            continue;
        };
        let local_port = endpoint["runtime_local_port"]
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .ok_or_else(|| ApiError::conflict(format!("QEMU link {link_id} has no local port")))?;
        let remote_port = endpoint["runtime_remote_port"]
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .ok_or_else(|| ApiError::conflict(format!("QEMU link {link_id} has no peer port")))?;
        let adapter_number = endpoint["adapter_number"]
            .as_u64()
            .and_then(|adapter| u32::try_from(adapter).ok())
            .ok_or_else(|| {
                ApiError::conflict(format!("QEMU link {link_id} has an invalid adapter"))
            })?;
        links.push(QemuLink {
            link_id,
            adapter_number,
            local_port,
            remote_port,
        });
    }
    Ok(links)
}

async fn ensure_link_qemu_nodes_stopped(
    state: &AppState,
    project_id: &str,
    link: &Value,
) -> Result<(), ApiError> {
    let Some(endpoints) = link.get("nodes").and_then(Value::as_array) else {
        return Ok(());
    };
    for endpoint in endpoints {
        let Some(node_id) = endpoint.get("node_id").and_then(Value::as_str) else {
            continue;
        };
        let row = sqlx::query(
            "SELECT object_json FROM topology_objects
             WHERE project_id = ? AND kind = 'node' AND object_id = ?",
        )
        .bind(project_id)
        .bind(node_id)
        .fetch_optional(state.store.pool())
        .await?;
        let Some(row) = row else { continue };
        let node = decode_json(row.get::<String, _>("object_json"))?;
        if node.get("node_type").and_then(Value::as_str) == Some("qemu") {
            let vm = state.qemu.vm_from_node(&node).map_err(runtime_api_error)?;
            if state.qemu.status(&vm).await.map_err(runtime_api_error)? != "stopped" {
                return Err(ApiError::conflict(
                    "Stop both QEMU nodes before changing their Ethernet links",
                ));
            }
        }
    }
    Ok(())
}

async fn link_cleanup_endpoints(
    pool: &SqlitePool,
    project_id: &str,
    link: &Value,
) -> Result<Vec<Value>, ApiError> {
    let Some(endpoints) = link.get("nodes").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let mut runtime_endpoints = Vec::new();
    for endpoint in endpoints {
        let Some(node_id) = endpoint.get("node_id").and_then(Value::as_str) else {
            continue;
        };
        let row = sqlx::query(
            "SELECT object_json FROM topology_objects
             WHERE project_id = ? AND kind = 'node' AND object_id = ?",
        )
        .bind(project_id)
        .bind(node_id)
        .fetch_optional(pool)
        .await?;
        let Some(row) = row else { continue };
        let node = decode_json(row.get::<String, _>("object_json"))?;
        if node.get("node_type").and_then(Value::as_str) != Some("docker") {
            continue;
        }
        let Some(container_id) = node.get("container_id").and_then(Value::as_str) else {
            continue;
        };
        runtime_endpoints.push(json!({ "container_id": container_id }));
    }
    Ok(runtime_endpoints)
}

async fn cleanup_link_runtime(
    state: &AppState,
    project_id: &str,
    link: &Value,
) -> Result<(), ApiError> {
    let endpoints = link
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut node_types = HashSet::new();
    for endpoint in &endpoints {
        let Some(node_id) = endpoint.get("node_id").and_then(Value::as_str) else {
            continue;
        };
        let row = sqlx::query(
            "SELECT object_json FROM topology_objects
             WHERE project_id = ? AND kind = 'node' AND object_id = ?",
        )
        .bind(project_id)
        .bind(node_id)
        .fetch_optional(state.store.pool())
        .await?;
        if let Some(row) = row {
            let node = decode_json(row.get::<String, _>("object_json"))?;
            if let Some(node_type) = node.get("node_type").and_then(Value::as_str) {
                node_types.insert(node_type.to_owned());
            }
        }
    }
    if node_types.contains("qemu") {
        ensure_link_qemu_nodes_stopped(state, project_id, link).await?;
    }
    if node_types.contains("docker") {
        let link_id = link
            .get("link_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let cleanup = link_cleanup_endpoints(state.store.pool(), project_id, link).await?;
        state
            .runtime
            .remove_link(link_id, &cleanup)
            .await
            .map_err(runtime_api_error)?;
    }
    Ok(())
}

async fn get_node(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    let project_id = project_id.to_string();
    let node = get_topology_object(
        state.store.pool(),
        &project_id,
        "node",
        &node_id.to_string(),
    )
    .await?
    .0;
    let mut nodes = refresh_node_statuses(&state, &project_id, vec![node]).await;
    Ok(Json(nodes.pop().expect("the requested node was provided")))
}

async fn update_node(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
    Json(patch): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    update_topology_object(
        state,
        project_id.to_string(),
        node_id.to_string(),
        "node",
        "node_id",
        patch,
    )
    .await
}

async fn delete_node(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    delete_node_and_connected_links(state, project_id.to_string(), node_id.to_string()).await
}

async fn start_node(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    change_node_runtime_state(state, project_id, node_id, NodeAction::Start).await
}

async fn stop_node(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    change_node_runtime_state(state, project_id, node_id, NodeAction::Stop).await
}

async fn suspend_node(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    change_node_runtime_state(state, project_id, node_id, NodeAction::Suspend).await
}

async fn reload_node(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    change_node_runtime_state(state, project_id, node_id, NodeAction::Reload).await
}

async fn change_node_runtime_state(
    state: AppState,
    project_id: Uuid,
    node_id: Uuid,
    action: NodeAction,
) -> Result<Json<Value>, ApiError> {
    let project_id = project_id.to_string();
    let node_id = node_id.to_string();
    let mut node = get_topology_object(state.store.pool(), &project_id, "node", &node_id)
        .await?
        .0;
    let status = apply_node_lifecycle(&state, &project_id, &mut node, action).await?;
    node["status"] = json!(status);
    let mut tx = state.store.pool().begin().await?;
    sqlx::query(
        "UPDATE topology_objects SET object_json = ?
         WHERE project_id = ? AND kind = 'node' AND object_id = ?",
    )
    .bind(serde_json::to_string(&node).map_err(internal_json_error)?)
    .bind(&project_id)
    .bind(&node_id)
    .execute(&mut *tx)
    .await?;
    let event = Store::append_event(&mut tx, &project_id, "node.updated", &node).await?;
    tx.commit().await?;
    state.store.publish(event);
    Ok(Json(node))
}

async fn get_link(
    State(state): State<AppState>,
    Path((project_id, link_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    get_topology_object(
        state.store.pool(),
        &project_id.to_string(),
        "link",
        &link_id.to_string(),
    )
    .await
}

async fn update_link(
    State(state): State<AppState>,
    Path((project_id, link_id)): Path<(Uuid, Uuid)>,
    Json(patch): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    if patch_has_changed_link_nodes(&state, project_id, link_id, &patch).await? {
        return Err(ApiError::conflict(
            "Delete and recreate the link to change runtime endpoints",
        ));
    }
    update_topology_object(
        state,
        project_id.to_string(),
        link_id.to_string(),
        "link",
        "link_id",
        patch,
    )
    .await
}

async fn patch_has_changed_link_nodes(
    state: &AppState,
    project_id: Uuid,
    link_id: Uuid,
    patch: &Value,
) -> Result<bool, ApiError> {
    let Some(nodes) = patch.get("nodes") else {
        return Ok(false);
    };
    let current = get_topology_object(
        state.store.pool(),
        &project_id.to_string(),
        "link",
        &link_id.to_string(),
    )
    .await?;
    Ok(current.0.get("nodes") != Some(nodes))
}

async fn delete_link(
    State(state): State<AppState>,
    Path((project_id, link_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let project_id = project_id.to_string();
    let link_id = link_id.to_string();
    let link = get_topology_object(state.store.pool(), &project_id, "link", &link_id)
        .await?
        .0;
    cleanup_link_runtime(&state, &project_id, &link).await?;
    delete_topology_object(state, project_id, link_id, "link").await
}

async fn get_drawing(
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

async fn update_drawing(
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

async fn delete_drawing(
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

async fn create_drawing(
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

async fn create_topology_object(
    state: AppState,
    project_id: &str,
    kind: &str,
    id_field: &str,
    value: Value,
) -> Result<Response, ApiError> {
    let object = value
        .as_object()
        .ok_or_else(|| ApiError::bad_request("topology object must be an object"))?;
    let object_id = object
        .get(id_field)
        .and_then(Value::as_str)
        .expect("create handlers fill generated IDs")
        .to_owned();
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut tx = state.store.pool().begin().await?;
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

async fn get_topology_object(
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

async fn update_topology_object(
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
    let mut tx = state.store.pool().begin().await?;
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

async fn delete_topology_object(
    state: AppState,
    project_id: String,
    object_id: String,
    kind: &str,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.store.pool().begin().await?;
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

async fn delete_node_and_connected_links(
    state: AppState,
    project_id: String,
    node_id: String,
) -> Result<StatusCode, ApiError> {
    let mut tx = state.store.pool().begin().await?;
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
    for row in link_rows {
        let link_id: String = row.get("object_id");
        let link = decode_json(row.get::<String, _>("object_json"))?;
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
    for event in events {
        state.store.publish(event);
    }
    Ok(StatusCode::NO_CONTENT)
}

fn validate_link_shape(endpoints: &[Value]) -> Result<(), ApiError> {
    if endpoints.len() != 2 {
        return Err(ApiError::bad_request(
            "link must have exactly two endpoints",
        ));
    }
    let mut unique_nodes = HashSet::new();
    for endpoint in endpoints {
        let node_id = endpoint
            .get("node_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ApiError::bad_request("each link endpoint requires node_id"))?;
        if !unique_nodes.insert(node_id.to_owned()) {
            return Err(ApiError::bad_request(
                "a link cannot connect a node to itself",
            ));
        }
        for field in ["adapter_number", "port_number"] {
            if endpoint
                .get(field)
                .and_then(Value::as_i64)
                .is_none_or(|number| number < 0)
            {
                return Err(ApiError::bad_request(format!(
                    "each link endpoint requires a non-negative {field}"
                )));
            }
        }
    }
    Ok(())
}

async fn validate_link_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    project_id: &str,
    link: &Value,
) -> Result<(), ApiError> {
    let endpoints = link
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::bad_request("link must contain a nodes array"))?;
    validate_link_shape(endpoints)?;
    for endpoint in endpoints {
        let node_id = endpoint
            .get("node_id")
            .and_then(Value::as_str)
            .expect("link shape was validated");
        let exists = sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(
                SELECT 1 FROM topology_objects
                WHERE project_id = ? AND kind = 'node' AND object_id = ?
            )",
        )
        .bind(project_id)
        .bind(node_id)
        .fetch_one(&mut **tx)
        .await?;
        if exists == 0 {
            return Err(ApiError::bad_request(format!(
                "Node {node_id} does not exist in this project"
            )));
        }
        let adapter_number = endpoint
            .get("adapter_number")
            .and_then(Value::as_i64)
            .expect("link shape was validated");
        let port_number = endpoint
            .get("port_number")
            .and_then(Value::as_i64)
            .expect("link shape was validated");
        let occupied = sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(
                SELECT 1 FROM link_endpoints
                WHERE project_id = ? AND node_id = ?
                  AND adapter_number = ? AND port_number = ?
            )",
        )
        .bind(project_id)
        .bind(node_id)
        .bind(adapter_number)
        .bind(port_number)
        .fetch_one(&mut **tx)
        .await?;
        if occupied != 0 {
            return Err(ApiError::conflict(format!(
                "Port {adapter_number}/{port_number} on node {node_id} is already linked"
            )));
        }
    }
    Ok(())
}

async fn store_link_endpoints_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    project_id: &str,
    link_id: &str,
    link: &Value,
) -> Result<(), ApiError> {
    let endpoints = link
        .get("nodes")
        .and_then(Value::as_array)
        .expect("link shape was validated");
    for (index, endpoint) in endpoints.iter().enumerate() {
        sqlx::query(
            "INSERT INTO link_endpoints
             (project_id, link_id, endpoint_index, node_id, adapter_number, port_number)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(project_id)
        .bind(link_id)
        .bind(index as i64)
        .bind(
            endpoint["node_id"]
                .as_str()
                .expect("link shape was validated"),
        )
        .bind(
            endpoint["adapter_number"]
                .as_i64()
                .expect("link shape was validated"),
        )
        .bind(
            endpoint["port_number"]
                .as_i64()
                .expect("link shape was validated"),
        )
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn get_topology_snapshot(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let project_id = project_id.to_string();
    let mut tx = state.store.pool().begin().await?;
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

async fn project_notifications(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Query(query): Query<NotificationQuery>,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let project_id = project_id.to_string();
    ensure_project_exists_pool(state.store.pool(), &project_id).await?;
    Ok(websocket
        .on_upgrade(move |socket| websocket_session(socket, state, project_id, query.after)))
}

#[derive(Default, serde::Deserialize)]
struct NotificationQuery {
    after: Option<i64>,
}

async fn websocket_session(
    socket: WebSocket,
    state: AppState,
    project_id: String,
    after: Option<i64>,
) {
    let mut receiver = state.store.subscribe();
    let (mut sink, mut stream) = socket.split();
    let mut cursor = match after {
        Some(cursor) => cursor,
        None => match latest_project_event(state.store.pool(), &project_id).await {
            Ok(cursor) => cursor,
            Err(error) => {
                tracing::error!(%error, %project_id, "could not initialize project event stream");
                return;
            }
        },
    };

    if after.is_some()
        && !send_events_after(&mut sink, &state.store, &project_id, &mut cursor).await
    {
        return;
    }

    loop {
        tokio::select! {
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => {}
                }
            }
            event = receiver.recv() => {
                match event {
                    Ok(event) if event.project_id == project_id && event.sequence > cursor => {
                        if !send_event(&mut sink, &event).await {
                            break;
                        }
                        cursor = event.sequence;
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(%project_id, skipped, "project event subscriber lagged; replaying from storage");
                        if !send_events_after(&mut sink, &state.store, &project_id, &mut cursor).await {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

async fn send_events_after(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    store: &Store,
    project_id: &str,
    cursor: &mut i64,
) -> bool {
    loop {
        let events = match store
            .events_after(project_id, *cursor, EVENT_PAGE_SIZE)
            .await
        {
            Ok(events) => events,
            Err(error) => {
                tracing::error!(%error, %project_id, "could not replay project events");
                return false;
            }
        };
        if events.is_empty() {
            return true;
        }
        let count = events.len();
        for event in events {
            if !send_event(sink, &event).await {
                return false;
            }
            *cursor = event.sequence;
        }
        if count < EVENT_PAGE_SIZE as usize {
            return true;
        }
    }
}

async fn send_event(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    event: &ProjectEvent,
) -> bool {
    match serde_json::to_string(event) {
        Ok(text) => sink.send(Message::Text(text.into())).await.is_ok(),
        Err(error) => {
            tracing::error!(%error, "could not encode project notification");
            false
        }
    }
}

async fn latest_project_event(pool: &SqlitePool, project_id: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COALESCE(MAX(sequence), 0) FROM project_events WHERE project_id = ?")
        .bind(project_id)
        .fetch_one(pool)
        .await
}

async fn list_objects(
    pool: &SqlitePool,
    project_id: &str,
    kind: &str,
) -> Result<Json<Vec<Value>>, ApiError> {
    ensure_project_exists_pool(pool, project_id).await?;
    let rows = sqlx::query(
        "SELECT object_json FROM topology_objects
         WHERE project_id = ? AND kind = ? ORDER BY name, object_id",
    )
    .bind(project_id)
    .bind(kind)
    .fetch_all(pool)
    .await?;
    let values = rows
        .into_iter()
        .map(|row| decode_json(row.get::<String, _>("object_json")))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(values))
}

async fn refresh_node_statuses(
    state: &AppState,
    project_id: &str,
    mut nodes: Vec<Value>,
) -> Vec<Value> {
    if nodes.is_empty() {
        return nodes;
    }
    let statuses = state.runtime.project_node_states(project_id).await.ok();
    let mut qemu_queries = Vec::new();
    for (index, node) in nodes.iter_mut().enumerate() {
        let Some(node_id) = node.get("node_id").and_then(Value::as_str) else {
            continue;
        };
        match node.get("node_type").and_then(Value::as_str) {
            Some("docker") if node.get("container_id").is_some() => {
                let Some(statuses) = statuses.as_ref() else {
                    continue;
                };
                match statuses.get(node_id) {
                    Some(status) => {
                        node["status"] = json!(status);
                        node["runtime_missing"] = json!(false);
                    }
                    None => {
                        node["status"] = json!("stopped");
                        node["runtime_missing"] = json!(true);
                    }
                }
            }
            Some("qemu") => match state.qemu.vm_from_node(node) {
                Ok(vm) if !vm.overlay.is_file() => {
                    node["status"] = json!("stopped");
                    node["runtime_missing"] = json!(true);
                }
                Ok(vm) => qemu_queries.push((index, node_id.to_owned(), vm)),
                Err(error) => {
                    tracing::debug!(error = %error.message(), %project_id, %node_id, "QEMU node metadata is invalid")
                }
            },
            _ => {}
        }
    }
    let qemu_results = futures_util::stream::iter(qemu_queries)
        .map(|(index, node_id, vm)| async move { (index, node_id, state.qemu.status(&vm).await) })
        .buffer_unordered(16)
        .collect::<Vec<_>>()
        .await;
    for (index, node_id, result) in qemu_results {
        match result {
            Ok(status) => {
                nodes[index]["status"] = json!(status);
                nodes[index]["runtime_missing"] = json!(false);
            }
            Err(error) => {
                tracing::debug!(error = %error.message(), %project_id, %node_id, "QEMU state refresh was unavailable")
            }
        }
    }
    nodes
}

async fn list_objects_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    project_id: &str,
    kind: &str,
) -> Result<Vec<Value>, ApiError> {
    let rows = sqlx::query(
        "SELECT object_json FROM topology_objects
         WHERE project_id = ? AND kind = ? ORDER BY name, object_id",
    )
    .bind(project_id)
    .bind(kind)
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter()
        .map(|row| decode_json(row.get::<String, _>("object_json")))
        .collect()
}

async fn fetch_project(pool: &SqlitePool, project_id: &str) -> Result<Value, ApiError> {
    let row = sqlx::query("SELECT project_json FROM projects WHERE project_id = ?")
        .bind(project_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Project {project_id} does not exist")))?;
    decode_json(row.get::<String, _>("project_json"))
}

async fn ensure_project_exists_pool(pool: &SqlitePool, project_id: &str) -> Result<(), ApiError> {
    let exists =
        sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM projects WHERE project_id = ?)")
            .bind(project_id)
            .fetch_one(pool)
            .await?;
    if exists == 0 {
        return Err(ApiError::not_found(format!(
            "Project {project_id} does not exist"
        )));
    }
    Ok(())
}

async fn ensure_project_exists(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    project_id: &str,
) -> Result<(), ApiError> {
    let exists =
        sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM projects WHERE project_id = ?)")
            .bind(project_id)
            .fetch_one(&mut **tx)
            .await?;
    if exists == 0 {
        return Err(ApiError::not_found(format!(
            "Project {project_id} does not exist"
        )));
    }
    Ok(())
}

fn as_object_mut<'a>(
    value: &'a mut Value,
    message: &str,
) -> Result<&'a mut Map<String, Value>, ApiError> {
    value
        .as_object_mut()
        .ok_or_else(|| ApiError::bad_request(message))
}

fn required_string<'a>(object: &'a Map<String, Value>, field: &str) -> Result<&'a str, ApiError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::bad_request(format!("{field} is required")))
}

fn parse_or_generate_uuid(value: Option<&Value>, field: &str) -> Result<String, ApiError> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(Uuid::new_v4().to_string());
    };
    let text = value
        .as_str()
        .ok_or_else(|| ApiError::bad_request(format!("{field} must be a UUID string")))?;
    let id = Uuid::parse_str(text)
        .map_err(|_| ApiError::bad_request(format!("{field} must be a UUID string")))?;
    if id.get_version_num() != 4 {
        return Err(ApiError::bad_request(format!(
            "{field} must be a version 4 UUID"
        )));
    }
    Ok(id.to_string())
}

fn set_default(object: &mut Map<String, Value>, field: &str, default: Value) {
    object.entry(field.to_owned()).or_insert(default);
}

fn decode_json(value: String) -> Result<Value, ApiError> {
    serde_json::from_str(&value).map_err(internal_json_error)
}

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        http::{Method, Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use serde_json::Value;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use super::{AppState, router};
    use crate::storage::Store;
    use crate::{qemu::QemuRuntime, runtime::DockerRuntime};

    async fn test_router() -> (axum::Router, TempDir) {
        let directory = TempDir::new().expect("temporary test directory");
        let database = directory.path().join("controller.sqlite");
        let store = Store::connect(database.to_str().expect("UTF-8 temporary path"))
            .await
            .expect("test database connects");
        let docker = directory.path().join("docker-test-double");
        let state_file = directory.path().join("container-state");
        let script = format!(
            "#!/bin/sh\nstate_file='{}'\ncase \"$1\" in\n  info) echo 29.6.2 ;;\n  image) if [ \"$2\" = ls ]; then printf '{{\"Repository\":\"alpine\",\"Tag\":\"latest\",\"ID\":\"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"}}\\n'; else echo sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef; fi ;;\n  create) echo 0123456789abcdef; echo created > \"$state_file\" ;;\n  inspect) cat \"$state_file\" ;;\n  start|unpause) echo running > \"$state_file\" ;;\n  stop|restart) echo exited > \"$state_file\" ;;\n  pause) echo paused > \"$state_file\" ;;\n  network) if [ \"$2\" = create ]; then echo test-network-id; fi; exit 0 ;;\n  ps) exit 0 ;;\n  *) exit 0 ;;\nesac\n",
            state_file.display()
        );
        std::fs::write(&docker, script).expect("write Docker test double");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755))
            .expect("make Docker test double executable");
        let qemu_images = directory.path().join("qemu-images");
        std::fs::create_dir_all(&qemu_images).expect("QEMU image directory exists");
        std::fs::write(qemu_images.join("router.qcow2"), b"base disk")
            .expect("QEMU base disk exists");
        let qemu = directory.path().join("qemu-system-test-double");
        let qemu_img = directory.path().join("qemu-img-test-double");
        let argument_log = directory.path().join("qemu-arguments.log");
        let test_binary = std::env::current_exe().expect("Rust test executable is available");
        let qemu_script = format!(
            "#!/bin/sh\nif [ \"$1\" = -version ]; then echo 'QEMU emulator version 9.0'; exit 0; fi\nprintf '%s\\n' \"$*\" >> '{}'\nqmp=''\npidfile=''\nwhile [ $# -gt 0 ]; do case \"$1\" in -qmp) qmp=\"$2\"; shift 2 ;; -pidfile) pidfile=\"$2\"; shift 2 ;; *) shift ;; esac; done\nqmp=${{qmp#unix:}}\nqmp=${{qmp%%,*}}\nNETWORK_SIMULATOR_TEST_QMP_SOCKET=\"$qmp\" NETWORK_SIMULATOR_TEST_QMP_PIDFILE=\"$pidfile\" '{}' --exact qemu::tests::qmp_process_fixture --nocapture >/dev/null 2>&1 &\necho $! > \"$pidfile\"\n",
            argument_log.display(),
            test_binary.display()
        );
        let qemu_img_script = "#!/bin/sh\ncase \"$1\" in\n  --version) echo 'qemu-img version 9.0' ;;\n  info) echo '{\"format\":\"qcow2\",\"virtual-size\":1048576}' ;;\n  create) for target do :; done; touch \"$target\" ;;\n  *) exit 1 ;;\nesac\n";
        std::fs::write(&qemu, qemu_script).expect("write QEMU test double");
        std::fs::write(&qemu_img, qemu_img_script).expect("write qemu-img test double");
        std::fs::set_permissions(&qemu, std::fs::Permissions::from_mode(0o755))
            .expect("make QEMU test double executable");
        std::fs::set_permissions(&qemu_img, std::fs::Permissions::from_mode(0o755))
            .expect("make qemu-img test double executable");
        (
            router(AppState::with_runtimes(
                store,
                DockerRuntime::with_binary(docker),
                QemuRuntime::with_paths(
                    qemu,
                    qemu_img,
                    qemu_images,
                    directory.path().join("qemu-state"),
                    directory.path().join("qemu-sockets"),
                ),
            )),
            directory,
        )
    }

    async fn request(
        app: &axum::Router,
        method: Method,
        uri: String,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        let body = match body {
            Some(value) => {
                builder = builder.header("content-type", "application/json");
                Body::from(value.to_string())
            }
            None => Body::empty(),
        };
        let response = app
            .clone()
            .oneshot(builder.body(body).expect("request builds"))
            .await
            .expect("router returns a response");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("response body reads")
            .to_bytes();
        let json = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or_else(|error| {
                panic!(
                    "response is not JSON ({error}), status={status}, body={:?}",
                    String::from_utf8_lossy(&bytes)
                )
            })
        };
        (status, json)
    }

    #[tokio::test]
    async fn static_spa_serves_direct_project_routes_and_keeps_v3_errors_as_404() {
        let (app, _directory) = test_router().await;
        let response = app
            .clone()
            .oneshot(
                Request::get("/projects/p1")
                    .body(Body::empty())
                    .expect("project route request builds"),
            )
            .await
            .expect("project route returns a response");
        assert_eq!(response.status(), StatusCode::OK);
        let shell = response
            .into_body()
            .collect()
            .await
            .expect("SPA shell reads")
            .to_bytes();
        let shell = String::from_utf8(shell.to_vec()).expect("SPA shell is UTF-8");
        assert!(shell.contains("Network Studio"));
        let asset = shell
            .split("href=\"/assets/")
            .nth(1)
            .and_then(|path| path.split('\"').next())
            .expect("shell references its generated stylesheet");
        let response = app
            .clone()
            .oneshot(
                Request::get(format!("/assets/{asset}"))
                    .body(Body::empty())
                    .expect("asset request builds"),
            )
            .await
            .expect("stylesheet returns a response");
        assert_eq!(response.status(), StatusCode::OK);

        let response = app
            .oneshot(
                Request::get("/v3/not-a-real-route")
                    .body(Body::empty())
                    .expect("unknown API request builds"),
            )
            .await
            .expect("unknown API route returns a response");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn v3_topology_flow_persists_snapshot_and_cascades_node_links() {
        let (app, _directory) = test_router().await;
        let (status, project) = request(
            &app,
            Method::POST,
            "/v3/projects".into(),
            Some(serde_json::json!({ "name": "Core lab" })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(project["filename"], "Core lab.gns3");
        let project_id = project["project_id"].as_str().expect("project UUID");

        let create_node = |name: &str| serde_json::json!({ "name": name, "node_type": "docker", "image": "alpine:latest" });
        let (status, node_a) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes"),
            Some(create_node("Edge A")),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(node_a["node_type"], "docker");
        assert!(node_a["container_id"].is_string());
        let (status, node_b) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes"),
            Some(create_node("Edge B")),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        let endpoints = serde_json::json!([
            { "node_id": node_a["node_id"], "adapter_number": 0, "port_number": 0 },
            { "node_id": node_b["node_id"], "adapter_number": 0, "port_number": 0 }
        ]);
        let (status, link) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/links"),
            Some(serde_json::json!({ "nodes": endpoints })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(link["nodes"].as_array().map(Vec::len), Some(2));

        let (status, conflict) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/links"),
            Some(serde_json::json!({ "nodes": endpoints })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(conflict["message"].is_string());

        let (status, snapshot) = request(
            &app,
            Method::GET,
            format!("/v3/projects/{project_id}/topology"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(snapshot["nodes"].as_array().map(Vec::len), Some(2));
        assert_eq!(snapshot["links"].as_array().map(Vec::len), Some(1));
        assert!(snapshot["event_cursor"].as_i64().unwrap_or_default() >= 4);

        let (status, _) = request(
            &app,
            Method::DELETE,
            format!(
                "/v3/projects/{project_id}/nodes/{}",
                node_a["node_id"].as_str().unwrap()
            ),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (_, snapshot) = request(
            &app,
            Method::GET,
            format!("/v3/projects/{project_id}/topology"),
            None,
        )
        .await;
        assert_eq!(snapshot["nodes"].as_array().map(Vec::len), Some(1));
        assert_eq!(snapshot["links"].as_array().map(Vec::len), Some(0));
    }

    #[tokio::test]
    async fn templates_create_nodes_with_real_runtime_lifecycle_routes() {
        let (app, _directory) = test_router().await;
        let (status, templates) = request(&app, Method::GET, "/v3/templates".into(), None).await;
        assert_eq!(status, StatusCode::OK);
        let template = templates[0]["template_id"].as_str().expect("template UUID");
        assert_eq!(templates[0]["template_type"], "docker");

        let (_, project) = request(
            &app,
            Method::POST,
            "/v3/projects".into(),
            Some(serde_json::json!({ "name": "Runtime lab" })),
        )
        .await;
        let project_id = project["project_id"].as_str().unwrap();
        let (status, node) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/templates/{template}"),
            Some(serde_json::json!({
                "name": "Alpine node",
                "x": 24,
                "y": -18,
                "start_command": "/bin/sh -c 'sleep 3600'"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(node["image"], "alpine:latest");
        assert_eq!(node["x"], 24);
        assert_eq!(node["start_command"], "/bin/sh -c 'sleep 3600'");

        let (status, node) = request(
            &app,
            Method::POST,
            format!(
                "/v3/projects/{project_id}/nodes/{}/start",
                node["node_id"].as_str().unwrap()
            ),
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(node["status"], "started");

        let (status, node) = request(
            &app,
            Method::POST,
            format!(
                "/v3/projects/{project_id}/nodes/{}/stop",
                node["node_id"].as_str().unwrap()
            ),
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(node["status"], "stopped");
    }

    #[tokio::test]
    async fn qemu_templates_create_private_disks_and_exercise_qmp_process_lifecycle() {
        let (app, directory) = test_router().await;
        let (status, templates) = request(&app, Method::GET, "/v3/templates".into(), None).await;
        assert_eq!(status, StatusCode::OK);
        let template = templates
            .as_array()
            .unwrap()
            .iter()
            .find(|template| template["template_type"] == "qemu")
            .expect("local QEMU disk template is listed");
        let template_id = template["template_id"].as_str().unwrap();
        assert_eq!(template["name"], "router.qcow2");

        let (_, project) = request(
            &app,
            Method::POST,
            "/v3/projects".into(),
            Some(serde_json::json!({ "name": "QEMU link lab" })),
        )
        .await;
        let project_id = project["project_id"].as_str().unwrap();
        let mut qemu_nodes = Vec::new();
        for name in ["Router A", "Router B"] {
            let (status, node) = request(
                &app,
                Method::POST,
                format!("/v3/projects/{project_id}/templates/{template_id}"),
                Some(serde_json::json!({ "name": name })),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED);
            assert_eq!(node["node_type"], "qemu");
            assert!(std::path::Path::new(node["runtime_overlay"].as_str().unwrap()).is_file());
            assert_eq!(node["status"], "stopped");
            qemu_nodes.push(node);
        }

        let (status, link) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/links"),
            Some(serde_json::json!({
                "nodes": [
                    { "node_id": qemu_nodes[0]["node_id"], "adapter_number": 0, "port_number": 0 },
                    { "node_id": qemu_nodes[1]["node_id"], "adapter_number": 1, "port_number": 0 }
                ]
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let local_a = link["nodes"][0]["runtime_local_port"].as_u64().unwrap();
        let peer_a = link["nodes"][0]["runtime_remote_port"].as_u64().unwrap();
        assert_eq!(local_a, link["nodes"][1]["runtime_remote_port"]);
        assert_eq!(peer_a, link["nodes"][1]["runtime_local_port"]);
        assert_ne!(local_a, peer_a);

        let node_a = qemu_nodes[0]["node_id"].as_str().unwrap();
        let (status, node) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes/{node_a}/start"),
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(node["status"], "started");
        let (status, node) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes/{node_a}/suspend"),
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(node["status"], "suspended");
        let (status, node) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes/{node_a}/start"),
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(node["status"], "started");
        let (status, _) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes/{node_a}/reload"),
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let node_b = qemu_nodes[1]["node_id"].as_str().unwrap();
        let (status, node_b) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes/{node_b}/start"),
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(node_b["status"], "started");
        let args = std::fs::read_to_string(directory.path().join("qemu-arguments.log"))
            .expect("QEMU launch arguments were recorded");
        let netdev = format!(
            "dgram,id=ns_{}",
            link["link_id"].as_str().unwrap().replace('-', "")
        );
        assert!(args.contains(&netdev));
        assert!(args.contains(&format!("local.port={local_a}")));
        assert!(args.contains(&format!("remote.port={peer_a}")));
        let (status, error) = request(
            &app,
            Method::DELETE,
            format!(
                "/v3/projects/{project_id}/links/{}",
                link["link_id"].as_str().unwrap()
            ),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("Stop both QEMU nodes")
        );

        for node_id in [node_a, node_b["node_id"].as_str().unwrap()] {
            let (status, node) = request(
                &app,
                Method::POST,
                format!("/v3/projects/{project_id}/nodes/{node_id}/stop"),
                Some(serde_json::json!({})),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(node["status"], "stopped");
        }
        for node in &qemu_nodes {
            assert!(std::path::Path::new(node["runtime_overlay"].as_str().unwrap()).is_file());
        }
        let (status, _) = request(
            &app,
            Method::DELETE,
            format!("/v3/projects/{project_id}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        for node in qemu_nodes {
            assert!(!std::path::Path::new(node["runtime_overlay"].as_str().unwrap()).exists());
        }
    }

    #[tokio::test]
    async fn rejects_unsupported_node_types() {
        let (app, _directory) = test_router().await;
        let (_, project) = request(
            &app,
            Method::POST,
            "/v3/projects".into(),
            Some(serde_json::json!({ "name": "Runtime compatibility" })),
        )
        .await;
        let project_id = project["project_id"].as_str().unwrap();
        let (status, error) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes"),
            Some(serde_json::json!({ "name": "Fake switch", "node_type": "ethernet_switch" })),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("Executable node types")
        );
    }
}

fn internal_json_error(error: serde_json::Error) -> ApiError {
    tracing::error!(%error, "stored JSON could not be decoded");
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Stored data is invalid")
}
