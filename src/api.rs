use std::collections::HashSet;

use axum::{
    Json, Router,
    body::Body,
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{any, get, post},
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::{Row, SqlitePool};
use tokio::sync::broadcast;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UnixStream},
};
use tower_http::{
    limit::RequestBodyLimitLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use uuid::Uuid;

use crate::{
    dynamips::{DynamipsRuntime, DynamipsVm},
    marketplace::{ApplianceRecord, MarketplaceCatalog},
    qemu::{QemuLink, QemuRuntime},
    runtime::{DockerRuntime, NodeAction, RuntimeError, parse_start_command},
    storage::{ProjectEvent, Store},
    ubridge::{self, UbridgeRuntime},
    vpcs::VpcsRuntime,
};

const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;
const MAX_DYNAMIPS_IMAGE_BYTES: usize = 512 * 1024 * 1024;
const MAX_QEMU_IMAGE_BYTES: usize = 64 * 1024 * 1024 * 1024;
const EVENT_PAGE_SIZE: i64 = 512;

#[derive(Clone)]
pub struct AppState {
    store: Store,
    runtime: DockerRuntime,
    qemu: QemuRuntime,
    vpcs: VpcsRuntime,
    dynamips: DynamipsRuntime,
    ubridge: UbridgeRuntime,
    marketplace: MarketplaceCatalog,
}

impl AppState {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            runtime: DockerRuntime::new(),
            qemu: QemuRuntime::new(),
            vpcs: VpcsRuntime::new(),
            dynamips: DynamipsRuntime::new(),
            ubridge: UbridgeRuntime::new(),
            marketplace: MarketplaceCatalog::new(),
        }
    }

    #[cfg(test)]
    fn with_runtimes(
        store: Store,
        runtime: DockerRuntime,
        qemu: QemuRuntime,
        vpcs: VpcsRuntime,
        dynamips: DynamipsRuntime,
    ) -> Self {
        Self {
            store,
            runtime,
            qemu,
            vpcs,
            dynamips,
            ubridge: UbridgeRuntime::new(),
            marketplace: MarketplaceCatalog::new(),
        }
    }
}

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/healthz", get(health))
        .route("/v3/version", get(version))
        .route("/v3/runtimes", get(runtimes))
        .route("/v3/resources/monitor", get(resource_monitor))
        .route("/v3/templates", get(list_templates))
        .route("/v3/marketplace/ishare2/images", get(search_ishare2_images))
        .route(
            "/v3/marketplace/ishare2/images/{image_type}/{image_id}/download",
            post(download_ishare2_image),
        )
        .route(
            "/v3/marketplace/gns3/appliances",
            get(search_gns3_appliances),
        )
        .route(
            "/v3/marketplace/gns3/appliances/{file_name}",
            get(get_gns3_appliance),
        )
        .route(
            "/v3/marketplace/gns3/appliances/{file_name}/import",
            post(import_gns3_appliance),
        )
        .route(
            "/v3/marketplace/gns3/imported",
            get(list_imported_appliances),
        )
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
        .route(
            "/v3/compute/projects/{project_id}/qemu/nodes/{node_id}/console/ws",
            get(qemu_console),
        )
        .route(
            "/v3/compute/projects/{project_id}/docker/nodes/{node_id}/console/ws",
            get(docker_console),
        )
        .route(
            "/v3/compute/projects/{project_id}/vpcs/nodes/{node_id}/console/ws",
            get(vpcs_console),
        )
        .route(
            "/v3/compute/projects/{project_id}/dynamips/nodes/{node_id}/console/ws",
            get(dynamips_console),
        )
        .route("/v3", any(|| async { StatusCode::NOT_FOUND }))
        .route("/v3/{*path}", any(|| async { StatusCode::NOT_FOUND }))
        .route_service("/", ServeFile::new("frontend/dist/client/index.html"))
        .route_service(
            "/{*path}",
            ServeFile::new("frontend/dist/client/index.html"),
        )
        .nest_service("/assets", ServeDir::new("frontend/dist/client/assets"))
        .route_layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES));
    let image_upload = Router::new()
        .route(
            "/v3/dynamips/images/{file_name}",
            post(upload_dynamips_image),
        )
        .layer(RequestBodyLimitLayer::new(MAX_DYNAMIPS_IMAGE_BYTES));
    let qemu_image_upload = Router::new()
        .route("/v3/qemu/images/{file_name}", post(upload_qemu_image))
        .layer(RequestBodyLimitLayer::new(MAX_QEMU_IMAGE_BYTES));
    Router::new()
        .merge(api)
        .merge(image_upload)
        .merge(qemu_image_upload)
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
        "qemu": state.qemu.info().await,
        "vpcs": state.vpcs.info().await,
        "dynamips": state.dynamips.info().await,
        "ubridge": state.ubridge.info().await
    }))
}

async fn resource_monitor() -> Json<Value> {
    let cpu_before = proc_cpu_times();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let cpu_after = proc_cpu_times();
    let cpu_usage_percent = cpu_before.zip(cpu_after).and_then(|(before, after)| {
        let total = after.0.checked_sub(before.0)?;
        let idle = after.1.checked_sub(before.1)?;
        (total > 0)
            .then(|| ((total.saturating_sub(idle) as f64 / total as f64) * 100.0).clamp(0.0, 100.0))
    });
    let memory = proc_memory_bytes();
    let controller = proc_controller_stats();
    let load_average = std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|contents| {
            let values = contents
                .split_whitespace()
                .take(3)
                .map(str::parse::<f64>)
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            (values.len() == 3).then_some(values)
        });
    Json(json!({
        "sampled_at_unix_ms": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_millis()),
        "host": {
            "cpu_usage_percent": cpu_usage_percent,
            "cpu_count": proc_cpu_count(),
            "memory_used_bytes": memory.map(|(used, _)| used),
            "memory_total_bytes": memory.map(|(_, total)| total),
            "load_average": load_average
        },
        "controller": controller
    }))
}

fn proc_cpu_times() -> Option<(u64, u64)> {
    let contents = std::fs::read_to_string("/proc/stat").ok()?;
    let mut values = contents
        .lines()
        .find(|line| line.starts_with("cpu "))?
        .split_whitespace()
        .skip(1)
        .map(str::parse::<u64>);
    let times = values.by_ref().collect::<Result<Vec<_>, _>>().ok()?;
    let total = times.iter().copied().sum();
    let idle = times
        .get(3)
        .copied()
        .unwrap_or_default()
        .saturating_add(times.get(4).copied().unwrap_or_default());
    Some((total, idle))
}

fn proc_cpu_count() -> Option<usize> {
    let contents = std::fs::read_to_string("/proc/stat").ok()?;
    let count = contents
        .lines()
        .filter(|line| {
            line.strip_prefix("cpu").is_some_and(|suffix| {
                suffix
                    .chars()
                    .next()
                    .is_some_and(|digit| digit.is_ascii_digit())
            })
        })
        .count();
    (count > 0).then_some(count)
}

fn proc_memory_bytes() -> Option<(u64, u64)> {
    let contents = std::fs::read_to_string("/proc/meminfo").ok()?;
    let values = contents
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            let kibibytes = value.split_whitespace().next()?.parse::<u64>().ok()?;
            Some((key, kibibytes.saturating_mul(1024)))
        })
        .collect::<std::collections::HashMap<_, _>>();
    let total = *values.get("MemTotal")?;
    let available = *values.get("MemAvailable")?;
    Some((total.saturating_sub(available), total))
}

fn proc_controller_stats() -> Value {
    let contents = std::fs::read_to_string("/proc/self/status").ok();
    let mut rss_bytes = None;
    let mut thread_count = None;
    if let Some(contents) = contents {
        for line in contents.lines() {
            if let Some(value) = line.strip_prefix("VmRSS:") {
                rss_bytes = value
                    .split_whitespace()
                    .next()
                    .and_then(|value| value.parse::<u64>().ok())
                    .map(|kibibytes| kibibytes.saturating_mul(1024));
            } else if let Some(value) = line.strip_prefix("Threads:") {
                thread_count = value.trim().parse::<u64>().ok();
            }
        }
    }
    json!({ "memory_rss_bytes": rss_bytes, "thread_count": thread_count })
}

#[derive(Deserialize)]
struct CatalogSearchQuery {
    q: Option<String>,
    kind: Option<String>,
}

async fn search_ishare2_images(
    State(state): State<AppState>,
    Query(query): Query<CatalogSearchQuery>,
) -> Result<Json<Value>, ApiError> {
    let results = state
        .marketplace
        .search_ishare2(
            query.q.as_deref().unwrap_or_default(),
            query.kind.as_deref().unwrap_or("all"),
        )
        .await
        .map_err(catalog_api_error)?;
    Ok(Json(json!({
        "source": "iShare2 / LabHub",
        "results": results
    })))
}

async fn download_ishare2_image(
    State(state): State<AppState>,
    Path((image_type, image_id)): Path<(String, u64)>,
) -> Result<Json<crate::marketplace::IshareDownloadResult>, ApiError> {
    state
        .marketplace
        .download_ishare2(&image_type, image_id)
        .await
        .map(Json)
        .map_err(catalog_api_error)
}

async fn search_gns3_appliances(
    State(state): State<AppState>,
    Query(query): Query<CatalogSearchQuery>,
) -> Result<Json<Value>, ApiError> {
    let results = state
        .marketplace
        .search_gns3_appliances(query.q.as_deref().unwrap_or_default())
        .await
        .map_err(catalog_api_error)?;
    Ok(Json(json!({
        "source": "GNS3 Marketplace appliance registry",
        "results": results
    })))
}

async fn get_gns3_appliance(
    State(state): State<AppState>,
    Path(file_name): Path<String>,
) -> Result<Json<ApplianceRecord>, ApiError> {
    state
        .marketplace
        .appliance_details(&file_name)
        .await
        .map(Json)
        .map_err(catalog_api_error)
}

async fn import_gns3_appliance(
    State(state): State<AppState>,
    Path(file_name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let appliance = state
        .marketplace
        .import_appliance(&file_name)
        .await
        .map_err(catalog_api_error)?;
    let templates = available_templates(&state).await;
    let matching_templates = templates
        .iter()
        .filter(|template| template["appliance_id"].as_str() == Some(&appliance.appliance_id))
        .count();
    Ok(Json(json!({
        "appliance": appliance,
        "matching_local_templates": matching_templates,
        "message": if matching_templates > 0 {
            "Appliance imported and matched to installed runtime images"
        } else {
            "Appliance definition imported; install or select its required image to use it"
        }
    })))
}

async fn list_imported_appliances(
    State(state): State<AppState>,
) -> Result<Json<Vec<ApplianceRecord>>, ApiError> {
    state
        .marketplace
        .imported_appliances()
        .map(Json)
        .map_err(catalog_api_error)
}

fn catalog_api_error(message: String) -> ApiError {
    ApiError::new(StatusCode::BAD_GATEWAY, message)
}

async fn upload_dynamips_image(
    State(state): State<AppState>,
    Path(file_name): Path<String>,
    body: Body,
) -> Result<Json<Value>, ApiError> {
    let (temporary_path, final_path) = state
        .dynamips
        .prepare_upload_name(&file_name)
        .map_err(runtime_api_error)?;
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)
        .await
        .map_err(|error| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Could not create temporary IOS upload file: {error}"),
            )
        })?;
    let mut stream = body.into_data_stream();
    let mut size = 0_usize;
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                drop(file);
                let _ = std::fs::remove_file(&temporary_path);
                return Err(ApiError::bad_request(format!(
                    "IOS image upload failed: {error}"
                )));
            }
        };
        size = size.saturating_add(chunk.len());
        if size > MAX_DYNAMIPS_IMAGE_BYTES {
            drop(file);
            let _ = std::fs::remove_file(&temporary_path);
            return Err(ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "IOS images must be 512 MiB or smaller",
            ));
        }
        if let Err(error) = file.write_all(&chunk).await {
            drop(file);
            let _ = std::fs::remove_file(&temporary_path);
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Could not write IOS image upload: {error}"),
            ));
        }
    }
    if let Err(error) = file.flush().await {
        drop(file);
        let _ = std::fs::remove_file(&temporary_path);
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not flush IOS image upload: {error}"),
        ));
    }
    drop(file);
    if size == 0 {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(ApiError::bad_request("The IOS image upload is empty"));
    }
    let image = match state
        .dynamips
        .finalize_uploaded_image(&temporary_path, &final_path)
    {
        Ok(image) => image,
        Err(error) => {
            let _ = std::fs::remove_file(&temporary_path);
            return Err(runtime_api_error(error));
        }
    };
    Ok(Json(json!({
        "name": file_name,
        "image": image,
        "template_type": "dynamips",
        "bytes": size
    })))
}

async fn upload_qemu_image(
    State(state): State<AppState>,
    Path(file_name): Path<String>,
    body: Body,
) -> Result<Json<Value>, ApiError> {
    let (temporary_path, final_path) = state
        .qemu
        .prepare_upload_name(&file_name)
        .map_err(runtime_api_error)?;
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)
        .await
        .map_err(|error| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Could not create temporary QEMU upload file: {error}"),
            )
        })?;
    let mut stream = body.into_data_stream();
    let mut size = 0_usize;
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                drop(file);
                let _ = std::fs::remove_file(&temporary_path);
                return Err(ApiError::bad_request(format!(
                    "QEMU image upload failed: {error}"
                )));
            }
        };
        size = size.saturating_add(chunk.len());
        if size > MAX_QEMU_IMAGE_BYTES {
            drop(file);
            let _ = std::fs::remove_file(&temporary_path);
            return Err(ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "QEMU disk images must be 64 GiB or smaller",
            ));
        }
        if let Err(error) = file.write_all(&chunk).await {
            drop(file);
            let _ = std::fs::remove_file(&temporary_path);
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Could not write QEMU image upload: {error}"),
            ));
        }
    }
    if let Err(error) = file.flush().await {
        drop(file);
        let _ = std::fs::remove_file(&temporary_path);
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not flush QEMU image upload: {error}"),
        ));
    }
    drop(file);
    if size == 0 {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(ApiError::bad_request("QEMU image upload is empty"));
    }
    match state
        .qemu
        .finalize_uploaded_image(&temporary_path, &final_path)
        .await
    {
        Ok(image) => Ok(Json(json!({
            "name": file_name,
            "image": image,
            "template_type": "qemu",
            "bytes": size
        }))),
        Err(error) => {
            let _ = std::fs::remove_file(&temporary_path);
            Err(runtime_api_error(error))
        }
    }
}

async fn list_templates(State(state): State<AppState>) -> Result<Json<Vec<Value>>, ApiError> {
    Ok(Json(available_templates(&state).await))
}

async fn available_templates(state: &AppState) -> Vec<Value> {
    let mut templates = Vec::new();
    match state.runtime.templates().await {
        Ok(available) => templates.extend(available),
        Err(error) => tracing::debug!(error = %error.message(), "Docker templates are unavailable"),
    }
    match state.qemu.templates().await {
        Ok(available) => templates.extend(available),
        Err(error) => tracing::debug!(error = %error.message(), "QEMU templates are unavailable"),
    }
    match state.vpcs.templates().await {
        Ok(available) => templates.extend(available),
        Err(error) => tracing::debug!(error = %error.message(), "VPCS templates are unavailable"),
    }
    match state.dynamips.templates().await {
        Ok(available) => templates.extend(available),
        Err(error) => {
            tracing::debug!(error = %error.message(), "Dynamips templates are unavailable")
        }
    }
    state.marketplace.templates(templates)
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
        Some("vpcs") => create_vpcs_node(state, project_id.to_string(), node).await,
        Some("dynamips") => create_dynamips_node(state, project_id.to_string(), node).await,
        Some(node_type) => Err(ApiError::bad_request(format!(
            "Unsupported node type '{node_type}'. Executable node types are Docker, QEMU, VPCS, and Dynamips."
        ))),
        None => Err(ApiError::bad_request("node_type is required")),
    }
}

async fn create_node_from_template(
    State(state): State<AppState>,
    Path((project_id, template_id)): Path<(Uuid, Uuid)>,
    Json(usage): Json<Value>,
) -> Result<impl IntoResponse, ApiError> {
    let templates = available_templates(&state).await;
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
    } else if node_type == "vpcs" {
        json!({
            "name": name,
            "node_type": "vpcs",
            "template_id": template_id,
            "adapters": 1,
            "memory": 0,
            "cpus": 0,
            "startup_script": usage.get("startup_script").cloned().unwrap_or(json!("")),
            "x": usage.get("x").cloned().unwrap_or(json!(0)),
            "y": usage.get("y").cloned().unwrap_or(json!(0))
        })
    } else if node_type == "dynamips" {
        json!({
            "name": name,
            "node_type": "dynamips",
            "template_id": template_id,
            "platform": template["platform"],
            "image": template["image"],
            "adapters": template["adapters"],
            "ram": template["memory"],
            "idlepc": usage.get("idlepc").cloned().unwrap_or(template["idlepc"].clone()),
            "startup_config": usage.get("startup_config").cloned().unwrap_or(json!("")),
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
        "vpcs" => create_vpcs_node(state, project_id.to_string(), node).await,
        "dynamips" => create_dynamips_node(state, project_id.to_string(), node).await,
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

async fn create_vpcs_node(
    state: AppState,
    project_id: String,
    mut node: Value,
) -> Result<Response, ApiError> {
    let object = as_object_mut(&mut node, "node body must be an object")?;
    let name = required_string(object, "name")?.to_owned();
    let node_type = required_string(object, "node_type")?;
    if node_type != "vpcs" {
        return Err(ApiError::bad_request(format!(
            "Unsupported VPCS node type '{node_type}'"
        )));
    }
    if object
        .get("compute_id")
        .and_then(Value::as_str)
        .is_some_and(|compute_id| compute_id != "local")
    {
        return Err(ApiError::bad_request(
            "This runtime build can execute nodes only on the local VPCS compute",
        ));
    }
    state
        .vpcs
        .ensure_available()
        .await
        .map_err(runtime_api_error)?;
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
    object.insert("project_id".into(), json!(project_id.clone()));
    object.insert("node_type".into(), json!("vpcs"));
    set_default(object, "compute_id", json!("local"));
    set_default(object, "status", json!("stopped"));
    set_default(object, "x", json!(0));
    set_default(object, "y", json!(0));
    set_default(object, "z", json!(1));
    set_default(object, "locked", json!(false));
    set_default(object, "adapters", json!(1));
    set_default(object, "memory", json!(0));
    set_default(object, "cpus", json!(0));
    set_default(object, "startup_script", json!(""));
    set_default(object, "properties", json!({}));
    if object.get("adapters").and_then(Value::as_u64) != Some(1) {
        return Err(ApiError::bad_request(
            "VPCS provides exactly one Ethernet adapter",
        ));
    }

    create_topology_object(state, &project_id, "node", "node_id", node).await
}

async fn create_dynamips_node(
    state: AppState,
    project_id: String,
    mut node: Value,
) -> Result<Response, ApiError> {
    let object = as_object_mut(&mut node, "node body must be an object")?;
    let name = required_string(object, "name")?.to_owned();
    if required_string(object, "node_type")? != "dynamips" {
        return Err(ApiError::bad_request(
            "Dynamips nodes require node_type 'dynamips'",
        ));
    }
    if object
        .get("compute_id")
        .and_then(Value::as_str)
        .is_some_and(|compute_id| compute_id != "local")
    {
        return Err(ApiError::bad_request(
            "This runtime build can execute nodes only on the local Dynamips compute",
        ));
    }
    let image = required_string(object, "image")?.to_owned();
    let platform = object
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or("c7200")
        .to_owned();
    let ram_mb = object.get("ram").and_then(Value::as_u64).unwrap_or(256);
    let idle_pc = object
        .get("idlepc")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let startup_config = object
        .get("startup_config")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
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
    object.insert("project_id".into(), json!(project_id.clone()));
    object.insert("node_type".into(), json!("dynamips"));
    object.insert("platform".into(), json!(platform));
    object.insert("image".into(), json!(image));
    object.insert("ram".into(), json!(ram_mb));
    object.insert("idlepc".into(), json!(idle_pc));
    object.insert("startup_config".into(), json!(startup_config));
    set_default(object, "compute_id", json!("local"));
    set_default(object, "status", json!("stopped"));
    set_default(object, "x", json!(0));
    set_default(object, "y", json!(0));
    set_default(object, "z", json!(1));
    set_default(object, "locked", json!(false));
    set_default(object, "adapters", json!(1));
    set_default(object, "memory", json!(ram_mb));
    set_default(object, "nvram", json!(128));
    set_default(object, "properties", json!({}));

    let vm = state
        .dynamips
        .create_router(
            &project_id,
            &node_id.to_string(),
            &name,
            &image,
            &platform,
            ram_mb,
            idle_pc.as_deref(),
            &startup_config,
        )
        .await
        .map_err(runtime_api_error)?;
    for (key, value) in state
        .dynamips
        .runtime_fields(&vm)
        .as_object()
        .expect("runtime fields are an object")
    {
        object.insert(key.clone(), value.clone());
    }

    match create_topology_object(state.clone(), &project_id, "node", "node_id", node).await {
        Ok(response) => Ok(response),
        Err(error) => {
            if let Err(cleanup_error) = state.dynamips.remove_router(&vm).await {
                tracing::error!(error = %cleanup_error.message(), %node_id, "could not roll back created Dynamips router");
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
    let first_runtime_type = runtime_endpoints
        .first()
        .and_then(|endpoint| endpoint["node_type"].as_str())
        .unwrap_or_default();
    let mixed_runtime_link = runtime_endpoints
        .iter()
        .any(|endpoint| endpoint["node_type"].as_str() != Some(first_runtime_type));
    if mixed_runtime_link {
        ensure_link_nodes_stopped(&state, &project_id, &runtime_endpoints).await?;
        create_shared_ethernet_link(&state, &link_id, &mut link, &runtime_endpoints).await?;
        let cleanup_link = link.clone();
        return match create_topology_object(state.clone(), &project_id, "link", "link_id", link)
            .await
        {
            Ok(response) => Ok(response),
            Err(error) => {
                if let Err(cleanup_error) =
                    cleanup_shared_ethernet_link(&state, &link_id, &cleanup_link).await
                {
                    tracing::error!(error = %cleanup_error.message, %link_id, "could not roll back shared Ethernet segment");
                }
                Err(error)
            }
        };
    }
    let udp_segment = runtime_endpoints
        .iter()
        .all(|endpoint| matches!(endpoint["node_type"].as_str(), Some("vpcs" | "dynamips")));
    let runtime_type = if udp_segment {
        "udp_segment"
    } else {
        first_runtime_type
    };
    if runtime_type == "docker" {
        state
            .runtime
            .create_link(&project_id, &link_id, &runtime_endpoints)
            .await
            .map_err(runtime_api_error)?;
    } else if runtime_type == "qemu" {
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
                    "Stop all QEMU nodes attached to this Ethernet segment before changing it",
                ));
            }
        }
        let (multicast_address, multicast_port) =
            QemuRuntime::multicast_endpoint(&link_id).map_err(runtime_api_error)?;
        let endpoints = link["nodes"]
            .as_array_mut()
            .ok_or_else(|| ApiError::bad_request("link must contain a nodes array"))?;
        for endpoint in endpoints {
            let endpoint_object = endpoint
                .as_object_mut()
                .ok_or_else(|| ApiError::bad_request("each link endpoint must be an object"))?;
            endpoint_object.insert(
                "runtime_multicast_address".into(),
                json!(multicast_address.to_string()),
            );
            endpoint_object.insert("runtime_multicast_port".into(), json!(multicast_port));
        }
    } else if runtime_type == "udp_segment" {
        let node_ids = runtime_endpoints
            .iter()
            .filter_map(|endpoint| endpoint["node_id"].as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        let switch_port = state
            .vpcs
            .create_segment(&link_id, node_ids)
            .await
            .map_err(runtime_api_error)?;
        let setup_result: Result<(), ApiError> = async {
            let endpoints = link["nodes"]
                .as_array_mut()
                .ok_or_else(|| ApiError::bad_request("link must contain a nodes array"))?;
            for (index, endpoint) in runtime_endpoints.iter().enumerate() {
                if endpoint["node_type"].as_str() != Some("dynamips") {
                    continue;
                }
                let node_id = endpoint["node_id"].as_str().unwrap_or_default();
                let row = sqlx::query(
                    "SELECT object_json FROM topology_objects
                     WHERE project_id = ? AND kind = 'node' AND object_id = ?",
                )
                .bind(&project_id)
                .bind(node_id)
                .fetch_one(state.store.pool())
                .await?;
                let node = decode_json(row.get::<String, _>("object_json"))?;
                let vm = state
                    .dynamips
                    .vm_from_node(&node)
                    .map_err(runtime_api_error)?;
                let local_port = state
                    .dynamips
                    .switch_endpoint(
                        &vm,
                        endpoint["adapter_number"].as_u64().unwrap_or_default() as u32,
                        endpoint["port_number"].as_u64().unwrap_or_default() as u32,
                        switch_port,
                    )
                    .await
                    .map_err(runtime_api_error)?;
                state
                    .vpcs
                    .register_external_endpoint(node_id, local_port)
                    .await;
                if let Some(endpoint_object) =
                    endpoints.get_mut(index).and_then(Value::as_object_mut)
                {
                    endpoint_object.insert("runtime_udp_port".into(), json!(local_port));
                }
            }
            Ok(())
        }
        .await;
        if let Err(error) = setup_result {
            for endpoint in &runtime_endpoints {
                let Some(node_id) = endpoint["node_id"].as_str() else {
                    continue;
                };
                state.vpcs.remove_external_endpoint(node_id, None).await;
                if endpoint["node_type"].as_str() == Some("dynamips")
                    && let Ok(row) = sqlx::query(
                        "SELECT object_json FROM topology_objects
                         WHERE project_id = ? AND kind = 'node' AND object_id = ?",
                    )
                    .bind(&project_id)
                    .bind(node_id)
                    .fetch_one(state.store.pool())
                    .await
                    && let Ok(node) = decode_json(row.get::<String, _>("object_json"))
                    && let Ok(vm) = state.dynamips.vm_from_node(&node)
                {
                    let _ = state.dynamips.remove_switch_endpoint(&vm).await;
                }
            }
            state.vpcs.remove_link(&link_id).await;
            return Err(error);
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
            if runtime_type == "udp_segment" {
                for endpoint in &runtime_endpoints {
                    let Some(node_id) = endpoint["node_id"].as_str() else {
                        continue;
                    };
                    state.vpcs.remove_external_endpoint(node_id, None).await;
                    if endpoint["node_type"].as_str() == Some("dynamips") {
                        if let Ok(row) = sqlx::query(
                            "SELECT object_json FROM topology_objects
                             WHERE project_id = ? AND kind = 'node' AND object_id = ?",
                        )
                        .bind(&project_id)
                        .bind(node_id)
                        .fetch_one(state.store.pool())
                        .await
                        {
                            if let Ok(node) = decode_json(row.get::<String, _>("object_json")) {
                                if let Ok(vm) = state.dynamips.vm_from_node(&node) {
                                    let _ = state.dynamips.remove_switch_endpoint(&vm).await;
                                }
                            }
                        }
                    }
                }
                state.vpcs.remove_link(&link_id).await;
            }
            Err(error)
        }
    }
}

async fn ensure_link_nodes_stopped(
    state: &AppState,
    project_id: &str,
    endpoints: &[Value],
) -> Result<(), ApiError> {
    for endpoint in endpoints {
        let node_id = endpoint["node_id"].as_str().unwrap_or_default();
        let row = sqlx::query(
            "SELECT object_json FROM topology_objects
             WHERE project_id = ? AND kind = 'node' AND object_id = ?",
        )
        .bind(project_id)
        .bind(node_id)
        .fetch_one(state.store.pool())
        .await?;
        let node = decode_json(row.get::<String, _>("object_json"))?;
        let status = match endpoint["node_type"].as_str().unwrap_or_default() {
            "docker" => {
                let id = endpoint["container_id"].as_str().unwrap_or_default();
                state.runtime.container_status(id).await
            }
            "qemu" => {
                let vm = state.qemu.vm_from_node(&node).map_err(runtime_api_error)?;
                state.qemu.status(&vm).await
            }
            "vpcs" => state.vpcs.status(node_id).await,
            "dynamips" => {
                let vm = state
                    .dynamips
                    .vm_from_node(&node)
                    .map_err(runtime_api_error)?;
                state.dynamips.status(&vm).await
            }
            node_type => {
                return Err(ApiError::bad_request(format!(
                    "Node type '{node_type}' cannot join a shared Ethernet segment"
                )));
            }
        }
        .map_err(runtime_api_error)?;
        if status != "stopped" {
            return Err(ApiError::conflict(format!(
                "Stop node {node_id} before changing its shared Ethernet segment"
            )));
        }
    }
    Ok(())
}

async fn create_shared_ethernet_link(
    state: &AppState,
    link_id: &str,
    link: &mut Value,
    runtime_endpoints: &[Value],
) -> Result<(), ApiError> {
    let segment = state
        .ubridge
        .create_segment(link_id)
        .await
        .map_err(runtime_api_error)?;
    let endpoints = link["nodes"]
        .as_array_mut()
        .ok_or_else(|| ApiError::bad_request("link must contain a nodes array"))?;
    let mut created_taps: Vec<String> = Vec::new();
    let setup = async {
        for (index, runtime_endpoint) in runtime_endpoints.iter().enumerate() {
            let endpoint = endpoints
                .get_mut(index)
                .and_then(Value::as_object_mut)
                .ok_or_else(|| ApiError::bad_request("each link endpoint must be an object"))?;
            let node_id = runtime_endpoint["node_id"].as_str().unwrap_or_default();
            let node_type = runtime_endpoint["node_type"].as_str().unwrap_or_default();
            let adapter = runtime_endpoint["adapter_number"]
                .as_u64()
                .unwrap_or_default();
            let port = runtime_endpoint["port_number"].as_u64().unwrap_or_default();
            let endpoint_id = shared_endpoint_id(link_id, node_id, adapter, port);
            endpoint.insert("runtime_ubridge_segment".into(), json!(segment));
            endpoint.insert("runtime_ubridge_endpoint".into(), json!(endpoint_id));

            if node_type == "docker" {
                let host = ubridge::host_veth_name(&endpoint_id).map_err(runtime_api_error)?;
                let guest = ubridge::guest_veth_name(&endpoint_id).map_err(runtime_api_error)?;
                endpoint.insert("runtime_host_veth".into(), json!(host));
                endpoint.insert("runtime_guest_veth".into(), json!(guest));
                endpoint.insert(
                    "runtime_guest_interface".into(),
                    // The current Docker node container owns eth0 on its
                    // private node network. Shared uBridge ports start at eth1.
                    json!(format!("eth{}", port + 1)),
                );
                continue;
            }

            let tap = ubridge::tap_name(&endpoint_id).map_err(runtime_api_error)?;
            state
                .ubridge
                .create_tap(&tap, &segment)
                .await
                .map_err(runtime_api_error)?;
            created_taps.push(tap.clone());
            endpoint.insert("runtime_tap".into(), json!(tap));

            if matches!(node_type, "vpcs" | "dynamips") {
                let bridge_port = state
                    .ubridge
                    .reserve_udp_port()
                    .await
                    .map_err(runtime_api_error)?;
                endpoint.insert("runtime_bridge_udp_port".into(), json!(bridge_port));
            }
        }
        Ok::<(), ApiError>(())
    }
    .await;
    if let Err(error) = setup {
        for tap in created_taps {
            let _ = state.ubridge.delete_tap(&tap, &segment).await;
        }
        let _ = state.ubridge.delete_segment(&segment).await;
        return Err(error);
    }
    Ok(())
}

async fn cleanup_shared_ethernet_link(
    state: &AppState,
    link_id: &str,
    link: &Value,
) -> Result<(), ApiError> {
    let segment = link["nodes"]
        .as_array()
        .and_then(|endpoints| {
            endpoints
                .iter()
                .find_map(|endpoint| endpoint["runtime_ubridge_segment"].as_str())
        })
        .map(str::to_owned)
        .unwrap_or(ubridge::segment_name(link_id).map_err(runtime_api_error)?);
    if let Some(endpoints) = link["nodes"].as_array() {
        for endpoint in endpoints {
            if let Some(node_id) = endpoint["node_id"].as_str() {
                state.vpcs.remove_bridge_port(node_id).await;
            }
            if let Some(endpoint_id) = endpoint["runtime_ubridge_endpoint"].as_str() {
                let _ = state.ubridge.delete_udp_tap_endpoint(endpoint_id).await;
            }
            if let Some(host) = endpoint["runtime_host_veth"].as_str() {
                let _ = state.ubridge.delete_veth(host).await;
            }
            if let Some(tap) = endpoint["runtime_tap"].as_str() {
                state
                    .ubridge
                    .delete_tap(tap, &segment)
                    .await
                    .map_err(runtime_api_error)?;
            }
        }
    }
    state
        .ubridge
        .delete_segment(&segment)
        .await
        .map_err(|error| {
            tracing::warn!(error = %error.message(), %link_id, "could not remove shared Ethernet bridge");
            runtime_api_error(error)
        })
}

fn shared_endpoint_id(link_id: &str, node_id: &str, adapter: u64, port: u64) -> String {
    let seed = format!("network-simulator/{link_id}/{node_id}/{adapter}/{port}");
    Uuid::new_v5(&Uuid::NAMESPACE_URL, seed.as_bytes()).to_string()
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
            if node_type == "vpcs" || node_type == "dynamips" {
                if adapter != 0 || port != 0 {
                    return Err(ApiError::bad_request(
                        "This Dynamips/VPCS runtime provides only Ethernet port 0/0",
                    ));
                }
            } else {
                return Err(ApiError::bad_request(format!(
                    "Node type '{node_type}' has no Ethernet runtime"
                )));
            }
        }
        if (node_type == "docker" && (adapter > 0 || port >= adapters))
            || (node_type == "qemu" && (adapter >= adapters || port != 0))
            || ((node_type == "vpcs" || node_type == "dynamips") && (adapter != 0 || port != 0))
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
            if matches!(
                action,
                NodeAction::Start | NodeAction::Stop | NodeAction::Reload
            ) {
                detach_docker_shared_links(state, project_id, node).await;
            }
            let status = state
                .runtime
                .lifecycle(container_id, action)
                .await
                .map_err(runtime_api_error)?;
            if matches!(action, NodeAction::Start | NodeAction::Reload) {
                if let Err(error) =
                    attach_docker_shared_links(state, project_id, node, container_id).await
                {
                    detach_docker_shared_links(state, project_id, node).await;
                    let _ = state
                        .runtime
                        .lifecycle(container_id, NodeAction::Stop)
                        .await;
                    return Err(error);
                }
            }
            Ok(status)
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
        "vpcs" => {
            let node_id = node
                .get("node_id")
                .and_then(Value::as_str)
                .ok_or_else(|| ApiError::conflict("VPCS node has no node ID"))?;
            let name = node
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| ApiError::conflict("VPCS node has no name"))?;
            if matches!(action, NodeAction::Start | NodeAction::Reload) {
                for (link_id, node_ids) in
                    vpcs_links_for_node(state.store.pool(), project_id, node_id).await?
                {
                    state.vpcs.set_link(&link_id, node_ids).await;
                }
            }
            let shared_links =
                shared_link_endpoints_for_node(state.store.pool(), project_id, node_id).await?;
            for (_link_id, endpoint) in &shared_links {
                if let Some(port) = endpoint["runtime_bridge_udp_port"]
                    .as_u64()
                    .and_then(|port| u16::try_from(port).ok())
                {
                    state.vpcs.set_bridge_port(node_id, port).await;
                }
            }
            if matches!(action, NodeAction::Stop | NodeAction::Reload) {
                for (_link_id, endpoint) in &shared_links {
                    if let Some(bridge_id) = endpoint["runtime_ubridge_endpoint"].as_str() {
                        let _ = state.ubridge.delete_udp_tap_endpoint(bridge_id).await;
                    }
                }
            }
            let status = state
                .vpcs
                .lifecycle(
                    project_id,
                    node_id,
                    name,
                    node.get("startup_script").and_then(Value::as_str),
                    action,
                )
                .await
                .map_err(runtime_api_error)?;
            if matches!(action, NodeAction::Start | NodeAction::Reload) {
                let Some(local_port) = state.vpcs.active_udp_port(node_id).await else {
                    return Err(ApiError::conflict(
                        "VPCS started without allocating its Ethernet UDP endpoint",
                    ));
                };
                for (_link_id, endpoint) in &shared_links {
                    let bridge_id = endpoint["runtime_ubridge_endpoint"]
                        .as_str()
                        .unwrap_or_default();
                    let tap = endpoint["runtime_tap"].as_str().unwrap_or_default();
                    let switch_port = endpoint["runtime_bridge_udp_port"]
                        .as_u64()
                        .and_then(|port| u16::try_from(port).ok())
                        .ok_or_else(|| ApiError::conflict("VPCS link has no bridge UDP port"))?;
                    if let Err(error) = state
                        .ubridge
                        .attach_udp_to_tap_at(bridge_id, tap, switch_port, local_port)
                        .await
                    {
                        let _ = state
                            .vpcs
                            .lifecycle(
                                project_id,
                                node_id,
                                name,
                                node.get("startup_script").and_then(Value::as_str),
                                NodeAction::Stop,
                            )
                            .await;
                        for (_link_id, endpoint) in &shared_links {
                            if let Some(bridge_id) = endpoint["runtime_ubridge_endpoint"].as_str() {
                                let _ = state.ubridge.delete_udp_tap_endpoint(bridge_id).await;
                            }
                        }
                        return Err(runtime_api_error(error));
                    }
                }
            }
            Ok(status)
        }
        "dynamips" => {
            let vm = state
                .dynamips
                .vm_from_node(node)
                .map_err(runtime_api_error)?;
            if matches!(action, NodeAction::Stop | NodeAction::Reload) {
                for (_link_id, endpoint) in
                    shared_link_endpoints_for_node(state.store.pool(), project_id, &vm.node_id)
                        .await?
                {
                    if let Some(bridge_id) = endpoint["runtime_ubridge_endpoint"].as_str() {
                        let _ = state.ubridge.delete_udp_tap_endpoint(bridge_id).await;
                    }
                }
            }
            if matches!(action, NodeAction::Start | NodeAction::Reload) {
                attach_dynamips_links_for_node(state, project_id, &vm).await?;
            }
            state
                .dynamips
                .lifecycle(&vm, action)
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
    if node_type != "docker"
        && node_type != "qemu"
        && node_type != "vpcs"
        && node_type != "dynamips"
    {
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
        "vpcs" => state
            .vpcs
            .remove_node(
                node.get("project_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                node_id,
            )
            .await
            .map_err(runtime_api_error),
        "dynamips" => {
            let vm = state
                .dynamips
                .vm_from_node(node)
                .map_err(runtime_api_error)?;
            state
                .dynamips
                .remove_router(&vm)
                .await
                .map_err(runtime_api_error)
        }
        _ => Ok(()),
    }
}

async fn qemu_links_for_node(
    pool: &SqlitePool,
    project_id: &str,
    node_id: &str,
) -> Result<Vec<QemuLink>, ApiError> {
    // QEMU enumerates guest NICs in device argument order; keep that order
    // aligned with the topology's adapter numbers regardless of SQLite row order.
    let rows = sqlx::query(
        "SELECT o.object_id, o.object_json
         FROM topology_objects AS o
         JOIN link_endpoints AS e
           ON e.project_id = o.project_id AND e.link_id = o.object_id
         WHERE o.project_id = ? AND o.kind = 'link' AND e.node_id = ?
         ORDER BY e.adapter_number, o.object_id",
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
        let Some(endpoint) = endpoints
            .iter()
            .find(|endpoint| endpoint["node_id"].as_str() == Some(node_id))
        else {
            continue;
        };
        // Older persisted links used a pair of unicast UDP ports. Derive the
        // multicast bus from their stable link ID when upgrading those records.
        let (fallback_address, fallback_port) =
            QemuRuntime::multicast_endpoint(&link_id).map_err(runtime_api_error)?;
        let multicast_address = endpoint["runtime_multicast_address"]
            .as_str()
            .and_then(|address| address.parse().ok())
            .unwrap_or(fallback_address);
        let multicast_port = endpoint["runtime_multicast_port"]
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .unwrap_or(fallback_port);
        let adapter_number = endpoint["adapter_number"]
            .as_u64()
            .and_then(|adapter| u32::try_from(adapter).ok())
            .ok_or_else(|| {
                ApiError::conflict(format!("QEMU link {link_id} has an invalid adapter"))
            })?;
        links.push(QemuLink {
            link_id,
            adapter_number,
            multicast_address,
            multicast_port,
            tap: endpoint["runtime_tap"].as_str().map(str::to_owned),
        });
    }
    Ok(links)
}

async fn vpcs_links_for_node(
    pool: &SqlitePool,
    project_id: &str,
    node_id: &str,
) -> Result<Vec<(String, Vec<String>)>, ApiError> {
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
    rows.into_iter()
        .map(|row| {
            let link_id: String = row.get("object_id");
            let link = decode_json(row.get::<String, _>("object_json"))?;
            let node_ids = link
                .get("nodes")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|endpoint| {
                    endpoint
                        .get("node_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .collect();
            Ok((link_id, node_ids))
        })
        .collect()
}

async fn shared_link_endpoints_for_node(
    pool: &SqlitePool,
    project_id: &str,
    node_id: &str,
) -> Result<Vec<(String, Value)>, ApiError> {
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
    let mut shared = Vec::new();
    for row in rows {
        let link_id: String = row.get("object_id");
        let link = decode_json(row.get::<String, _>("object_json"))?;
        let Some(endpoint) = link
            .get("nodes")
            .and_then(Value::as_array)
            .and_then(|endpoints| {
                endpoints.iter().find(|endpoint| {
                    endpoint["node_id"].as_str() == Some(node_id)
                        && endpoint.get("runtime_ubridge_segment").is_some()
                })
            })
        else {
            continue;
        };
        shared.push((link_id, endpoint.clone()));
    }
    Ok(shared)
}

async fn attach_docker_shared_links(
    state: &AppState,
    project_id: &str,
    node: &Value,
    container_id: &str,
) -> Result<(), ApiError> {
    let node_id = node["node_id"].as_str().unwrap_or_default();
    let links = shared_link_endpoints_for_node(state.store.pool(), project_id, node_id).await?;
    if links.is_empty() {
        return Ok(());
    }
    let namespace_pid = state
        .runtime
        .container_pid(container_id)
        .await
        .map_err(runtime_api_error)?;
    let mut attached_hosts: Vec<String> = Vec::new();
    for (_link_id, endpoint) in links {
        let segment = endpoint["runtime_ubridge_segment"]
            .as_str()
            .unwrap_or_default();
        let host = endpoint["runtime_host_veth"].as_str().unwrap_or_default();
        let guest = endpoint["runtime_guest_veth"].as_str().unwrap_or_default();
        let guest_name = endpoint["runtime_guest_interface"]
            .as_str()
            .unwrap_or("eth1");
        let result = async {
            state.ubridge.create_veth(host, guest, segment).await?;
            state.ubridge.set_interface_up(guest).await?;
            state
                .ubridge
                .move_veth_to_container(guest, namespace_pid, guest_name)
                .await
        }
        .await;
        if let Err(error) = result {
            let _ = state.ubridge.delete_veth(host).await;
            for previous_host in attached_hosts {
                let _ = state.ubridge.delete_veth(&previous_host).await;
            }
            return Err(runtime_api_error(error));
        }
        attached_hosts.push(host.to_owned());
    }
    Ok(())
}

async fn detach_docker_shared_links(state: &AppState, project_id: &str, node: &Value) {
    let node_id = node["node_id"].as_str().unwrap_or_default();
    match shared_link_endpoints_for_node(state.store.pool(), project_id, node_id).await {
        Ok(links) => {
            for (_link_id, endpoint) in links {
                if let Some(host) = endpoint["runtime_host_veth"].as_str() {
                    if let Err(error) = state.ubridge.delete_veth(host).await {
                        tracing::debug!(error = %error.message(), %node_id, %host, "Docker shared-link veth was already absent or could not be removed");
                    }
                }
            }
        }
        Err(error) => {
            tracing::error!(error = %error.message, %node_id, "could not load Docker shared-link endpoints for cleanup")
        }
    }
}

async fn attach_dynamips_links_for_node(
    state: &AppState,
    project_id: &str,
    vm: &DynamipsVm,
) -> Result<(), ApiError> {
    let shared_links =
        shared_link_endpoints_for_node(state.store.pool(), project_id, &vm.node_id).await?;
    let shared_link_ids = shared_links
        .iter()
        .map(|(link_id, _)| link_id.clone())
        .collect::<HashSet<_>>();
    for (_link_id, endpoint) in shared_links {
        let bridge_port = endpoint["runtime_bridge_udp_port"]
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .ok_or_else(|| ApiError::conflict("Dynamips link has no bridge UDP port"))?;
        let endpoint_id = endpoint["runtime_ubridge_endpoint"]
            .as_str()
            .unwrap_or_default();
        let tap = endpoint["runtime_tap"].as_str().unwrap_or_default();
        let adapter = endpoint["adapter_number"].as_u64().unwrap_or_default() as u32;
        let port = endpoint["port_number"].as_u64().unwrap_or_default() as u32;
        let local_port = state
            .dynamips
            .switch_endpoint(vm, adapter, port, bridge_port)
            .await
            .map_err(runtime_api_error)?;
        state
            .ubridge
            .attach_udp_to_tap_at(endpoint_id, tap, bridge_port, local_port)
            .await
            .map_err(runtime_api_error)?;
    }
    for (link_id, node_ids) in
        vpcs_links_for_node(state.store.pool(), project_id, &vm.node_id).await?
    {
        if shared_link_ids.contains(&link_id) {
            continue;
        }
        let switch_port = state
            .vpcs
            .create_segment(&link_id, node_ids)
            .await
            .map_err(runtime_api_error)?;
        let local_port = state
            .dynamips
            .switch_endpoint(vm, 0, 0, switch_port)
            .await
            .map_err(runtime_api_error)?;
        state
            .vpcs
            .register_external_endpoint(&vm.node_id, local_port)
            .await;
    }
    Ok(())
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
                    "Stop all QEMU nodes attached to this Ethernet segment before changing it",
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
    let shared_ethernet = endpoints
        .iter()
        .any(|endpoint| endpoint.get("runtime_ubridge_segment").is_some());
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
    if shared_ethernet {
        let link_id = link
            .get("link_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        cleanup_shared_ethernet_link(state, link_id, link).await?;
    }
    if node_types.contains("docker") && !shared_ethernet {
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
    if node_types.contains("vpcs") {
        // The shared UDP switch is owned by VPCS even when the segment also
        // contains IOS routers. Remove the Dynamips NIO before releasing it.
    }
    if node_types.contains("vpcs") || node_types.contains("dynamips") {
        if let Some(endpoints) = link.get("nodes").and_then(Value::as_array) {
            for endpoint in endpoints {
                let Some(node_id) = endpoint["node_id"].as_str() else {
                    continue;
                };
                state.vpcs.remove_external_endpoint(node_id, None).await;
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
                if node.get("node_type").and_then(Value::as_str) == Some("dynamips") {
                    let vm = state
                        .dynamips
                        .vm_from_node(&node)
                        .map_err(runtime_api_error)?;
                    state
                        .dynamips
                        .remove_switch_endpoint(&vm)
                        .await
                        .map_err(runtime_api_error)?;
                }
            }
        }
        if let Some(link_id) = link.get("link_id").and_then(Value::as_str) {
            state.vpcs.remove_link(link_id).await;
        }
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
    if endpoints.len() < 2 {
        return Err(ApiError::bad_request(
            "link must have at least two endpoints",
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

async fn qemu_console(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let project_id = project_id.to_string();
    let node_id = node_id.to_string();
    let node = get_topology_object(state.store.pool(), &project_id, "node", &node_id)
        .await?
        .0;
    if node["node_type"].as_str() != Some("qemu") {
        return Err(ApiError::conflict(
            "Serial consoles are available for QEMU nodes only",
        ));
    }
    let vm = state.qemu.vm_from_node(&node).map_err(runtime_api_error)?;
    if state.qemu.status(&vm).await.map_err(runtime_api_error)? != "started" {
        return Err(ApiError::conflict(
            "Start the QEMU node before opening its console",
        ));
    }
    let serial = UnixStream::connect(&vm.serial_socket)
        .await
        .map_err(|error| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("Could not connect to the QEMU serial console: {error}"),
            )
        })?;
    Ok(websocket.on_upgrade(move |socket| qemu_console_session(socket, serial)))
}

async fn docker_console(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let project_id = project_id.to_string();
    let node_id = node_id.to_string();
    let node = get_topology_object(state.store.pool(), &project_id, "node", &node_id)
        .await?
        .0;
    if node["node_type"].as_str() != Some("docker") {
        return Err(ApiError::conflict(
            "Attached Docker consoles are available for Docker nodes only",
        ));
    }
    let container_id = node["container_id"]
        .as_str()
        .ok_or_else(|| ApiError::conflict("Docker node has no runtime container"))?;
    if state
        .runtime
        .container_status(container_id)
        .await
        .map_err(runtime_api_error)?
        != "started"
    {
        return Err(ApiError::conflict(
            "Start the Docker node before opening its console",
        ));
    }
    let process = state
        .runtime
        .attach_process(container_id)
        .map_err(runtime_api_error)?;
    Ok(websocket.on_upgrade(move |socket| docker_console_session(socket, process)))
}

async fn vpcs_console(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let project_id = project_id.to_string();
    let node_id = node_id.to_string();
    let node = get_topology_object(state.store.pool(), &project_id, "node", &node_id)
        .await?
        .0;
    if node["node_type"].as_str() != Some("vpcs") {
        return Err(ApiError::conflict(
            "VPCS consoles are available for VPCS nodes only",
        ));
    }
    if state
        .vpcs
        .status(&node_id)
        .await
        .map_err(runtime_api_error)?
        != "started"
    {
        return Err(ApiError::conflict(
            "Start the VPCS node before opening its console",
        ));
    }
    let console = state
        .vpcs
        .connect_console(&node_id)
        .await
        .map_err(runtime_api_error)?;
    Ok(websocket.on_upgrade(move |socket| vpcs_console_session(socket, console)))
}

async fn dynamips_console(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
    websocket: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let project_id = project_id.to_string();
    let node_id = node_id.to_string();
    let node = get_topology_object(state.store.pool(), &project_id, "node", &node_id)
        .await?
        .0;
    if node["node_type"].as_str() != Some("dynamips") {
        return Err(ApiError::conflict(
            "Console access is available for Dynamips IOS routers only",
        ));
    }
    let vm = state
        .dynamips
        .vm_from_node(&node)
        .map_err(runtime_api_error)?;
    let console = state
        .dynamips
        .console(&vm)
        .await
        .map_err(runtime_api_error)?;
    Ok(websocket.on_upgrade(move |socket| vpcs_console_session(socket, console)))
}

async fn qemu_console_session(socket: WebSocket, serial: UnixStream) {
    let (mut websocket_sink, mut websocket_stream) = socket.split();
    let (mut serial_reader, mut serial_writer) = tokio::io::split(serial);
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        tokio::select! {
            incoming = websocket_stream.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    if serial_writer.write_all(text.as_bytes()).await.is_err() { break; }
                }
                Some(Ok(Message::Binary(bytes))) => {
                    if serial_writer.write_all(&bytes).await.is_err() { break; }
                }
                Some(Ok(Message::Ping(payload))) => {
                    if websocket_sink.send(Message::Pong(payload)).await.is_err() { break; }
                }
                Some(Ok(Message::Pong(_))) => {}
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
            },
            received = serial_reader.read(&mut buffer) => match received {
                Ok(0) | Err(_) => break,
                Ok(length) => {
                    if websocket_sink.send(Message::Binary(buffer[..length].to_vec().into())).await.is_err() { break; }
                }
            }
        }
    }
    let _ = websocket_sink.close().await;
}

async fn vpcs_console_session(socket: WebSocket, console: TcpStream) {
    let (mut websocket_sink, mut websocket_stream) = socket.split();
    let (mut console_reader, mut console_writer) = tokio::io::split(console);
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        tokio::select! {
            incoming = websocket_stream.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    if console_writer.write_all(text.as_bytes()).await.is_err() { break; }
                }
                Some(Ok(Message::Binary(bytes))) => {
                    if console_writer.write_all(&bytes).await.is_err() { break; }
                }
                Some(Ok(Message::Ping(payload))) => {
                    if websocket_sink.send(Message::Pong(payload)).await.is_err() { break; }
                }
                Some(Ok(Message::Pong(_))) => {}
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
            },
            received = console_reader.read(&mut buffer) => match received {
                Ok(0) | Err(_) => break,
                Ok(length) => {
                    if websocket_sink.send(Message::Binary(buffer[..length].to_vec().into())).await.is_err() { break; }
                }
            }
        }
    }
    let _ = websocket_sink.close().await;
}

async fn docker_console_session(socket: WebSocket, mut process: tokio::process::Child) {
    let (Some(mut stdin), Some(mut stdout), Some(mut stderr)) = (
        process.stdin.take(),
        process.stdout.take(),
        process.stderr.take(),
    ) else {
        return;
    };
    let (mut websocket_sink, mut websocket_stream) = socket.split();
    let mut stdout_open = true;
    let mut stderr_open = true;
    let mut stdout_buffer = [0_u8; 16 * 1024];
    let mut stderr_buffer = [0_u8; 16 * 1024];

    loop {
        tokio::select! {
            incoming = websocket_stream.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    if stdin.write_all(text.as_bytes()).await.is_err() { break; }
                }
                Some(Ok(Message::Binary(bytes))) => {
                    if stdin.write_all(&bytes).await.is_err() { break; }
                }
                Some(Ok(Message::Ping(payload))) => {
                    if websocket_sink.send(Message::Pong(payload)).await.is_err() { break; }
                }
                Some(Ok(Message::Pong(_))) => {}
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
            },
            received = stdout.read(&mut stdout_buffer), if stdout_open => match received {
                Ok(0) | Err(_) => stdout_open = false,
                Ok(length) => {
                    if websocket_sink.send(Message::Binary(stdout_buffer[..length].to_vec().into())).await.is_err() { break; }
                }
            },
            received = stderr.read(&mut stderr_buffer), if stderr_open => match received {
                Ok(0) | Err(_) => stderr_open = false,
                Ok(length) => {
                    if websocket_sink.send(Message::Binary(stderr_buffer[..length].to_vec().into())).await.is_err() { break; }
                }
            },
            _ = process.wait(), if !stdout_open && !stderr_open => break,
        }
    }
    let _ = stdin.write_all(&[0x10, 0x11]).await;
    let _ = stdin.flush().await;
    if tokio::time::timeout(std::time::Duration::from_millis(500), process.wait())
        .await
        .is_err()
    {
        drop(stdin);
        let _ = process.kill().await;
        let _ = process.wait().await;
    }
    let _ = websocket_sink.close().await;
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
    let mut dynamips_queries = Vec::new();
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
            Some("vpcs") => match state.vpcs.status(node_id).await {
                Ok(status) => {
                    node["status"] = json!(status);
                    node["runtime_missing"] = json!(false);
                }
                Err(error) => {
                    tracing::debug!(error = %error.message(), %project_id, %node_id, "VPCS node state refresh was unavailable")
                }
            },
            Some("dynamips") => match state.dynamips.vm_from_node(node) {
                Ok(vm) => dynamips_queries.push((index, node_id.to_owned(), vm)),
                Err(error) => {
                    tracing::debug!(error = %error.message(), %project_id, %node_id, "Dynamips node metadata is invalid")
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
    let dynamips_results =
        futures_util::stream::iter(dynamips_queries)
            .map(|(index, node_id, vm)| async move {
                (index, node_id, state.dynamips.status(&vm).await)
            })
            .buffer_unordered(16)
            .collect::<Vec<_>>()
            .await;
    for (index, node_id, result) in dynamips_results {
        match result {
            Ok(status) => {
                nodes[index]["status"] = json!(status);
                nodes[index]["runtime_missing"] = json!(false);
            }
            Err(error) => {
                nodes[index]["status"] = json!("stopped");
                nodes[index]["runtime_missing"] = json!(false);
                tracing::debug!(error = %error.message(), %project_id, %node_id, "Dynamips state refresh was unavailable")
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
    use crate::{
        dynamips::DynamipsRuntime, qemu::QemuRuntime, runtime::DockerRuntime, vpcs::VpcsRuntime,
    };

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
        let script = script.replace(
            "  ps) exit 0 ;;",
            "  attach) printf 'DOCKER-ATTACHED\\n'; cat ;;\n  ps) exit 0 ;;",
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
                VpcsRuntime::with_paths(
                    std::env::var_os("NETWORK_SIMULATOR_TEST_VPCS")
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(|| directory.path().join("vpcs-test-double-not-installed")),
                    directory.path().join("vpcs-state"),
                ),
                DynamipsRuntime::with_paths(
                    directory.path().join("dynamips-test-double-not-installed"),
                    directory.path().join("dynamips-images"),
                    directory.path().join("dynamips-state"),
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

    async fn websocket_status(app: &axum::Router, uri: String) -> StatusCode {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::{TcpListener, TcpStream},
            time::{Duration, timeout},
        };

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("temporary HTTP listener binds");
        let address = listener.local_addr().expect("HTTP listener has an address");
        let server_app = app.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, server_app)
                .await
                .expect("test HTTP server runs");
        });
        let mut client = TcpStream::connect(address)
            .await
            .expect("WebSocket client connects");
        client
            .write_all(
                format!(
                    "GET {uri} HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .expect("WebSocket upgrade request writes");
        let mut response = [0_u8; 512];
        let length = timeout(Duration::from_secs(2), client.read(&mut response))
            .await
            .expect("HTTP response arrives before timeout")
            .expect("HTTP response reads");
        server.abort();
        let first_line = String::from_utf8_lossy(&response[..length]);
        first_line
            .split_whitespace()
            .nth(1)
            .and_then(|value| value.parse().ok())
            .and_then(|status| StatusCode::from_u16(status).ok())
            .expect("HTTP response contains a status code")
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
        let template = templates
            .as_array()
            .unwrap()
            .iter()
            .find(|template| template["template_type"] == "docker")
            .expect("local Docker image template is listed");
        let template = template["template_id"].as_str().expect("template UUID");

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
    #[ignore = "requires the upstream VPCS executable; set NETWORK_SIMULATOR_TEST_VPCS and run with --ignored"]
    async fn vpcs_template_creates_nodes_and_shared_segments_through_the_controller() {
        let (app, _directory) = test_router().await;
        let (status, templates) = request(&app, Method::GET, "/v3/templates".into(), None).await;
        assert_eq!(status, StatusCode::OK);
        let template = templates
            .as_array()
            .unwrap()
            .iter()
            .find(|template| template["template_type"] == "vpcs")
            .expect("VPCS template is listed when its executable is configured");
        let template_id = template["template_id"].as_str().unwrap();
        let (_, project) = request(
            &app,
            Method::POST,
            "/v3/projects".into(),
            Some(serde_json::json!({ "name": "VPCS lab" })),
        )
        .await;
        let project_id = project["project_id"].as_str().unwrap();

        let mut nodes = Vec::new();
        for (name, address) in [("PC A", "10.0.0.1/24"), ("PC B", "10.0.0.2/24")] {
            let (status, node) = request(
                &app,
                Method::POST,
                format!("/v3/projects/{project_id}/templates/{template_id}"),
                Some(serde_json::json!({
                    "name": name,
                    "startup_script": format!("ip {address}")
                })),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED);
            assert_eq!(node["node_type"], "vpcs");
            assert_eq!(node["adapters"], 1);
            nodes.push(node);
        }

        let (status, link) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/links"),
            Some(serde_json::json!({
                "nodes": nodes.iter().map(|node| serde_json::json!({
                    "node_id": node["node_id"],
                    "adapter_number": 0,
                    "port_number": 0
                })).collect::<Vec<_>>()
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(link["nodes"].as_array().unwrap().len(), 2);

        for node in &nodes {
            let (status, started) = request(
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
            assert_eq!(started["status"], "started");
        }

        let console_path = format!(
            "/v3/compute/projects/{project_id}/vpcs/nodes/{}/console/ws",
            nodes[0]["node_id"].as_str().unwrap()
        );
        use futures_util::{SinkExt, StreamExt};
        use tokio::{
            net::TcpListener,
            time::{Duration, timeout},
        };
        use tokio_tungstenite::{connect_async, tungstenite::Message};
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("temporary VPCS console listener binds");
        let address = listener.local_addr().unwrap();
        let server_app = app.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, server_app)
                .await
                .expect("VPCS console HTTP server runs");
        });
        let (mut console, _) = connect_async(format!("ws://{address}{console_path}"))
            .await
            .expect("VPCS console WebSocket connects");
        let mut startup_output = String::new();
        timeout(Duration::from_secs(5), async {
            loop {
                let Some(Ok(Message::Binary(bytes))) = console.next().await else {
                    panic!("VPCS console closed before its prompt");
                };
                startup_output.push_str(&String::from_utf8_lossy(&bytes));
                if startup_output.contains("VPCS>") {
                    break;
                }
            }
        })
        .await
        .expect("VPCS startup script reaches the browser console");
        console
            .send(Message::Text("ping 10.0.0.2 -c 1\r".into()))
            .await
            .expect("browser console sends a VPCS command");
        let mut ping_output = String::new();
        timeout(Duration::from_secs(12), async {
            loop {
                let Some(Ok(Message::Binary(bytes))) = console.next().await else {
                    panic!("VPCS console closed while ping was running");
                };
                ping_output.push_str(&String::from_utf8_lossy(&bytes));
                if ping_output.contains("bytes from 10.0.0.2") {
                    break;
                }
            }
        })
        .await
        .expect("VPCS ping response reaches the browser console");
        assert!(
            ping_output.contains("bytes from 10.0.0.2"),
            "unexpected output: {ping_output}"
        );
        console
            .send(Message::Close(None))
            .await
            .expect("browser console closes cleanly");
        server.abort();

        for node in &nodes {
            let (status, stopped) = request(
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
            assert_eq!(stopped["status"], "stopped");
        }
    }

    #[tokio::test]
    #[ignore = "requires Docker, alpine:latest, upstream VPCS, and capable upstream uBridge"]
    async fn alpine_and_vpcs_exchange_icmp_over_a_real_mixed_link_through_the_controller() {
        use std::{env, path::PathBuf};
        use tokio::process::Command;

        use crate::runtime::DockerRuntime;

        let directory = TempDir::new().expect("temporary mixed-runtime API state");
        let store = Store::connect(
            directory
                .path()
                .join("controller.sqlite")
                .to_str()
                .expect("temporary database path is UTF-8"),
        )
        .await
        .expect("test database connects");
        let vpcs_binary = env::var_os("NETWORK_SIMULATOR_TEST_VPCS")
            .expect("NETWORK_SIMULATOR_TEST_VPCS names upstream VPCS");
        let app = router(AppState::with_runtimes(
            store,
            DockerRuntime::new(),
            crate::qemu::QemuRuntime::new(),
            VpcsRuntime::with_paths(vpcs_binary, directory.path().join("vpcs-state")),
            DynamipsRuntime::new(),
        ));
        let mut project_id: Option<String> = None;

        let run: Result<(), String> = async {
            let (status, project) = request(
                &app,
                Method::POST,
                "/v3/projects".into(),
                Some(serde_json::json!({ "name": "Live Docker VPCS link" })),
            )
            .await;
            if status != StatusCode::CREATED {
                return Err(format!("project create returned {status}: {project}"));
            }
            let current_project_id = project["project_id"]
                .as_str()
                .ok_or_else(|| "created project has no ID".to_owned())?
                .to_owned();
            project_id = Some(current_project_id.clone());

            let (status, docker_node) = request(
                &app,
                Method::POST,
                format!("/v3/projects/{current_project_id}/nodes"),
                Some(serde_json::json!({
                    "name": "Alpine peer",
                    "node_type": "docker",
                    "image": "alpine:latest",
                    "start_command": "/bin/sh -c 'sleep 3600'",
                    "adapters": 1
                })),
            )
            .await;
            if status != StatusCode::CREATED {
                return Err(format!(
                    "Docker node create returned {status}: {docker_node}"
                ));
            }

            let (status, vpcs_node) = request(
                &app,
                Method::POST,
                format!("/v3/projects/{current_project_id}/nodes"),
                Some(serde_json::json!({
                    "name": "VPCS peer",
                    "node_type": "vpcs",
                    "startup_script": "ip 10.0.0.1/24"
                })),
            )
            .await;
            if status != StatusCode::CREATED {
                return Err(format!("VPCS node create returned {status}: {vpcs_node}"));
            }

            let endpoints = serde_json::json!([
                { "node_id": docker_node["node_id"], "adapter_number": 0, "port_number": 0 },
                { "node_id": vpcs_node["node_id"], "adapter_number": 0, "port_number": 0 }
            ]);
            let (status, link) = request(
                &app,
                Method::POST,
                format!("/v3/projects/{current_project_id}/links"),
                Some(serde_json::json!({ "nodes": endpoints })),
            )
            .await;
            if status != StatusCode::CREATED
                || link["nodes"][0]["runtime_ubridge_segment"]
                    .as_str()
                    .is_none()
            {
                return Err(format!("mixed link create returned {status}: {link}"));
            }

            for node in [&vpcs_node, &docker_node] {
                let node_id = node["node_id"]
                    .as_str()
                    .ok_or_else(|| "created node has no ID".to_owned())?;
                let (status, started) = request(
                    &app,
                    Method::POST,
                    format!("/v3/projects/{current_project_id}/nodes/{node_id}/start"),
                    Some(serde_json::json!({})),
                )
                .await;
                if status != StatusCode::OK || started["status"] != "started" {
                    return Err(format!("node start returned {status}: {started}"));
                }
            }

            let container_id = docker_node["container_id"]
                .as_str()
                .ok_or_else(|| "Docker node has no container ID".to_owned())?;
            let docker_binary = env::var_os("NETWORK_SIMULATOR_DOCKER")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("docker"));
            for args in [
                vec!["ip", "addr", "add", "10.0.0.2/24", "dev", "eth1"],
                vec!["ip", "link", "set", "eth1", "up"],
            ] {
                let output = Command::new(&docker_binary)
                    .args(["exec", container_id])
                    .args(args)
                    .output()
                    .await
                    .map_err(|error| format!("could not configure Alpine eth1: {error}"))?;
                if !output.status.success() {
                    return Err(format!(
                        "could not configure Alpine eth1: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    ));
                }
            }

            let output = Command::new(docker_binary)
                .args([
                    "exec",
                    container_id,
                    "ping",
                    "-c",
                    "1",
                    "-W",
                    "3",
                    "10.0.0.1",
                ])
                .output()
                .await
                .map_err(|error| format!("could not run Alpine-to-VPCS ping: {error}"))?;
            if !output.status.success()
                || !String::from_utf8_lossy(&output.stdout).contains("1 packets received")
            {
                return Err(format!(
                    "Alpine-to-VPCS ICMP failed: {}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            Ok(())
        }
        .await;

        let cleanup = if let Some(project_id) = project_id {
            let (status, body) = request(
                &app,
                Method::DELETE,
                format!("/v3/projects/{project_id}"),
                None,
            )
            .await;
            (status == StatusCode::NO_CONTENT)
                .then_some(())
                .ok_or_else(|| format!("temporary project cleanup returned {status}: {body}"))
        } else {
            Ok(())
        };
        assert!(run.is_ok(), "live mixed-runtime API test failed: {run:?}");
        assert!(
            cleanup.is_ok(),
            "live mixed-runtime cleanup failed: {cleanup:?}"
        );
    }

    #[tokio::test]
    async fn docker_console_attaches_to_the_running_container_tty_over_websocket() {
        use futures_util::{SinkExt, StreamExt};
        use tokio::{
            net::TcpListener,
            time::{Duration, timeout},
        };
        use tokio_tungstenite::{connect_async, tungstenite::Message};

        let (app, _directory) = test_router().await;
        let (_, project) = request(
            &app,
            Method::POST,
            "/v3/projects".into(),
            Some(serde_json::json!({ "name": "Docker console lab" })),
        )
        .await;
        let project_id = project["project_id"].as_str().unwrap();
        let (status, node) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes"),
            Some(serde_json::json!({
                "name": "Interactive node",
                "node_type": "docker",
                "image": "alpine:latest"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let node_id = node["node_id"].as_str().unwrap();
        let console_path =
            format!("/v3/compute/projects/{project_id}/docker/nodes/{node_id}/console/ws");
        assert_eq!(
            websocket_status(&app, console_path.clone()).await,
            StatusCode::CONFLICT
        );

        let (status, _) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes/{node_id}/start"),
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("temporary HTTP listener binds");
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("test HTTP server runs");
        });
        let (mut socket, _) = connect_async(format!("ws://{address}{console_path}"))
            .await
            .expect("Docker console websocket connects");
        let initial = timeout(Duration::from_secs(2), socket.next())
            .await
            .expect("Docker attach produces initial terminal output")
            .expect("Docker console remains open")
            .expect("Docker console output is valid")
            .into_data();
        assert!(String::from_utf8_lossy(&initial).contains("DOCKER-ATTACHED"));

        socket
            .send(Message::Text("show interfaces\r\n".into()))
            .await
            .expect("terminal input reaches the attached process");
        let echoed = timeout(Duration::from_secs(2), socket.next())
            .await
            .expect("attached process echoes terminal input")
            .expect("Docker console remains open")
            .expect("Docker console input echo is valid")
            .into_data();
        assert!(String::from_utf8_lossy(&echoed).contains("show interfaces"));
        socket
            .close(None)
            .await
            .expect("Docker console closes cleanly");
        server.abort();
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
        for name in ["Router A", "Router B", "Router C"] {
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

        let console_path = format!(
            "/v3/compute/projects/{project_id}/qemu/nodes/{}/console/ws",
            qemu_nodes[0]["node_id"].as_str().unwrap()
        );
        assert_eq!(
            websocket_status(&app, console_path.clone()).await,
            StatusCode::CONFLICT
        );

        let (status, link) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/links"),
            Some(serde_json::json!({
                "nodes": [
                    { "node_id": qemu_nodes[0]["node_id"], "adapter_number": 0, "port_number": 0 },
                    { "node_id": qemu_nodes[1]["node_id"], "adapter_number": 1, "port_number": 0 },
                    { "node_id": qemu_nodes[2]["node_id"], "adapter_number": 0, "port_number": 0 }
                ]
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let multicast_address = link["nodes"][0]["runtime_multicast_address"]
            .as_str()
            .unwrap();
        let multicast_port = link["nodes"][0]["runtime_multicast_port"].as_u64().unwrap();
        assert!(link["nodes"].as_array().unwrap().iter().all(|endpoint| {
            endpoint["runtime_multicast_address"] == multicast_address
                && endpoint["runtime_multicast_port"] == multicast_port
        }));

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
        assert_eq!(
            websocket_status(&app, console_path).await,
            StatusCode::SERVICE_UNAVAILABLE
        );
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
        let (status, node) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes/{node_b}/start"),
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(node["status"], "started");
        let node_c = qemu_nodes[2]["node_id"].as_str().unwrap();
        let (status, node) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes/{node_c}/start"),
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(node["status"], "started");
        let args = std::fs::read_to_string(directory.path().join("qemu-arguments.log"))
            .expect("QEMU launch arguments were recorded");
        let netdev = format!(
            "socket,id=ns_{}",
            link["link_id"].as_str().unwrap().replace('-', "")
        );
        assert!(args.contains(&netdev));
        assert!(args.contains(&format!("mcast={multicast_address}:{multicast_port}")));
        assert!(args.contains("-chardev socket,id=serial0,path="));
        assert!(args.contains("server=on,wait=off -serial chardev:serial0"));
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
                .contains("Stop all QEMU nodes")
        );

        for node_id in [node_a, node_b, node_c] {
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
    async fn qemu_image_upload_is_installed_in_the_shared_runtime_library() {
        let (app, directory) = test_router().await;
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v3/qemu/images/custom-router.qcow2")
                    .header("content-type", "application/octet-stream")
                    .body(Body::from("test disk bytes"))
                    .expect("QEMU upload request builds"),
            )
            .await
            .expect("QEMU image upload response");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("QEMU upload response body")
            .to_bytes();
        let uploaded: Value = serde_json::from_slice(&body).expect("QEMU upload response JSON");
        assert_eq!(uploaded["name"], "custom-router.qcow2");
        assert_eq!(uploaded["template_type"], "qemu");
        assert_eq!(uploaded["bytes"], 15);
        let image_path = std::path::PathBuf::from(uploaded["image"].as_str().unwrap());
        assert!(image_path.starts_with(directory.path().join("qemu-images")));
        assert_eq!(std::fs::read(&image_path).unwrap(), b"test disk bytes");

        let (status, templates) = request(&app, Method::GET, "/v3/templates".into(), None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            templates
                .as_array()
                .unwrap()
                .iter()
                .any(|template| template["hda_disk_image"] == uploaded["image"])
        );
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
