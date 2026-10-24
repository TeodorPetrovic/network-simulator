use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::path::{Component, Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{
        Path, Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get, post, put},
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sqlx::{Row, SqlitePool};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard, broadcast};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UnixStream},
    time::{Duration, sleep},
};
use tower_http::{
    limit::RequestBodyLimitLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use uuid::Uuid;

#[path = "../auth/mod.rs"]
pub(crate) mod auth;
#[path = "controller_routes.rs"]
mod controller_routes;

use crate::{
    builtin::BuiltinRuntime,
    dynamips::{DynamipsOptions, DynamipsRuntime},
    links::{AdapterNio, LinkEngine, NioAdapter},
    marketplace::{ApplianceRecord, MarketplaceCatalog},
    qemu::{QemuLink, QemuRuntime, QemuTemplateOptions},
    runtime::{
        DockerRuntime, DockerTemplateOptions, NodeAction, RuntimeError, parse_start_command,
    },
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
    builtin: BuiltinRuntime,
    ubridge: UbridgeRuntime,
    marketplace: MarketplaceCatalog,
    bind_address: SocketAddr,
    started_at: Instant,
    project_operation_locks: Arc<StdMutex<HashMap<String, Arc<AsyncMutex<()>>>>>,
    project_activity: ProjectActivityTracker,
    idle_stop_minutes: Option<u64>,
}

impl AppState {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            runtime: DockerRuntime::new(),
            qemu: QemuRuntime::new(),
            vpcs: VpcsRuntime::new(),
            dynamips: DynamipsRuntime::new(),
            builtin: BuiltinRuntime::default(),
            ubridge: UbridgeRuntime::new(),
            marketplace: MarketplaceCatalog::new(),
            bind_address: std::env::var("NETWORK_SIMULATOR_BIND")
                .ok()
                .and_then(|address| address.parse().ok())
                .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 3080))),
            started_at: Instant::now(),
            project_operation_locks: Arc::new(StdMutex::new(HashMap::new())),
            project_activity: ProjectActivityTracker::default(),
            idle_stop_minutes: configured_idle_stop_minutes(),
        }
    }

    #[cfg(test)]
    fn with_runtimes(
        store: Store,
        runtime: DockerRuntime,
        qemu: QemuRuntime,
        vpcs: VpcsRuntime,
        dynamips: DynamipsRuntime,
        marketplace: MarketplaceCatalog,
    ) -> Self {
        Self::with_runtimes_and_ubridge(
            store,
            runtime,
            qemu,
            vpcs,
            dynamips,
            UbridgeRuntime::new(),
            marketplace,
        )
    }

    #[cfg(test)]
    fn with_runtimes_and_ubridge(
        store: Store,
        runtime: DockerRuntime,
        qemu: QemuRuntime,
        vpcs: VpcsRuntime,
        dynamips: DynamipsRuntime,
        ubridge: UbridgeRuntime,
        marketplace: MarketplaceCatalog,
    ) -> Self {
        Self {
            store,
            runtime,
            qemu,
            vpcs,
            dynamips,
            builtin: BuiltinRuntime::default(),
            ubridge,
            marketplace,
            bind_address: SocketAddr::from(([127, 0, 0, 1], 3080)),
            started_at: Instant::now(),
            project_operation_locks: Arc::new(StdMutex::new(HashMap::new())),
            project_activity: ProjectActivityTracker::default(),
            idle_stop_minutes: configured_idle_stop_minutes(),
        }
    }

    pub fn with_bind_address(mut self, bind_address: SocketAddr) -> Self {
        self.bind_address = bind_address;
        self
    }

    async fn lock_project_operations(&self, project_id: &str) -> OwnedMutexGuard<()> {
        let lock = self
            .project_operation_locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(project_id.to_owned())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();
        lock.lock_owned().await
    }

    /// Reattach links for runtimes that survived a controller restart. Nodes
    /// that were stopped with the controller are attached later by their
    /// normal start lifecycle path.
    /// Kill runtime processes a previous controller run left behind.
    pub fn reap_orphaned_runtimes(&self) {
        let killed = self.vpcs.reap_orphans();
        if killed > 0 {
            tracing::warn!(killed, "killed orphaned VPCS processes from an earlier run");
        }
        let killed = self.dynamips.reap_orphans();
        if killed > 0 {
            tracing::warn!(
                killed,
                "killed orphaned Dynamips hypervisors from an earlier run"
            );
        }
    }

    /// Terminate processes this controller owns that cannot outlive it.
    pub async fn shutdown_runtimes(&self) {
        let rows = match sqlx::query(
            "SELECT project_id, object_id, object_json FROM topology_objects
             WHERE kind = 'node' AND json_extract(object_json, '$.status') IN ('started', 'suspended')",
        )
        .fetch_all(self.store.pool())
        .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::error!(%error, "could not load running nodes for graceful shutdown");
                Vec::new()
            }
        };
        for row in rows {
            let project_id: String = row.get("project_id");
            let node_id: String = row.get("object_id");
            let node = match decode_json(row.get::<String, _>("object_json")) {
                Ok(node) => node,
                Err(error) => {
                    tracing::warn!(error = %error.message, %project_id, %node_id, "skipping invalid node during graceful shutdown");
                    continue;
                }
            };
            if !controller_routes::bulk_node_action_applies(&node, NodeAction::Stop) {
                continue;
            }
            let (Ok(project_uuid), Ok(node_uuid)) =
                (Uuid::parse_str(&project_id), Uuid::parse_str(&node_id))
            else {
                tracing::warn!(%project_id, %node_id, "skipping node with an invalid ID during graceful shutdown");
                continue;
            };
            if let Err(error) =
                change_node_runtime_state(self.clone(), project_uuid, node_uuid, NodeAction::Stop)
                    .await
            {
                tracing::error!(error = %error.message, %project_id, %node_id, "could not stop node during graceful shutdown");
            }
        }
        self.dynamips.kill_all().await;
        self.ubridge.shutdown().await;
        self.vpcs.kill_all().await;
    }

    fn start_idle_stop_worker(&self) {
        let Some(minutes) = self.idle_stop_minutes.filter(|minutes| *minutes > 0) else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::warn!("idle stop is configured but no Tokio runtime is active");
            return;
        };
        let state = self.clone();
        handle.spawn(async move {
            let interval = Duration::from_secs(minutes.saturating_mul(60));
            let mut ticker = tokio::time::interval(interval.min(Duration::from_secs(60)));
            ticker.tick().await;
            loop {
                ticker.tick().await;
                state.stop_idle_projects(interval).await;
            }
        });
        tracing::info!(idle_stop_minutes = minutes, "project idle stop enabled");
    }

    async fn stop_idle_projects(&self, idle_for: Duration) {
        let rows = match sqlx::query("SELECT project_id FROM projects ORDER BY project_id")
            .fetch_all(self.store.pool())
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::error!(%error, "could not list projects for idle stop");
                return;
            }
        };
        for row in rows {
            let project_id: String = row.get("project_id");
            if self
                .project_activity
                .is_idle(&project_id, self.started_at, idle_for)
            {
                self.stop_idle_project(&project_id, idle_for).await;
            }
        }
    }

    async fn stop_idle_project(&self, project_id: &str, idle_for: Duration) {
        let _project_lock = self.lock_project_operations(project_id).await;
        if !self
            .project_activity
            .is_idle(project_id, self.started_at, idle_for)
        {
            return;
        }
        let rows = match sqlx::query(
            "SELECT object_id, object_json FROM topology_objects
             WHERE project_id = ? AND kind = 'node' ORDER BY object_id",
        )
        .bind(project_id)
        .fetch_all(self.store.pool())
        .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::error!(%error, %project_id, "could not load nodes for idle stop");
                return;
            }
        };
        for row in rows {
            if !self
                .project_activity
                .is_idle(project_id, self.started_at, idle_for)
            {
                return;
            }
            let node_id: String = row.get("object_id");
            let mut node = match decode_json(row.get::<String, _>("object_json")) {
                Ok(node) => node,
                Err(error) => {
                    tracing::warn!(error = %error.message, %project_id, %node_id, "skipping invalid node during idle stop");
                    continue;
                }
            };
            if !controller_routes::bulk_node_action_applies(&node, NodeAction::Stop) {
                continue;
            }
            match apply_node_lifecycle(self, project_id, &mut node, NodeAction::Stop).await {
                Ok(status) => {
                    node["status"] = json!(status);
                    let result = async {
                        let mut tx = self.store.pool().begin().await?;
                        sqlx::query(
                            "UPDATE topology_objects SET object_json = ?
                             WHERE project_id = ? AND kind = 'node' AND object_id = ?",
                        )
                        .bind(serde_json::to_string(&node).map_err(internal_json_error)?)
                        .bind(project_id)
                        .bind(&node_id)
                        .execute(&mut *tx)
                        .await?;
                        let event =
                            Store::append_event(&mut tx, project_id, "node.updated", &node).await?;
                        tx.commit().await?;
                        Ok::<_, ApiError>(event)
                    }
                    .await;
                    match result {
                        Ok(event) => self.store.publish(event),
                        Err(error) => {
                            tracing::error!(error = %error.message, %project_id, %node_id, "could not persist idle-stopped node")
                        }
                    }
                }
                Err(error) => {
                    tracing::error!(error = %error.message, %project_id, %node_id, "could not stop idle node")
                }
            }
        }
    }

    pub async fn reattach_running_links(&self) {
        let rows = match sqlx::query(
            "SELECT project_id, object_json FROM topology_objects WHERE kind = 'link'",
        )
        .fetch_all(self.store.pool())
        .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::error!(%error, "could not load links for runtime reconciliation");
                return;
            }
        };
        for row in rows {
            let project_id = row.get::<String, _>("project_id");
            let link = match decode_json(row.get::<String, _>("object_json")) {
                Ok(link) => link,
                Err(error) => {
                    tracing::warn!(error = %error.message, %project_id, "skipping invalid link during runtime reconciliation");
                    continue;
                }
            };
            let _project_lock = self.lock_project_operations(&project_id).await;
            if let Err(error) = LinkEngine::new(self.ubridge.clone())
                .claim_link_ports(&link)
                .await
            {
                tracing::warn!(error = error.message(), %project_id, link_id = %link["link_id"], "could not reserve persisted UDP ports during runtime reconciliation");
                continue;
            }
            let endpoints = match link_runtime_endpoints(self.store.pool(), &project_id, &link)
                .await
            {
                Ok(endpoints) => endpoints,
                Err(error) => {
                    tracing::warn!(error = %error.message, %project_id, link_id = %link["link_id"], "could not resolve link endpoints during runtime reconciliation");
                    continue;
                }
            };
            for endpoint in link["nodes"].as_array().into_iter().flatten() {
                let node_id = endpoint["node_id"].as_str().unwrap_or_default();
                let Some(runtime_endpoint) = endpoints
                    .iter()
                    .find(|candidate| candidate["node_id"].as_str() == Some(node_id))
                else {
                    continue;
                };
                match runtime_endpoint_status(self, &project_id, runtime_endpoint).await {
                    Ok(status) if status != "stopped" => {
                        if let Err(error) = attach_runtime_link_endpoint(
                            self,
                            &project_id,
                            &link,
                            endpoint,
                            runtime_endpoint,
                        )
                        .await
                        {
                            tracing::warn!(error = %error.message, %project_id, %node_id, link_id = %link["link_id"], "could not reattach link endpoint after controller restart");
                        }
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!(error = %error.message, %project_id, %node_id, "could not inspect node during link reconciliation")
                    }
                }
            }
        }
    }
}

#[derive(Clone, Default)]
struct ProjectActivityTracker {
    projects: Arc<StdMutex<HashMap<String, ProjectActivity>>>,
}

#[derive(Clone, Copy)]
struct ProjectActivity {
    last_activity: Instant,
    open_websockets: usize,
}

impl ProjectActivityTracker {
    fn record_request(&self, project_id: &str) {
        let mut projects = self
            .projects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let activity = projects
            .entry(project_id.to_owned())
            .or_insert(ProjectActivity {
                last_activity: Instant::now(),
                open_websockets: 0,
            });
        activity.last_activity = Instant::now();
    }

    fn open_websocket(&self, project_id: String) -> ProjectWebSocketGuard {
        let mut projects = self
            .projects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let activity = projects
            .entry(project_id.clone())
            .or_insert(ProjectActivity {
                last_activity: Instant::now(),
                open_websockets: 0,
            });
        activity.last_activity = Instant::now();
        activity.open_websockets = activity.open_websockets.saturating_add(1);
        ProjectWebSocketGuard {
            tracker: self.clone(),
            project_id,
        }
    }

    fn is_idle(&self, project_id: &str, fallback_since: Instant, idle_for: Duration) -> bool {
        self.is_idle_at(project_id, fallback_since, idle_for, Instant::now())
    }

    fn is_idle_at(
        &self,
        project_id: &str,
        fallback_since: Instant,
        idle_for: Duration,
        now: Instant,
    ) -> bool {
        let projects = self
            .projects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(activity) = projects.get(project_id) else {
            return now.saturating_duration_since(fallback_since) >= idle_for;
        };
        activity.open_websockets == 0
            && now.saturating_duration_since(activity.last_activity) >= idle_for
    }
}

struct ProjectWebSocketGuard {
    tracker: ProjectActivityTracker,
    project_id: String,
}

impl Drop for ProjectWebSocketGuard {
    fn drop(&mut self) {
        let mut projects = self
            .tracker
            .projects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(activity) = projects.get_mut(&self.project_id) {
            activity.open_websockets = activity.open_websockets.saturating_sub(1);
            if activity.open_websockets == 0 {
                activity.last_activity = Instant::now();
            }
        }
    }
}

fn configured_idle_stop_minutes() -> Option<u64> {
    let configured = std::env::var("NETWORK_SIMULATOR_IDLE_STOP_MINUTES").ok()?;
    match configured.trim().parse::<u64>() {
        Ok(minutes) if (1..=525_600).contains(&minutes) => Some(minutes),
        _ => {
            tracing::warn!(value = %configured, "ignoring invalid NETWORK_SIMULATOR_IDLE_STOP_MINUTES; expected an integer from 1 to 525600");
            None
        }
    }
}

fn project_id_from_path(path: &str) -> Option<String> {
    let segments = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let project_segment = match segments.as_slice() {
        ["v3", "projects", project_id, ..] => *project_id,
        ["v3", "compute", "projects", project_id, ..] => *project_id,
        _ => return None,
    };
    Uuid::parse_str(project_segment)
        .ok()
        .map(|project_id| project_id.to_string())
}

async fn track_project_activity(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let project_id = project_id_from_path(request.uri().path());
    let response = next.run(request).await;
    if response.status().is_success() {
        if let Some(project_id) = project_id {
            state.project_activity.record_request(&project_id);
        }
    }
    response
}

pub fn router(state: AppState) -> Router {
    state.start_idle_stop_worker();
    let api = Router::new()
        .route("/healthz", get(health))
        .route(
            "/v3/version",
            get(version).post(controller_routes::check_version),
        )
        .route("/v3/runtimes", get(runtimes))
        .route("/v3/resources/monitor", get(resource_monitor))
        .route("/v3/docker/images/pull", post(pull_docker_image))
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
        .route("/v3/marketplace/gns3/custom", post(create_custom_appliance))
        .route(
            "/v3/marketplace/gns3/imported",
            get(list_imported_appliances),
        )
        .route("/v3/marketplace/gns3/manager", get(get_appliance_manager))
        .route(
            "/v3/marketplace/gns3/appliances/{file_name}/bindings",
            put(set_appliance_image_binding).delete(clear_appliance_image_binding),
        )
        .route(
            "/v3/marketplace/gns3/import/{file_name}",
            post(import_appliance_file),
        )
        .route(
            "/v3/projects",
            get(projects::list_projects).post(projects::create_project),
        )
        .route(
            "/v3/projects/{project_id}",
            get(projects::get_project)
                .put(projects::update_project)
                .delete(projects::delete_project),
        )
        .route(
            "/v3/projects/{project_id}/open",
            post(projects::open_project),
        )
        .route(
            "/v3/projects/{project_id}/close",
            post(projects::close_project),
        )
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
            "/v3/projects/{project_id}/nodes/{node_id}/config",
            put(update_node_configuration),
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
            "/v3/projects/{project_id}/links/{link_id}/available_filters",
            get(available_link_filters),
        )
        .route(
            "/v3/projects/{project_id}/links/{link_id}/capture/start",
            post(start_link_capture),
        )
        .route(
            "/v3/projects/{project_id}/links/{link_id}/capture/stop",
            post(stop_link_capture),
        )
        .route(
            "/v3/projects/{project_id}/links/{link_id}/capture/stream",
            get(stream_link_capture),
        )
        .route(
            "/v3/projects/{project_id}/links/{link_id}/capture/file",
            get(download_link_capture),
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
            "/v3/compute/projects/{project_id}/docker/nodes/{node_id}/aux_console/ws",
            get(docker_aux_console),
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
    auth::protect(
        Router::new()
            .merge(api)
            .merge(controller_routes::router())
            .merge(auth::router())
            .merge(image_upload)
            .merge(qemu_image_upload)
            .layer(TraceLayer::new_for_http().make_span_with(request_span))
            .layer(middleware::from_fn_with_state(
                state.clone(),
                track_project_activity,
            )),
        state.clone(),
    )
    .with_state(state)
}

fn request_trace_path<B>(request: &axum::http::Request<B>) -> &str {
    request.uri().path()
}

fn request_span<B>(request: &axum::http::Request<B>) -> tracing::Span {
    tracing::info_span!(
        "http_request",
        method = %request.method(),
        path = %request_trace_path(request),
        version = ?request.version(),
    )
}

#[cfg(test)]
#[path = "tests/request_trace_tests.rs"]
mod request_trace_tests;

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

#[derive(Debug)]
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
        "controller_host": null,
        "version": "3.1.0.dev7",
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
    let templates = available_templates(&state).await?;
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

async fn get_appliance_manager(
    State(state): State<AppState>,
) -> Result<Json<crate::marketplace::ApplianceManagerState>, ApiError> {
    let image_options = base_runtime_templates(&state).await;
    state
        .marketplace
        .appliance_manager(&image_options)
        .await
        .map(Json)
        .map_err(catalog_api_error)
}

#[derive(Deserialize)]
struct CreateCustomAppliance {
    name: String,
    source_template_id: String,
    #[serde(default = "default_object")]
    runtime_settings: Value,
}

fn default_object() -> Value {
    json!({})
}

async fn create_custom_appliance(
    State(state): State<AppState>,
    Json(request): Json<CreateCustomAppliance>,
) -> Result<Json<crate::marketplace::ApplianceRecord>, ApiError> {
    let image_options = base_runtime_templates(&state).await;
    state
        .marketplace
        .create_custom_appliance(
            &request.name,
            &request.source_template_id,
            &request.runtime_settings,
            &image_options,
        )
        .map(Json)
        .map_err(ApiError::bad_request)
}

#[derive(Deserialize)]
struct SetApplianceImageBinding {
    version: String,
    image_key: String,
    image_id: String,
    #[serde(default)]
    allow_checksum_mismatch: bool,
}

async fn set_appliance_image_binding(
    State(state): State<AppState>,
    Path(file_name): Path<String>,
    Json(request): Json<SetApplianceImageBinding>,
) -> Result<Json<Value>, ApiError> {
    let templates = base_runtime_templates(&state).await;
    let appliance = state
        .marketplace
        .set_image_binding(
            &file_name,
            &request.version,
            &request.image_key,
            &request.image_id,
            request.allow_checksum_mismatch,
            &templates,
        )
        .await
        .map_err(catalog_api_error)?;
    Ok(Json(json!({ "appliance": appliance })))
}

#[derive(Deserialize)]
struct ClearApplianceImageBinding {
    version: String,
    image_key: String,
}

async fn clear_appliance_image_binding(
    State(state): State<AppState>,
    Path(file_name): Path<String>,
    Json(request): Json<ClearApplianceImageBinding>,
) -> Result<Json<Value>, ApiError> {
    let appliance = state
        .marketplace
        .clear_image_binding(&file_name, &request.version, &request.image_key)
        .await
        .map_err(catalog_api_error)?;
    Ok(Json(json!({ "appliance": appliance })))
}

async fn import_appliance_file(
    State(state): State<AppState>,
    Path(file_name): Path<String>,
    body: Body,
) -> Result<Json<ApplianceRecord>, ApiError> {
    let bytes = axum::body::to_bytes(body, MAX_BODY_BYTES)
        .await
        .map_err(|error| {
            ApiError::bad_request(format!("Could not read appliance file: {error}"))
        })?;
    state
        .marketplace
        .import_appliance_file(&file_name, &bytes)
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
    Ok(Json(available_templates(&state).await?))
}

async fn available_templates(state: &AppState) -> Result<Vec<Value>, ApiError> {
    // Discovered runtime images are only inputs to appliances, never palette entries;
    // the palette is the built-in nodes plus imported/custom appliances.
    let base = base_runtime_templates(state).await;
    let mut templates = BuiltinRuntime::templates();
    templates.extend(state.marketplace.templates(base));
    let rows = sqlx::query("SELECT template_id, template_json FROM runtime_template_profiles")
        .fetch_all(state.store.pool())
        .await?;
    for row in rows {
        let template_id: String = row.get("template_id");
        let profile = decode_json(row.get::<String, _>("template_json"))?;
        if let Some(template) = templates
            .iter_mut()
            .find(|template| template["template_id"].as_str() == Some(template_id.as_str()))
            && let (Some(base), Some(overrides)) = (template.as_object_mut(), profile.as_object())
        {
            for (key, value) in overrides {
                base.insert(key.clone(), value.clone());
            }
        }
    }
    Ok(templates)
}

async fn update_template_profile(
    State(state): State<AppState>,
    Path(template_id): Path<Uuid>,
    Json(patch): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let template_id = template_id.to_string();
    let template = available_templates(&state)
        .await?
        .into_iter()
        .find(|template| template["template_id"].as_str() == Some(template_id.as_str()))
        .ok_or_else(|| ApiError::not_found(format!("Template {template_id} does not exist")))?;
    let patch = validate_template_profile_patch(&template, patch)?;
    let row =
        sqlx::query("SELECT template_json FROM runtime_template_profiles WHERE template_id = ?")
            .bind(&template_id)
            .fetch_optional(state.store.pool())
            .await?;
    let mut profile = row
        .map(|row| decode_json(row.get::<String, _>("template_json")))
        .transpose()?
        .unwrap_or_else(|| json!({}));
    {
        let profile_object = as_object_mut(&mut profile, "template profile must be an object")?;
        for (key, value) in patch.as_object().expect("validated profile is an object") {
            profile_object.insert(key.clone(), value.clone());
        }
    }
    let profile_json = serde_json::to_string(&profile).map_err(internal_json_error)?;
    sqlx::query(
        "INSERT INTO runtime_template_profiles (template_id, template_type, template_json)
         VALUES (?, ?, ?)
         ON CONFLICT(template_id) DO UPDATE SET
           template_type = excluded.template_type,
           template_json = excluded.template_json,
           updated_at = CURRENT_TIMESTAMP",
    )
    .bind(&template_id)
    .bind(template["template_type"].as_str().unwrap_or_default())
    .bind(profile_json)
    .execute(state.store.pool())
    .await?;
    let mut updated = template;
    for (key, value) in profile.as_object().expect("profile is an object") {
        updated[key] = value.clone();
    }
    Ok(Json(updated))
}

async fn reset_template_profile(
    State(state): State<AppState>,
    Path(template_id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let template_id = template_id.to_string();
    sqlx::query("DELETE FROM runtime_template_profiles WHERE template_id = ?")
        .bind(&template_id)
        .execute(state.store.pool())
        .await?;
    let template = available_templates(&state)
        .await?
        .into_iter()
        .find(|template| template["template_id"].as_str() == Some(template_id.as_str()))
        .ok_or_else(|| ApiError::not_found(format!("Template {template_id} does not exist")))?;
    Ok(Json(template))
}

fn validate_template_profile_patch(template: &Value, patch: Value) -> Result<Value, ApiError> {
    let patch = patch
        .as_object()
        .ok_or_else(|| ApiError::bad_request("template settings must be an object"))?;
    let kind = template["template_type"].as_str().unwrap_or_default();
    let allowed: &[&str] = match kind {
        "qemu" => &[
            "name",
            "adapters",
            "memory",
            "cpus",
            "maxcpus",
            "linked_clone",
            "adapter_type",
            "hda_disk_interface",
            "hdb_disk_image",
            "hdb_disk_interface",
            "hdb_disk_image_backing_file",
            "hdc_disk_image",
            "hdc_disk_interface",
            "hdc_disk_image_backing_file",
            "hdd_disk_image",
            "hdd_disk_interface",
            "hdd_disk_image_backing_file",
            "hda_disk_image_backing_file",
            "cdrom_image",
            "bios_image",
            "kernel_image",
            "initrd",
            "kernel_command_line",
            "boot_priority",
            "console_type",
            "aux_type",
            "on_close",
            "process_priority",
            "options",
            "tpm",
            "uefi",
            "replicate_network_connection_state",
            "create_config_disk",
            "cpu_throttling",
        ],
        "docker" => &[
            "name",
            "adapters",
            "memory",
            "cpus",
            "start_command",
            "environment",
            "extra_hosts",
            "extra_volumes",
            "extra_configs",
            "startup_config_content",
            "console_type",
            "console_resolution",
            "console_http_port",
            "console_http_path",
            "aux_type",
            "custom_adapters",
        ],
        "dynamips" => &[
            "name",
            "platform",
            "chassis",
            "memory",
            "nvram",
            "idlepc",
            "mmap",
            "sparsemem",
            "clock_divisor",
            "idlemax",
            "idlesleep",
            "exec_area",
            "disk0",
            "disk1",
            "auto_delete_disks",
            "private_config",
            "mac_addr",
            "system_id",
            "slot0",
            "slot1",
            "slot2",
            "slot3",
            "slot4",
            "slot5",
            "slot6",
            "wic0",
            "wic1",
            "wic2",
            "npe",
            "midplane",
            "iomem",
            "ghost_ios",
            "console_type",
            "aux_type",
        ],
        "vpcs" => &[
            "name",
            "startup_script",
            "base_script_file",
            "console_type",
            "console_auto_start",
        ],
        "ethernet_switch" => &["name", "ports_mapping", "console_type"],
        "ethernet_hub" => &["name", "ports_mapping"],
        "cloud" => &[
            "name",
            "ports_mapping",
            "remote_console_host",
            "remote_console_port",
            "remote_console_type",
            "remote_console_http_path",
        ],
        "nat" => &["name", "ports_mapping"],
        _ => return Err(ApiError::bad_request("unsupported template runtime")),
    };
    let mut validated = serde_json::Map::new();
    for (key, value) in patch {
        if !allowed.contains(&key.as_str()) {
            return Err(ApiError::bad_request(format!(
                "'{key}' is not an editable {kind} template setting"
            )));
        }
        match key.as_str() {
            "name" => {
                let name = value
                    .as_str()
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| ApiError::bad_request("template name must not be empty"))?;
                if name.len() > 120 {
                    return Err(ApiError::bad_request("template name is too long"));
                }
                validated.insert(key.clone(), json!(name));
            }
            "ports_mapping" => {
                let mapping =
                    crate::builtin::BuiltinRuntime::normalize_template_ports(kind, value.clone())
                        .map_err(ApiError::bad_request)?;
                validated.insert(key.clone(), mapping);
            }
            "console_type" if kind == "ethernet_switch" => {
                let console_type = value
                    .as_str()
                    .filter(|value| matches!(*value, "none" | "telnet"))
                    .ok_or_else(|| ApiError::bad_request("unsupported switch console_type"))?;
                validated.insert(key.clone(), json!(console_type));
            }
            "remote_console_host" => {
                let host = value
                    .as_str()
                    .filter(|host| {
                        host.len() <= 255 && !host.bytes().any(|byte| byte.is_ascii_control())
                    })
                    .ok_or_else(|| ApiError::bad_request("remote_console_host is invalid"))?;
                validated.insert(key.clone(), json!(host));
            }
            "remote_console_port" => {
                let port = value
                    .as_u64()
                    .filter(|port| (1..=65535).contains(port))
                    .ok_or_else(|| {
                        ApiError::bad_request("remote_console_port must be from 1 to 65535")
                    })?;
                validated.insert(key.clone(), json!(port));
            }
            "remote_console_type" => {
                let console_type = value
                    .as_str()
                    .filter(|value| {
                        matches!(
                            *value,
                            "none" | "telnet" | "ssh" | "vnc" | "spice" | "http" | "https"
                        )
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request("unsupported cloud remote console type")
                    })?;
                validated.insert(key.clone(), json!(console_type));
            }
            "remote_console_http_path" => {
                let path = value
                    .as_str()
                    .filter(|path| {
                        path.starts_with('/') && !path.bytes().any(|byte| byte.is_ascii_control())
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request("remote_console_http_path must be an absolute path")
                    })?;
                validated.insert(key.clone(), json!(path));
            }
            "adapters" => {
                let adapters = value
                    .as_u64()
                    .filter(|value| {
                        if kind == "qemu" {
                            *value <= 275
                        } else if kind == "dynamips" {
                            (1..=99).contains(value)
                        } else {
                            (1..=99).contains(value)
                        }
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request(if kind == "qemu" {
                            "QEMU adapters must be an integer from 0 to 275"
                        } else {
                            "adapters must be an integer from 1 to 99"
                        })
                    })?;
                validated.insert(key.clone(), json!(adapters));
            }
            "memory" => {
                let memory = value
                    .as_u64()
                    .filter(|value| {
                        if kind == "qemu" {
                            (16..=1_048_576).contains(value)
                        } else if kind == "dynamips" {
                            (1..=1_048_576).contains(value)
                        } else {
                            *value <= 1_048_576
                        }
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request("memory is outside the supported range")
                    })?;
                validated.insert(key.clone(), json!(memory));
            }
            "cpus" if kind == "docker" => {
                let cpus = value
                    .as_f64()
                    .filter(|value| (0.0..=256.0).contains(value))
                    .ok_or_else(|| {
                        ApiError::bad_request("Docker CPU limit must be between 0 and 256")
                    })?;
                validated.insert(key.clone(), json!(cpus));
            }
            "cpus" => {
                let cpus = value
                    .as_u64()
                    .filter(|value| (1..=255).contains(value))
                    .ok_or_else(|| {
                        ApiError::bad_request("CPU count must be an integer from 1 to 256")
                    })?;
                validated.insert(key.clone(), json!(cpus));
            }
            "adapter_type" if kind == "qemu" => {
                let model = value
                    .as_str()
                    .filter(|value| {
                        matches!(
                            *value,
                            "e1000"
                                | "e1000-82544gc"
                                | "e1000-82545em"
                                | "e1000e"
                                | "i82550"
                                | "i82551"
                                | "i82557a"
                                | "i82557b"
                                | "i82557c"
                                | "i82558a"
                                | "i82558b"
                                | "i82559a"
                                | "i82559b"
                                | "i82559c"
                                | "i82559er"
                                | "i82562"
                                | "i82801"
                                | "igb"
                                | "ne2k_pci"
                                | "pcnet"
                                | "rocker"
                                | "rtl8139"
                                | "virtio"
                                | "virtio-net-pci"
                                | "vmxnet3"
                        )
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request("unsupported QEMU network adapter model")
                    })?;
                validated.insert(key.clone(), json!(model));
            }
            "hda_disk_interface" if kind == "qemu" => {
                let interface = value
                    .as_str()
                    .filter(|value| {
                        matches!(
                            *value,
                            "ide"
                                | "sata"
                                | "nvme"
                                | "scsi"
                                | "virtio"
                                | "sd"
                                | "mtd"
                                | "floppy"
                                | "pflash"
                                | "none"
                        )
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request("unsupported QEMU primary disk interface")
                    })?;
                validated.insert(key.clone(), json!(interface));
            }
            "boot_priority" if kind == "qemu" => {
                let order = value
                    .as_str()
                    .filter(|value| {
                        matches!(
                            *value,
                            "c" | "d" | "n" | "cn" | "cd" | "dn" | "dc" | "nc" | "nd"
                        )
                    })
                    .ok_or_else(|| ApiError::bad_request("unsupported QEMU boot priority"))?;
                validated.insert(key.clone(), json!(order));
            }
            "maxcpus" if kind == "qemu" => {
                let value = value
                    .as_u64()
                    .filter(|value| (1..=255).contains(value))
                    .ok_or_else(|| ApiError::bad_request("QEMU maxcpus must be from 1 to 255"))?;
                validated.insert(key.clone(), json!(value));
            }
            "linked_clone"
            | "tpm"
            | "uefi"
            | "replicate_network_connection_state"
            | "create_config_disk"
                if kind == "qemu" =>
            {
                let value = value
                    .as_bool()
                    .ok_or_else(|| ApiError::bad_request(format!("{key} must be a boolean")))?;
                validated.insert(key.clone(), json!(value));
            }
            "hdb_disk_interface" | "hdc_disk_interface" | "hdd_disk_interface"
                if kind == "qemu" =>
            {
                let interface = value
                    .as_str()
                    .filter(|value| {
                        matches!(
                            *value,
                            "ide"
                                | "sata"
                                | "nvme"
                                | "scsi"
                                | "virtio"
                                | "sd"
                                | "mtd"
                                | "floppy"
                                | "pflash"
                                | "none"
                        )
                    })
                    .ok_or_else(|| ApiError::bad_request(format!("unsupported QEMU {key}")))?;
                validated.insert(key.clone(), json!(interface));
            }
            "hdb_disk_image" | "hdc_disk_image" | "hdd_disk_image" | "cdrom_image"
            | "bios_image" | "kernel_image" | "initrd"
                if kind == "qemu" =>
            {
                let path = value
                    .as_str()
                    .filter(|value| value.len() <= 4096)
                    .ok_or_else(|| {
                        ApiError::bad_request(format!("{key} must be a path under 4096 characters"))
                    })?;
                validated.insert(key.clone(), json!(path));
            }
            "kernel_command_line" | "options" if kind == "qemu" => {
                let max = if key == "options" {
                    16 * 1024
                } else {
                    64 * 1024
                };
                let text = value
                    .as_str()
                    .filter(|text| text.len() <= max)
                    .ok_or_else(|| ApiError::bad_request(format!("{key} is too long")))?;
                validated.insert(key.clone(), json!(text));
            }
            "console_type" | "aux_type" if kind == "qemu" => {
                let value = value
                    .as_str()
                    .filter(|value| {
                        matches!(
                            *value,
                            "telnet" | "ssh" | "vnc" | "spice" | "spice+agent" | "none"
                        )
                    })
                    .ok_or_else(|| ApiError::bad_request(format!("unsupported QEMU {key}")))?;
                validated.insert(key.clone(), json!(value));
            }
            "on_close" if kind == "qemu" => {
                let value = value
                    .as_str()
                    .filter(|value| {
                        matches!(*value, "power_off" | "shutdown_signal" | "save_vm_state")
                    })
                    .ok_or_else(|| ApiError::bad_request("unsupported QEMU on_close action"))?;
                validated.insert(key.clone(), json!(value));
            }
            "process_priority" if kind == "qemu" => {
                let value = value
                    .as_str()
                    .filter(|value| {
                        matches!(
                            *value,
                            "realtime" | "very high" | "high" | "normal" | "low" | "very low"
                        )
                    })
                    .ok_or_else(|| ApiError::bad_request("unsupported QEMU process priority"))?;
                validated.insert(key.clone(), json!(value));
            }
            "cpu_throttling" if kind == "qemu" => {
                let value = value
                    .as_u64()
                    .filter(|value| *value <= 800)
                    .ok_or_else(|| {
                        ApiError::bad_request("QEMU cpu_throttling must be from 0 to 800")
                    })?;
                validated.insert(key.clone(), json!(value));
            }
            "platform" if kind == "dynamips" => {
                let platform = value
                    .as_str()
                    .filter(|value| {
                        matches!(
                            *value,
                            "c1700" | "c2600" | "c2691" | "c3600" | "c3725" | "c3745" | "c7200"
                        )
                    })
                    .ok_or_else(|| ApiError::bad_request("unsupported Dynamips platform"))?;
                validated.insert(key.clone(), json!(platform));
            }
            "chassis" | "slot0" | "slot1" | "slot2" | "slot3" | "slot4" | "slot5" | "slot6"
            | "wic0" | "wic1" | "wic2" | "npe" | "midplane" | "system_id" | "mac_addr"
                if kind == "dynamips" =>
            {
                let setting = value
                    .as_str()
                    .ok_or_else(|| ApiError::bad_request("Dynamips setting must be text"))?;
                validated.insert(key.clone(), json!(setting));
            }
            "nvram" | "clock_divisor" | "idlemax" | "idlesleep" | "exec_area" | "disk0"
            | "disk1" | "iomem"
                if kind == "dynamips" =>
            {
                let value = value
                    .as_u64()
                    .filter(|value| match key.as_str() {
                        "iomem" => *value <= 100,
                        "clock_divisor" => (1..=4096).contains(value),
                        "disk0" | "disk1" | "exec_area" => *value <= 16_384,
                        _ => *value <= 1_048_576,
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request(format!("{key} is outside the supported range"))
                    })?;
                validated.insert(key.clone(), json!(value));
            }
            "idlepc" if kind == "dynamips" => {
                let value = value
                    .as_str()
                    .filter(|value| value.is_empty() || value.starts_with("0x"))
                    .ok_or_else(|| {
                        ApiError::bad_request("Idle-PC must be empty or a hexadecimal value")
                    })?;
                validated.insert(key.clone(), json!(value));
            }
            "mmap" | "sparsemem" | "auto_delete_disks" | "ghost_ios" | "console_auto_start"
                if kind == "dynamips" || kind == "vpcs" =>
            {
                let value = value
                    .as_bool()
                    .ok_or_else(|| ApiError::bad_request(format!("{key} must be a boolean")))?;
                validated.insert(key.clone(), json!(value));
            }
            "private_config" | "startup_script" | "base_script_file"
                if kind == "dynamips" || kind == "vpcs" =>
            {
                let value = value
                    .as_str()
                    .filter(|value| value.len() <= 1_048_576)
                    .ok_or_else(|| {
                        ApiError::bad_request(format!("{key} must be text under 1 MiB"))
                    })?;
                validated.insert(key.clone(), json!(value));
            }
            "console_type" | "aux_type" if kind == "dynamips" => {
                let value = value
                    .as_str()
                    .filter(|value| matches!(*value, "telnet" | "ssh" | "none"))
                    .ok_or_else(|| ApiError::bad_request(format!("unsupported Dynamips {key}")))?;
                validated.insert(key.clone(), json!(value));
            }
            "console_type" if kind == "docker" => {
                let value = value
                    .as_str()
                    .filter(|value| matches!(*value, "none" | "telnet" | "vnc" | "http" | "https"))
                    .ok_or_else(|| ApiError::bad_request("unsupported Docker console_type"))?;
                validated.insert(key.clone(), json!(value));
            }
            "aux_type" if kind == "docker" => {
                let value = value
                    .as_str()
                    .filter(|value| matches!(*value, "none" | "telnet" | "ssh"))
                    .ok_or_else(|| ApiError::bad_request("unsupported Docker aux_type"))?;
                validated.insert(key.clone(), json!(value));
            }
            "console_resolution" if kind == "docker" => {
                let resolution = value
                    .as_str()
                    .filter(|value| {
                        value.split_once('x').is_some_and(|(w, h)| {
                            !w.is_empty()
                                && !h.is_empty()
                                && w.bytes().all(|byte| byte.is_ascii_digit())
                                && h.bytes().all(|byte| byte.is_ascii_digit())
                        })
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request("console_resolution must use WIDTHxHEIGHT")
                    })?;
                validated.insert(key.clone(), json!(resolution));
            }
            "console_http_port" if kind == "docker" => {
                let port = value
                    .as_u64()
                    .filter(|port| (1..=65535).contains(port))
                    .ok_or_else(|| {
                        ApiError::bad_request("console_http_port must be from 1 to 65535")
                    })?;
                validated.insert(key.clone(), json!(port));
            }
            "console_http_path" if kind == "docker" => {
                let path = value
                    .as_str()
                    .filter(|path| {
                        path.starts_with('/') && !path.bytes().any(|byte| byte.is_ascii_control())
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request("console_http_path must be an absolute URL path")
                    })?;
                validated.insert(key.clone(), json!(path));
            }
            "custom_adapters" if kind == "docker" => {
                if !value.is_array() || value.as_array().is_some_and(|adapters| adapters.len() > 99)
                {
                    return Err(ApiError::bad_request(
                        "custom_adapters must contain at most 99 entries",
                    ));
                }
                serde_json::from_value::<Vec<crate::runtime::DockerCustomAdapter>>(value.clone())
                    .map_err(|_| {
                    ApiError::bad_request("custom_adapters contain an invalid definition")
                })?;
                validated.insert(key.clone(), value.clone());
            }
            "console_type" if kind == "vpcs" => {
                let value = value
                    .as_str()
                    .filter(|value| matches!(*value, "telnet" | "ssh" | "none"))
                    .ok_or_else(|| {
                        ApiError::bad_request("VPCS console_type must be telnet, ssh, or none")
                    })?;
                validated.insert(key.clone(), json!(value));
            }
            "extra_volumes" if kind == "docker" => {
                let volumes = value.as_array().ok_or_else(|| {
                    ApiError::bad_request("extra_volumes must be an array of container paths")
                })?;
                if volumes.len() > 32
                    || volumes.iter().any(|volume| {
                        volume
                            .as_str()
                            .is_none_or(|path| !valid_container_path(path))
                    })
                {
                    return Err(ApiError::bad_request(
                        "Docker volume paths must be absolute, safe container paths (up to 32)",
                    ));
                }
                validated.insert(key.clone(), value.clone());
            }
            "environment" | "extra_hosts" | "start_command" | "startup_script"
                if matches!(kind, "docker" | "vpcs") =>
            {
                let text = value
                    .as_str()
                    .ok_or_else(|| ApiError::bad_request(format!("{key} must be text")))?;
                if text.len() > 64 * 1024 {
                    return Err(ApiError::bad_request(format!("{key} is too long")));
                }
                if key == "start_command" && !text.trim().is_empty() {
                    parse_start_command(text)
                        .map_err(|error| ApiError::bad_request(error.message()))?;
                }
                validated.insert(key.clone(), json!(text));
            }
            "startup_config_content" if kind == "docker" => {
                let text = value
                    .as_str()
                    .filter(|text| text.len() <= 1024 * 1024)
                    .ok_or_else(|| {
                        ApiError::bad_request("startup_config_content must be text under 1 MiB")
                    })?;
                validated.insert(key.clone(), json!(text));
            }
            "extra_configs" if kind == "docker" => {
                let configs = validate_docker_extra_configs_api(Some(value), None)?;
                validated.insert(
                    key.clone(),
                    serde_json::to_value(configs).map_err(internal_json_error)?,
                );
            }
            _ => {
                return Err(ApiError::bad_request(format!(
                    "'{key}' is not valid for {kind}"
                )));
            }
        }
    }
    Ok(Value::Object(validated))
}

fn valid_container_path(path: &str) -> bool {
    path.starts_with('/') && !path.contains("..") && path.len() <= 512 && !path.contains('\0')
}

fn validate_docker_extra_configs_api(
    configs: Option<&Value>,
    startup_config_content: Option<&str>,
) -> Result<Vec<crate::runtime::DockerExtraConfig>, ApiError> {
    let configs = match configs {
        Some(configs) if !configs.is_null() => {
            serde_json::from_value::<Vec<crate::runtime::DockerExtraConfig>>(configs.clone())
                .map_err(|_| {
                    ApiError::bad_request("extra_configs must be an array of target/content files")
                })?
        }
        Some(_) | None => Vec::new(),
    };
    crate::runtime::validate_docker_extra_configs(&configs, startup_config_content)
        .map_err(|error| ApiError::bad_request(error.message()))?;
    Ok(configs)
}

async fn pull_docker_image(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let image = body
        .get("image")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|image| !image.is_empty())
        .ok_or_else(|| ApiError::bad_request("Docker image reference is required"))?;
    let image_id = state
        .runtime
        .pull_image(image)
        .await
        .map_err(runtime_api_error)?;
    let templates = base_runtime_templates(&state).await;
    let template = templates
        .into_iter()
        .find(|template| {
            template["template_type"] == "docker"
                && template["image"].as_str().is_some_and(|candidate| {
                    candidate == image
                        || (!image.contains(':') && candidate.starts_with(&format!("{image}:")))
                })
        })
        .ok_or_else(|| {
            ApiError::conflict("Docker pulled the image but did not expose a local template")
        })?;
    Ok(Json(json!({ "image_id": image_id, "template": template })))
}

async fn base_runtime_templates(state: &AppState) -> Vec<Value> {
    let mut templates = BuiltinRuntime::templates();
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
    templates
}

fn runtime_api_error(error: RuntimeError) -> ApiError {
    let status = match &error {
        RuntimeError::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        RuntimeError::NotFound(_) => StatusCode::NOT_FOUND,
        RuntimeError::Failed(_) => StatusCode::CONFLICT,
    };
    ApiError::new(status, error.message())
}

mod projects;

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
        Some(node_type) if BuiltinRuntime::is_type(node_type) => {
            create_builtin_node(state, project_id.to_string(), node).await
        }
        Some(node_type) => Err(ApiError::bad_request(format!(
            "Unsupported node type '{node_type}'. Executable node types are Docker, QEMU, VPCS, Dynamips, Ethernet switch, Ethernet hub, Cloud, and NAT."
        ))),
        None => Err(ApiError::bad_request("node_type is required")),
    }
}

async fn create_builtin_node(
    state: AppState,
    project_id: String,
    mut node: Value,
) -> Result<Response, ApiError> {
    let object = as_object_mut(&mut node, "node body must be an object")?;
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
    object.insert("node_id".into(), json!(node_id));
    object.insert("project_id".into(), json!(project_id.clone()));
    let node = BuiltinRuntime::normalize_node(node).map_err(ApiError::bad_request)?;
    BuiltinRuntime::validate_host_interfaces(&node).map_err(ApiError::bad_request)?;
    create_topology_object(state, &project_id, "node", "node_id", node).await
}

async fn create_node_from_template(
    State(state): State<AppState>,
    Path((project_id, template_id)): Path<(Uuid, Uuid)>,
    Json(usage): Json<Value>,
) -> Result<impl IntoResponse, ApiError> {
    let template_id_string = template_id.to_string();
    let template = if let Some(row) =
        sqlx::query("SELECT template_json FROM controller_templates WHERE template_id = ?")
            .bind(&template_id_string)
            .fetch_optional(state.store.pool())
            .await?
    {
        decode_json(row.get::<String, _>("template_json"))?
    } else {
        available_templates(&state)
            .await?
            .into_iter()
            .find(|template| {
                template.get("template_id").and_then(Value::as_str)
                    == Some(template_id_string.as_str())
            })
            .ok_or_else(|| ApiError::not_found(format!("Template {template_id} does not exist")))?
    };
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
    if BuiltinRuntime::is_type(node_type) {
        let mut node = template.clone();
        node["node_type"] = json!(node_type);
        node["name"] = json!(name);
        node["template_id"] = json!(template_id);
        node["x"] = usage.get("x").cloned().unwrap_or(json!(0));
        node["y"] = usage.get("y").cloned().unwrap_or(json!(0));
        for key in [
            "ports_mapping",
            "remote_console_host",
            "remote_console_port",
            "remote_console_type",
            "remote_console_http_path",
            "nat_interface",
        ] {
            if let Some(value) = usage.get(key) {
                node[key] = value.clone();
            }
        }
        if let Some(compute_id) = usage.get("compute_id") {
            node["compute_id"] = compute_id.clone();
        }
        return create_builtin_node(state, project_id.to_string(), node).await;
    }
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
            "environment": template["environment"],
            "extra_hosts": template["extra_hosts"],
            "extra_volumes": template["extra_volumes"],
            "extra_configs": usage
                .get("extra_configs")
                .filter(|value| !value.is_null())
                .cloned()
                .or_else(|| {
                    template
                        .get("extra_configs")
                        .filter(|value| !value.is_null())
                        .cloned()
                })
                .unwrap_or(json!([])),
            "startup_config_content": usage
                .get("startup_config_content")
                .filter(|value| !value.is_null())
                .cloned()
                .or_else(|| {
                    template
                        .get("startup_config_content")
                        .filter(|value| !value.is_null())
                        .cloned()
                })
                .unwrap_or(json!("")),
            "console_type": template["console_type"],
            "console_resolution": template["console_resolution"],
            "console_http_port": template["console_http_port"],
            "console_http_path": template["console_http_path"],
            "aux_type": template["aux_type"],
            "custom_adapters": template["custom_adapters"],
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
            "platform": template["platform"],
            "adapter_type": template["adapter_type"],
            "hda_disk_interface": template["hda_disk_interface"],
            "boot_priority": template["boot_priority"],
            "linked_clone": template["linked_clone"],
            "maxcpus": template
                .get("maxcpus")
                .filter(|value| !value.is_null())
                .cloned()
                .or_else(|| template.get("cpus").cloned())
                .unwrap_or(json!(1)),
            "hdb_disk_image": template["hdb_disk_image"],
            "hdb_disk_interface": template["hdb_disk_interface"],
            "hdb_disk_image_backing_file": template["hdb_disk_image_backing_file"],
            "hdc_disk_image": template["hdc_disk_image"],
            "hdc_disk_interface": template["hdc_disk_interface"],
            "hdc_disk_image_backing_file": template["hdc_disk_image_backing_file"],
            "hdd_disk_image": template["hdd_disk_image"],
            "hdd_disk_interface": template["hdd_disk_interface"],
            "hdd_disk_image_backing_file": template["hdd_disk_image_backing_file"],
            "hda_disk_image_backing_file": template["hda_disk_image_backing_file"],
            "disk_images_to_reset": template["disk_images_to_reset"],
            "cdrom_image": template["cdrom_image"],
            "bios_image": template["bios_image"],
            "kernel_image": template["kernel_image"],
            "initrd": template["initrd"],
            "kernel_command_line": template["kernel_command_line"],
            "console_type": template["console_type"],
            "aux_type": template["aux_type"],
            "on_close": template["on_close"],
            "process_priority": template["process_priority"],
            "options": template["options"],
            "tpm": template["tpm"],
            "uefi": template["uefi"],
            "replicate_network_connection_state": template["replicate_network_connection_state"],
            "create_config_disk": template["create_config_disk"],
            "cpu_throttling": template["cpu_throttling"],
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
            "startup_script": usage
                .get("startup_script")
                .filter(|value| !value.is_null())
                .cloned()
                .or_else(|| template.get("startup_script").cloned())
                .unwrap_or(json!("")),
            "base_script_file": template["base_script_file"],
            "console_type": template["console_type"],
            "console_auto_start": template["console_auto_start"],
            "x": usage.get("x").cloned().unwrap_or(json!(0)),
            "y": usage.get("y").cloned().unwrap_or(json!(0))
        })
    } else if node_type == "dynamips" {
        json!({
            "name": name,
            "node_type": "dynamips",
            "template_id": template_id,
            "platform": template["platform"],
            "chassis": template["chassis"],
            "slot0": template["slot0"],
            "image": template["image"],
            "adapters": template["adapters"],
            "ram": template["memory"],
            "nvram": template["nvram"],
            "dynamips_id": template["dynamips_id"],
            "idlepc": usage.get("idlepc").cloned().unwrap_or(template["idlepc"].clone()),
            "startup_config": usage
                .get("startup_config")
                .filter(|value| !value.is_null())
                .cloned()
                .or_else(|| template.get("startup_config").cloned())
                .unwrap_or(json!("")),
            "private_config": template["private_config"],
            "mmap": template["mmap"],
            "sparsemem": template["sparsemem"],
            "clock_divisor": template["clock_divisor"],
            "idlemax": template["idlemax"],
            "idlesleep": template["idlesleep"],
            "exec_area": template["exec_area"],
            "disk0": template["disk0"],
            "disk1": template["disk1"],
            "auto_delete_disks": template["auto_delete_disks"],
            "mac_addr": template["mac_addr"],
            "system_id": template["system_id"],
            "slot1": template["slot1"],
            "slot2": template["slot2"],
            "slot3": template["slot3"],
            "slot4": template["slot4"],
            "slot5": template["slot5"],
            "slot6": template["slot6"],
            "wic0": template["wic0"],
            "wic1": template["wic1"],
            "wic2": template["wic2"],
            "npe": template["npe"],
            "midplane": template["midplane"],
            "iomem": template["iomem"],
            "ghost_ios": template["ghost_ios"],
            "console_type": template["console_type"],
            "aux_type": template["aux_type"],
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
    set_default(object, "adapters", json!(1));
    set_default(object, "memory", json!(0));
    set_default(object, "cpus", json!(0));
    set_default(object, "properties", json!({}));
    set_default(object, "console_type", json!("telnet"));
    set_default(object, "console_resolution", json!("1024x768"));
    set_default(object, "console_http_port", json!(80));
    set_default(object, "console_http_path", json!("/"));
    set_default(object, "aux_type", json!("none"));
    set_default(object, "custom_adapters", json!([]));
    set_default(object, "extra_configs", json!([]));
    set_default(object, "startup_config_content", json!(""));

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
    let docker_options = DockerTemplateOptions {
        environment: object
            .get("environment")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        extra_hosts: object
            .get("extra_hosts")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        extra_volumes: object
            .get("extra_volumes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        extra_configs: validate_docker_extra_configs_api(
            object.get("extra_configs"),
            object.get("startup_config_content").and_then(Value::as_str),
        )?,
        startup_config_content: object
            .get("startup_config_content")
            .and_then(Value::as_str)
            .map(str::to_owned),
        console_type: object
            .get("console_type")
            .and_then(Value::as_str)
            .unwrap_or("telnet")
            .to_owned(),
        console_resolution: object
            .get("console_resolution")
            .and_then(Value::as_str)
            .unwrap_or("1024x768")
            .to_owned(),
        console_http_port: object
            .get("console_http_port")
            .and_then(Value::as_u64)
            .and_then(|port| u16::try_from(port).ok()),
        console_http_path: object
            .get("console_http_path")
            .and_then(Value::as_str)
            .map(str::to_owned),
        aux_type: object
            .get("aux_type")
            .and_then(Value::as_str)
            .unwrap_or("none")
            .to_owned(),
        custom_adapters: serde_json::from_value(
            object
                .get("custom_adapters")
                .cloned()
                .unwrap_or_else(|| json!([])),
        )
        .map_err(|_| {
            ApiError::bad_request("custom_adapters must be an array of adapter definitions")
        })?,
        ..DockerTemplateOptions::default()
    };
    let (container_id, image_digest) = state
        .runtime
        .create_container_with_options(
            &project_id,
            &node_id,
            &image,
            &name,
            start_command.as_deref(),
            u64::try_from(memory_mb).ok(),
            Some(cpus),
            &docker_options,
        )
        .await
        .map_err(runtime_api_error)?;
    object.insert("container_id".into(), json!(container_id));
    object.insert("image_digest".into(), json!(image_digest));
    if let Some(console_port) = state.runtime.console_port(&container_id).await {
        object.insert("console".into(), json!(console_port));
    }

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
    set_default(object, "adapters", json!(1));
    set_default(object, "memory", json!(512));
    set_default(object, "cpus", json!(1));
    let default_maxcpus = object.get("cpus").and_then(Value::as_u64).unwrap_or(1);
    set_default(object, "maxcpus", json!(default_maxcpus));
    set_default(object, "linked_clone", json!(true));
    set_default(object, "hda_disk_interface", json!("ide"));
    set_default(object, "boot_priority", json!("c"));
    set_default(object, "console_type", json!("telnet"));
    set_default(object, "aux_type", json!("none"));
    set_default(object, "on_close", json!("power_off"));
    set_default(object, "process_priority", json!("normal"));
    set_default(object, "options", json!(""));
    set_default(object, "tpm", json!(false));
    set_default(object, "uefi", json!(false));
    set_default(object, "replicate_network_connection_state", json!(true));
    set_default(object, "create_config_disk", json!(false));
    set_default(object, "cpu_throttling", json!(0));
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
    if !(1..=255).contains(&cpus) {
        return Err(ApiError::bad_request("QEMU cpus must be between 1 and 255"));
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
    if !(0..=275).contains(&adapters) {
        return Err(ApiError::bad_request(
            "QEMU adapters must be between 0 and 275",
        ));
    }
    let maxcpus = object
        .get("maxcpus")
        .and_then(Value::as_u64)
        .filter(|maxcpus| (1..=255).contains(maxcpus))
        .ok_or_else(|| ApiError::bad_request("QEMU maxcpus must be from 1 to 255"))?;
    if maxcpus < cpus as u64 {
        return Err(ApiError::bad_request(
            "QEMU maxcpus cannot be less than cpus",
        ));
    }
    let disk_images_to_reset = object
        .get("disk_images_to_reset")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .map(|value| {
                    value.as_str().map(str::to_owned).ok_or_else(|| {
                        ApiError::bad_request("disk_images_to_reset entries must be strings")
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let qemu_options = QemuTemplateOptions {
        platform: object
            .get("platform")
            .and_then(Value::as_str)
            .map(str::to_owned),
        adapter_type: object
            .get("adapter_type")
            .and_then(Value::as_str)
            .map(str::to_owned),
        hda_disk_interface: object
            .get("hda_disk_interface")
            .and_then(Value::as_str)
            .map(str::to_owned),
        boot_priority: object
            .get("boot_priority")
            .and_then(Value::as_str)
            .map(str::to_owned),
        linked_clone: object.get("linked_clone").and_then(Value::as_bool),
        maxcpus: Some(maxcpus as u32),
        hdb_disk_image: object
            .get("hdb_disk_image")
            .and_then(Value::as_str)
            .map(str::to_owned),
        hdb_disk_interface: object
            .get("hdb_disk_interface")
            .and_then(Value::as_str)
            .map(str::to_owned),
        hda_disk_image_backing_file: object
            .get("hda_disk_image_backing_file")
            .and_then(Value::as_str)
            .map(str::to_owned),
        hdb_disk_image_backing_file: object
            .get("hdb_disk_image_backing_file")
            .and_then(Value::as_str)
            .map(str::to_owned),
        hdc_disk_image: object
            .get("hdc_disk_image")
            .and_then(Value::as_str)
            .map(str::to_owned),
        hdc_disk_interface: object
            .get("hdc_disk_interface")
            .and_then(Value::as_str)
            .map(str::to_owned),
        hdc_disk_image_backing_file: object
            .get("hdc_disk_image_backing_file")
            .and_then(Value::as_str)
            .map(str::to_owned),
        hdd_disk_image: object
            .get("hdd_disk_image")
            .and_then(Value::as_str)
            .map(str::to_owned),
        hdd_disk_interface: object
            .get("hdd_disk_interface")
            .and_then(Value::as_str)
            .map(str::to_owned),
        hdd_disk_image_backing_file: object
            .get("hdd_disk_image_backing_file")
            .and_then(Value::as_str)
            .map(str::to_owned),
        disk_images_to_reset,
        cdrom_image: object
            .get("cdrom_image")
            .and_then(Value::as_str)
            .map(str::to_owned),
        bios_image: object
            .get("bios_image")
            .and_then(Value::as_str)
            .map(str::to_owned),
        kernel_image: object
            .get("kernel_image")
            .and_then(Value::as_str)
            .map(str::to_owned),
        initrd: object
            .get("initrd")
            .and_then(Value::as_str)
            .map(str::to_owned),
        kernel_command_line: object
            .get("kernel_command_line")
            .and_then(Value::as_str)
            .map(str::to_owned),
        console_type: object
            .get("console_type")
            .and_then(Value::as_str)
            .map(str::to_owned),
        aux_type: object
            .get("aux_type")
            .and_then(Value::as_str)
            .map(str::to_owned),
        on_close: object
            .get("on_close")
            .and_then(Value::as_str)
            .map(str::to_owned),
        process_priority: object
            .get("process_priority")
            .and_then(Value::as_str)
            .map(str::to_owned),
        options: object
            .get("options")
            .and_then(Value::as_str)
            .map(str::to_owned),
        tpm: object.get("tpm").and_then(Value::as_bool),
        uefi: object.get("uefi").and_then(Value::as_bool),
        replicate_network_connection_state: object
            .get("replicate_network_connection_state")
            .and_then(Value::as_bool),
        create_config_disk: object.get("create_config_disk").and_then(Value::as_bool),
        cpu_throttling: object
            .get("cpu_throttling")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok()),
    };
    object.remove("disk_images_to_reset");
    let vm = state
        .qemu
        .create_vm_with_options(
            &project_id,
            &node_id,
            &image,
            memory_mb as u64,
            cpus as u32,
            adapters as u32,
            &qemu_options,
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
    set_default(object, "base_script_file", json!("vpcs_base_config.txt"));
    set_default(object, "console_type", json!("telnet"));
    set_default(object, "console_auto_start", json!(false));
    set_default(object, "properties", json!({}));
    if object
        .get("console_type")
        .and_then(Value::as_str)
        .is_some_and(|value| !matches!(value, "telnet" | "ssh" | "none"))
    {
        return Err(ApiError::bad_request(
            "VPCS console_type must be telnet, ssh, or none",
        ));
    }
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
    let chassis = object
        .get("chassis")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| match platform.as_str() {
            "c3600" => Some("3640".to_owned()),
            "c2600" => Some("2610".to_owned()),
            "c1700" => Some("1760".to_owned()),
            _ => None,
        });
    let slot0 = object
        .get("slot0")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let ram_mb = object
        .get("ram")
        .and_then(Value::as_u64)
        .unwrap_or(match platform.as_str() {
            "c1700" | "c2600" => 160,
            "c2691" | "c3600" => 192,
            "c3725" => 128,
            "c3745" => 256,
            _ => 512,
        });
    let nvram = object
        .get("nvram")
        .and_then(Value::as_u64)
        .unwrap_or(match platform.as_str() {
            "c2691" | "c3725" | "c3745" => 256,
            "c7200" => 512,
            _ => 128,
        });
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

    let existing_nodes = sqlx::query(
        "SELECT object_json FROM topology_objects WHERE project_id = ? AND kind = 'node'",
    )
    .bind(&project_id)
    .fetch_all(state.store.pool())
    .await?;
    let used_ids = existing_nodes
        .iter()
        .filter_map(|row| decode_json(row.get::<String, _>("object_json")).ok())
        .filter(|node| node["node_type"].as_str() == Some("dynamips"))
        .filter_map(|node| node["dynamips_id"].as_u64())
        .collect::<HashSet<_>>();
    let dynamips_id = object
        .get("dynamips_id")
        .and_then(Value::as_u64)
        .or_else(|| (1..=4096).find(|identifier| !used_ids.contains(identifier)))
        .filter(|identifier| (1..=4096).contains(identifier))
        .ok_or_else(|| ApiError::conflict("No free Dynamips internal identifier is available"))?
        as u32;
    let dynamips_options = dynamips_options_from_template(object, dynamips_id)?;

    object.insert("name".into(), json!(name));
    object.insert("node_id".into(), json!(node_id));
    object.insert("project_id".into(), json!(project_id.clone()));
    object.insert("node_type".into(), json!("dynamips"));
    object.insert("platform".into(), json!(platform));
    object.insert("chassis".into(), json!(chassis));
    object.insert("image".into(), json!(image));
    object.insert("ram".into(), json!(ram_mb));
    object.insert("nvram".into(), json!(nvram));
    object.insert("idlepc".into(), json!(idle_pc));
    object.insert("startup_config".into(), json!(startup_config));
    object.insert("dynamips_id".into(), json!(dynamips_id));
    object.insert(
        "private_config".into(),
        json!(dynamips_options.private_config),
    );
    object.insert("mmap".into(), json!(dynamips_options.mmap));
    object.insert("sparsemem".into(), json!(dynamips_options.sparsemem));
    object.insert(
        "clock_divisor".into(),
        json!(dynamips_options.clock_divisor),
    );
    object.insert("idlemax".into(), json!(dynamips_options.idlemax));
    object.insert("idlesleep".into(), json!(dynamips_options.idlesleep));
    object.insert("exec_area".into(), json!(dynamips_options.exec_area));
    object.insert("disk0".into(), json!(dynamips_options.disk0));
    object.insert("disk1".into(), json!(dynamips_options.disk1));
    object.insert(
        "auto_delete_disks".into(),
        json!(dynamips_options.auto_delete_disks),
    );
    object.insert("ghost_ios".into(), json!(dynamips_options.ghost_ios));
    object.insert("console_type".into(), json!(dynamips_options.console_type));
    object.insert("aux_type".into(), json!(dynamips_options.aux_type));
    for (index, slot) in dynamips_options.slots.iter().enumerate() {
        object.insert(format!("slot{index}"), json!(slot));
    }
    for (index, wic) in dynamips_options.wics.iter().enumerate() {
        object.insert(format!("wic{index}"), json!(wic));
    }
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
        .create_router_with_options(
            &project_id,
            &node_id.to_string(),
            &name,
            &image,
            &platform,
            chassis.as_deref(),
            slot0.as_deref(),
            ram_mb,
            nvram,
            idle_pc.as_deref(),
            &startup_config,
            &dynamips_options,
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

fn dynamips_options_from_template(
    object: &mut Map<String, Value>,
    dynamips_id: u32,
) -> Result<DynamipsOptions, ApiError> {
    // Appliance templates do not carry a runtime ID. `json!` in
    // `create_node_from_template` turns that absent field into `null`, which
    // fails to deserialize as `DynamipsOptions::dynamips_id: u32` before the
    // allocator's value can be applied. Normalize it before deserializing.
    object.insert("dynamips_id".into(), json!(dynamips_id));
    if object.get("sparsemem").is_none() {
        let platform = object
            .get("platform")
            .and_then(Value::as_str)
            .unwrap_or("c7200");
        object.insert(
            "sparsemem".into(),
            json!(!matches!(platform, "c1700" | "c2600")),
        );
    }
    let mut options: DynamipsOptions = serde_json::from_value(Value::Object(object.clone()))
        .map_err(|error| {
            ApiError::bad_request(format!("Dynamips runtime settings are invalid: {error}"))
        })?;
    options.dynamips_id = dynamips_id;
    options.mac_addr = options
        .mac_addr
        .take()
        .filter(|value| !value.trim().is_empty());
    options.system_id = options
        .system_id
        .take()
        .filter(|value| !value.trim().is_empty());
    options.slots = (0..7)
        .map(|index| {
            object
                .get(format!("slot{index}").as_str())
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    options.wics = (0..3)
        .map(|index| {
            object
                .get(format!("wic{index}").as_str())
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    Ok(options)
}

async fn create_link(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Json(mut link): Json<Value>,
) -> Result<impl IntoResponse, ApiError> {
    let project_id = project_id.to_string();
    let _project_lock = state.lock_project_operations(&project_id).await;
    let object = as_object_mut(&mut link, "link body must be an object")?;
    let link_id = parse_or_generate_uuid(object.get("link_id"), "link_id")?;
    let endpoints = object
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::bad_request("link must contain a nodes array"))?;
    if endpoints.len() != 2 {
        return Err(ApiError::bad_request(
            "GNS3 links connect exactly two endpoints; use a switch or hub for shared media",
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
    set_default(object, "filters", json!({}));
    set_default(object, "link_style", json!({}));
    object.insert("capturing".into(), json!(false));
    let filters = validate_link_filters(object.get("filters").unwrap_or(&Value::Null))?;
    object.insert("filters".into(), filters);
    if !object.get("suspend").is_some_and(Value::is_boolean) {
        return Err(ApiError::bad_request("suspend must be a boolean"));
    }
    if !object.get("link_style").is_some_and(Value::is_object) {
        return Err(ApiError::bad_request("link_style must be an object"));
    }
    object.insert("capture_file_name".into(), Value::Null);
    object.insert("capture_file_path".into(), Value::Null);
    object.insert("capture_endpoint_id".into(), Value::Null);
    object.insert("wireshark".into(), json!(false));
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
    let endpoints = link["nodes"].as_array_mut().expect("validated nodes array");
    for (endpoint, runtime_endpoint) in endpoints.iter_mut().zip(&runtime_endpoints) {
        if let Some(object) = endpoint.as_object_mut() {
            object.retain(|key, _| !key.starts_with("runtime_"));
        }
        endpoint["node_type"] = runtime_endpoint["node_type"].clone();
    }
    let mut should_attach = Vec::with_capacity(runtime_endpoints.len());
    for runtime_endpoint in &runtime_endpoints {
        should_attach.push(
            runtime_endpoint_status(&state, &project_id, runtime_endpoint).await? != "stopped",
        );
    }
    for endpoint in link["nodes"].as_array_mut().expect("validated nodes array") {
        if endpoint["node_type"].as_str() == Some("vpcs")
            && let Some(node_id) = endpoint["node_id"].as_str().map(str::to_owned)
            && let (Some(adapter_port), Some(runtime_port)) = (
                state.vpcs.adapter_udp_port(&node_id).await,
                state.vpcs.active_udp_port(&node_id).await,
            )
        {
            endpoint["runtime_adapter_port"] = json!(adapter_port);
            endpoint["runtime_udp_port"] = json!(runtime_port);
        }
    }
    let engine = LinkEngine::new(state.ubridge.clone());
    engine
        .reserve_pair(&mut link)
        .await
        .map_err(runtime_api_error)?;
    link["filter_endpoint_id"] = link["nodes"][0]["runtime_endpoint_id"].clone();
    for endpoint in link["nodes"].as_array_mut().expect("validated nodes array") {
        if let Some(object) = endpoint.as_object_mut() {
            object.remove("node_type");
        }
    }
    let mut attached = Vec::new();
    for ((endpoint, runtime_endpoint), should_attach) in link["nodes"]
        .as_array()
        .expect("validated nodes array")
        .iter()
        .zip(&runtime_endpoints)
        .zip(should_attach)
    {
        if should_attach {
            if let Err(error) =
                attach_runtime_link_endpoint(&state, &project_id, &link, endpoint, runtime_endpoint)
                    .await
            {
                for (attached_endpoint, attached_runtime) in attached.iter().rev() {
                    let _ = detach_runtime_link_endpoint(
                        &state,
                        &project_id,
                        attached_endpoint,
                        attached_runtime,
                    )
                    .await;
                }
                engine.release_link_ports(&link).await;
                return Err(error);
            }
            attached.push((endpoint.clone(), runtime_endpoint.clone()));
        }
    }
    match create_topology_object(state.clone(), &project_id, "link", "link_id", link.clone()).await
    {
        Ok(response) => Ok(response),
        Err(error) => {
            for endpoint in link["nodes"].as_array().expect("validated nodes array") {
                let node_id = endpoint["node_id"].as_str().unwrap_or_default();
                if let Some(runtime_endpoint) = runtime_endpoints
                    .iter()
                    .find(|candidate| candidate["node_id"].as_str() == Some(node_id))
                {
                    let _ = detach_runtime_link_endpoint(
                        &state,
                        &project_id,
                        endpoint,
                        runtime_endpoint,
                    )
                    .await;
                }
            }
            engine.release_link_ports(&link).await;
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
        let builtin_ports = node["ports_mapping"]
            .as_array()
            .map(Vec::len)
            .unwrap_or_default() as u64;
        let port_capacity = if BuiltinRuntime::is_type(node_type) {
            builtin_ports
        } else if node_type == "docker" || node_type == "qemu" {
            adapters
        } else {
            1
        };
        let valid_endpoint = match node_type {
            "docker" => adapter == 0 && port < adapters,
            "qemu" => adapter < adapters && port == 0,
            "vpcs" | "dynamips" => adapter == 0 && port == 0,
            "ethernet_switch" | "ethernet_hub" | "cloud" => adapter == 0 && port < builtin_ports,
            "nat" => adapter == 0 && port == 0 && builtin_ports == 1,
            _ => {
                return Err(ApiError::bad_request(format!(
                    "Node type '{node_type}' has no Ethernet runtime"
                )));
            }
        };
        if !valid_endpoint {
            return Err(ApiError::bad_request(format!(
                "Port {adapter}/{port} is outside the configured {port_capacity} {node_type} ports"
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

async fn load_topology_node(
    pool: &SqlitePool,
    project_id: &str,
    node_id: &str,
) -> Result<Value, ApiError> {
    let row = sqlx::query(
        "SELECT object_json FROM topology_objects
         WHERE project_id = ? AND kind = 'node' AND object_id = ?",
    )
    .bind(project_id)
    .bind(node_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| ApiError::not_found(format!("Node {node_id} does not exist")))?;
    decode_json(row.get::<String, _>("object_json"))
}

async fn runtime_endpoint_status(
    state: &AppState,
    project_id: &str,
    endpoint: &Value,
) -> Result<String, ApiError> {
    let node_id = endpoint["node_id"].as_str().unwrap_or_default();
    let node = load_topology_node(state.store.pool(), project_id, node_id).await?;
    let node_type = endpoint["node_type"].as_str().unwrap_or_default();
    if BuiltinRuntime::is_type(node_type) {
        return Ok(node["status"].as_str().unwrap_or("started").to_owned());
    }
    let status = match node_type {
        "docker" => {
            state
                .runtime
                .container_status(endpoint["container_id"].as_str().unwrap_or_default())
                .await
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
                "Unsupported node type '{node_type}'"
            )));
        }
    }
    .map_err(runtime_api_error)?;
    Ok(status)
}

async fn attach_runtime_link_endpoint(
    state: &AppState,
    project_id: &str,
    link: &Value,
    endpoint: &Value,
    runtime_endpoint: &Value,
) -> Result<(), ApiError> {
    let node_id = endpoint["node_id"].as_str().unwrap_or_default();
    let node = load_topology_node(state.store.pool(), project_id, node_id).await?;
    let engine = LinkEngine::new(state.ubridge.clone());
    let filters = link_filters_for_endpoint(link, endpoint);
    let suspended = link["suspend"].as_bool().unwrap_or(false);
    if matches!(
        runtime_endpoint["node_type"].as_str(),
        Some("cloud" | "nat")
    ) {
        BuiltinRuntime::validate_host_interfaces(&node).map_err(ApiError::bad_request)?;
    }
    let endpoint_id = endpoint["runtime_endpoint_id"].as_str().unwrap_or_default();
    let relay_already_exists = state.ubridge.has_link_endpoint(endpoint_id).await;
    let capture_selected = link["capturing"].as_bool().unwrap_or(false)
        && link["capture_endpoint_id"].as_str() == Some(endpoint_id);
    let capture_needed = capture_selected && !relay_already_exists;
    let mut endpoint = endpoint.clone();
    match runtime_endpoint["node_type"].as_str().unwrap_or_default() {
        "docker" => {
            let adapter = state
                .runtime
                .adapter_nio(&endpoint)
                .map_err(runtime_api_error)?;
            let AdapterNio::Ethernet {
                host_interface,
                guest_interface,
            } = adapter.clone()
            else {
                return Err(ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Docker did not provide an Ethernet NIO",
                ));
            };
            let container_id = runtime_endpoint["container_id"]
                .as_str()
                .unwrap_or_default();
            let endpoint_id = endpoint["runtime_endpoint_id"].as_str().unwrap_or_default();
            let setup = async {
                if !state.ubridge.has_link_endpoint(endpoint_id).await {
                    let pid = state
                        .runtime
                        .container_pid(container_id)
                        .await
                        .map_err(runtime_api_error)?;
                    let _ = state.ubridge.delete_veth(&host_interface).await;
                    state
                        .ubridge
                        .create_veth_pair(&host_interface, &guest_interface)
                        .await
                        .map_err(runtime_api_error)?;
                    let port = endpoint["port_number"].as_u64().unwrap_or_default();
                    let mac = docker_interface_mac(node_id, port)?;
                    state
                        .ubridge
                        .set_docker_mac_address(&guest_interface, &mac)
                        .await
                        .map_err(runtime_api_error)?;
                    state
                        .ubridge
                        .set_interface_up(&guest_interface)
                        .await
                        .map_err(runtime_api_error)?;
                    state
                        .ubridge
                        .set_interface_up(&host_interface)
                        .await
                        .map_err(runtime_api_error)?;
                    state
                        .ubridge
                        .move_veth_to_container(&guest_interface, pid, &guest_interface)
                        .await
                        .map_err(runtime_api_error)?;
                }
                engine
                    .attach_endpoint(&endpoint, adapter, &filters, suspended, None)
                    .await
                    .map_err(runtime_api_error)
            }
            .await;
            if let Err(error) = setup {
                let _ = state.ubridge.delete_veth(&host_interface).await;
                return Err(error);
            }
            endpoint["runtime_host_veth"] = json!(host_interface);
        }
        "qemu" => {
            let vm = state.qemu.vm_from_node(&node).map_err(runtime_api_error)?;
            let adapter = state
                .qemu
                .adapter_nio(&endpoint)
                .map_err(runtime_api_error)?;
            let qemu_link = qemu_link_from_endpoint(&endpoint)?;
            attach_runtime_nio_with_rollback(
                !relay_already_exists,
                || async {
                    state
                        .qemu
                        .attach_nio(&vm, &qemu_link)
                        .await
                        .map_err(runtime_api_error)
                },
                || async {
                    engine
                        .attach_endpoint(&endpoint, adapter, &filters, suspended, None)
                        .await
                        .map_err(runtime_api_error)
                },
                || async {
                    if !relay_already_exists {
                        let _ = state.qemu.detach_nio(&vm, &qemu_link).await;
                    }
                },
            )
            .await?;
        }
        "vpcs" => {
            if let Some(port) = state.vpcs.adapter_udp_port(node_id).await {
                endpoint["runtime_adapter_port"] = json!(port);
            }
            if let Some(port) = state.vpcs.active_udp_port(node_id).await {
                endpoint["runtime_udp_port"] = json!(port);
            }
            let adapter = state
                .vpcs
                .adapter_nio(&endpoint)
                .map_err(runtime_api_error)?;
            engine
                .attach_endpoint(&endpoint, adapter, &filters, suspended, None)
                .await
                .map_err(runtime_api_error)?;
        }
        "dynamips" => {
            let vm = state
                .dynamips
                .vm_from_node(&node)
                .map_err(runtime_api_error)?;
            let adapter = state
                .dynamips
                .adapter_nio(&endpoint)
                .map_err(runtime_api_error)?;
            let AdapterNio::Udp {
                adapter_port,
                runtime_port,
            } = adapter
            else {
                unreachable!()
            };
            let adapter_number = endpoint["adapter_number"].as_u64().unwrap_or_default() as u32;
            let port_number = endpoint["port_number"].as_u64().unwrap_or_default() as u32;
            attach_runtime_nio_with_rollback(
                !relay_already_exists,
                || async {
                    state
                        .dynamips
                        .attach_nio(&vm, adapter_number, port_number, runtime_port, adapter_port)
                        .await
                        .map_err(runtime_api_error)
                },
                || async {
                    engine
                        .attach_endpoint(&endpoint, adapter, &filters, suspended, None)
                        .await
                        .map_err(runtime_api_error)
                },
                || async {
                    if !relay_already_exists {
                        let _ = state
                            .dynamips
                            .detach_nio(&vm, adapter_number, port_number)
                            .await;
                    }
                },
            )
            .await?;
        }
        "ethernet_switch" | "ethernet_hub" | "cloud" | "nat" => {
            let adapter =
                BuiltinRuntime::adapter_nio(&node, &endpoint).map_err(runtime_api_error)?;
            state
                .builtin
                .attach_switch_port(&node, &endpoint)
                .await
                .map_err(runtime_api_error)?;
            if let Err(error) = engine
                .attach_endpoint(&endpoint, adapter, &filters, suspended, None)
                .await
            {
                state
                    .builtin
                    .detach_switch_port(
                        node_id,
                        endpoint["port_number"].as_u64().unwrap_or_default() as u16,
                    )
                    .await;
                return Err(runtime_api_error(error));
            }
        }
        node_type => {
            return Err(ApiError::bad_request(format!(
                "Unsupported node type '{node_type}'"
            )));
        }
    }
    if capture_needed {
        let capture_file_name = safe_capture_file_name(
            link["capture_file_name"]
                .as_str()
                .ok_or_else(|| ApiError::conflict("Capturing link has no capture file name"))?,
        )?;
        let stored_path = link["capture_file_path"]
            .as_str()
            .ok_or_else(|| ApiError::conflict("Capturing link has no capture file path"))?;
        let capture_path =
            canonical_capture_file_path(project_id, &capture_file_name, Some(stored_path), false)?;
        engine
            .start_capture(&endpoint, &capture_path.to_string_lossy())
            .await
            .map_err(runtime_api_error)?;
    }
    Ok(())
}

async fn attach_runtime_nio_with_rollback<
    Attach,
    AttachFuture,
    Relay,
    RelayFuture,
    Cleanup,
    CleanupFuture,
>(
    cleanup_runtime_on_relay_failure: bool,
    attach: Attach,
    attach_relay: Relay,
    cleanup: Cleanup,
) -> Result<(), ApiError>
where
    Attach: FnOnce() -> AttachFuture,
    AttachFuture: std::future::Future<Output = Result<(), ApiError>>,
    Relay: FnOnce() -> RelayFuture,
    RelayFuture: std::future::Future<Output = Result<(), ApiError>>,
    Cleanup: FnOnce() -> CleanupFuture,
    CleanupFuture: std::future::Future<Output = ()>,
{
    attach().await?;
    if let Err(error) = attach_relay().await {
        if cleanup_runtime_on_relay_failure {
            cleanup().await;
        }
        return Err(error);
    }
    Ok(())
}

fn link_filters_for_endpoint(link: &Value, endpoint: &Value) -> Value {
    let selected = link["filter_endpoint_id"].as_str().or_else(|| {
        link["nodes"]
            .as_array()
            .and_then(|nodes| nodes.first())
            .and_then(|first| first["runtime_endpoint_id"].as_str())
    });
    if selected == endpoint["runtime_endpoint_id"].as_str() {
        link.get("filters").cloned().unwrap_or_else(|| json!({}))
    } else {
        json!({})
    }
}

async fn detach_runtime_link_endpoint(
    state: &AppState,
    project_id: &str,
    endpoint: &Value,
    runtime_endpoint: &Value,
) -> Result<(), ApiError> {
    let node_id = endpoint["node_id"].as_str().unwrap_or_default();
    let engine = LinkEngine::new(state.ubridge.clone());
    let engine_result = engine.detach_endpoint(endpoint).await;
    match runtime_endpoint["node_type"].as_str().unwrap_or_default() {
        "docker" => {
            let host = endpoint["runtime_host_veth"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| {
                    endpoint["runtime_endpoint_id"]
                        .as_str()
                        .and_then(|id| ubridge::host_veth_name(id).ok())
                });
            if let Some(host) = host {
                let _ = state.ubridge.delete_veth(&host).await;
            }
        }
        "qemu" => {
            let node = load_topology_node(state.store.pool(), project_id, node_id).await?;
            let vm = state.qemu.vm_from_node(&node).map_err(runtime_api_error)?;
            if state.qemu.status(&vm).await.map_err(runtime_api_error)? != "stopped" {
                if let Ok(link) = qemu_link_from_endpoint(endpoint) {
                    let _ = state.qemu.detach_nio(&vm, &link).await;
                }
            }
        }
        "vpcs" => {}
        "dynamips" => {
            let node = load_topology_node(state.store.pool(), project_id, node_id).await?;
            let vm = state
                .dynamips
                .vm_from_node(&node)
                .map_err(runtime_api_error)?;
            let _ = state
                .dynamips
                .detach_nio(
                    &vm,
                    endpoint["adapter_number"].as_u64().unwrap_or_default() as u32,
                    endpoint["port_number"].as_u64().unwrap_or_default() as u32,
                )
                .await;
        }
        "ethernet_switch" | "ethernet_hub" => {
            state
                .builtin
                .detach_switch_port(
                    node_id,
                    endpoint["port_number"].as_u64().unwrap_or_default() as u16,
                )
                .await;
        }
        _ => {}
    }
    engine_result.map_err(runtime_api_error)
}

fn qemu_link_from_endpoint(endpoint: &Value) -> Result<QemuLink, ApiError> {
    Ok(QemuLink {
        link_id: endpoint["runtime_endpoint_id"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        adapter_number: endpoint["adapter_number"].as_u64().unwrap_or_default() as u32,
        adapter_port: endpoint["runtime_adapter_port"]
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .ok_or_else(|| ApiError::conflict("QEMU link has no adapter UDP port"))?,
        runtime_port: endpoint["runtime_udp_port"]
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .ok_or_else(|| ApiError::conflict("QEMU link has no runtime UDP port"))?,
    })
}

async fn node_link_entries(
    state: &AppState,
    project_id: &str,
    node_id: &str,
) -> Result<Vec<(Value, Value, Value)>, ApiError> {
    let links = list_objects(state.store.pool(), project_id, "link")
        .await?
        .0;
    let mut entries = Vec::new();
    for link in links {
        let Some(endpoint) = link
            .get("nodes")
            .and_then(Value::as_array)
            .and_then(|nodes| {
                nodes
                    .iter()
                    .find(|endpoint| endpoint["node_id"].as_str() == Some(node_id))
            })
            .cloned()
        else {
            continue;
        };
        let runtime_endpoint = link_runtime_endpoints(state.store.pool(), project_id, &link)
            .await?
            .into_iter()
            .find(|candidate| candidate["node_id"].as_str() == Some(node_id))
            .ok_or_else(|| {
                ApiError::conflict(format!("Link endpoint for {node_id} has no runtime"))
            })?;
        entries.push((link, endpoint, runtime_endpoint));
    }
    Ok(entries)
}

async fn attach_node_links(
    state: &AppState,
    project_id: &str,
    node_id: &str,
    before_start: bool,
) -> Result<(), ApiError> {
    let entries = node_link_entries(state, project_id, node_id).await?;
    for (link, endpoint, runtime_endpoint) in entries {
        let node_type = runtime_endpoint["node_type"].as_str().unwrap_or_default();
        if before_start && node_type == "vpcs" {
            let adapter_port = endpoint["runtime_adapter_port"]
                .as_u64()
                .and_then(|port| u16::try_from(port).ok())
                .ok_or_else(|| ApiError::conflict("VPCS link has no adapter UDP port"))?;
            let runtime_port = endpoint["runtime_udp_port"]
                .as_u64()
                .and_then(|port| u16::try_from(port).ok())
                .ok_or_else(|| ApiError::conflict("VPCS link has no runtime UDP port"))?;
            state
                .vpcs
                .set_nio_ports(node_id, adapter_port, runtime_port)
                .await;
        }
        if before_start && node_type == "dynamips" {
            // Dynamips binds its UDP NIO through the hypervisor before the IOS
            // process starts. The other runtimes are attached after startup.
            attach_runtime_link_endpoint(state, project_id, &link, &endpoint, &runtime_endpoint)
                .await?;
        }
        if !before_start && node_type != "dynamips" {
            attach_runtime_link_endpoint(state, project_id, &link, &endpoint, &runtime_endpoint)
                .await?;
        }
    }
    Ok(())
}

async fn detach_node_links(
    state: &AppState,
    project_id: &str,
    node_id: &str,
) -> Result<(), ApiError> {
    let entries = node_link_entries(state, project_id, node_id).await?;
    let mut first_error = None;
    for (_link, endpoint, runtime_endpoint) in entries.into_iter().rev() {
        if let Err(error) =
            detach_runtime_link_endpoint(state, project_id, &endpoint, &runtime_endpoint).await
        {
            first_error.get_or_insert(error);
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    Ok(())
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
    let node_id = node
        .get("node_id")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::conflict("Node has no node ID"))?;
    let before_stop = matches!(&action, NodeAction::Stop) && !BuiltinRuntime::is_type(node_type)
        || (matches!(&action, NodeAction::Reload) && matches!(node_type, "docker" | "vpcs"));
    if before_stop {
        detach_node_links(state, project_id, node_id).await?;
    }
    match node_type {
        "docker" => {
            let container_id = node
                .get("container_id")
                .and_then(Value::as_str)
                .ok_or_else(|| ApiError::conflict("Docker node has no runtime container"))?;
            let status = state
                .runtime
                .lifecycle(container_id, action)
                .await
                .map_err(runtime_api_error)?;
            if matches!(action, NodeAction::Start | NodeAction::Reload) {
                if let Err(error) = attach_node_links(state, project_id, node_id, false).await {
                    let _ = detach_node_links(state, project_id, node_id).await;
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
            let status = state
                .qemu
                .lifecycle(&vm, action, &[])
                .await
                .map_err(runtime_api_error)?;
            if matches!(&action, NodeAction::Start) {
                if let Err(error) = attach_node_links(state, project_id, node_id, false).await {
                    let _ = detach_node_links(state, project_id, node_id).await;
                    let _ = state.qemu.lifecycle(&vm, NodeAction::Stop, &[]).await;
                    return Err(error);
                }
            }
            Ok(status)
        }
        "vpcs" => {
            let name = node
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| ApiError::conflict("VPCS node has no name"))?;
            if matches!(action, NodeAction::Start | NodeAction::Reload) {
                for (_link, endpoint, _runtime) in
                    node_link_entries(state, project_id, node_id).await?
                {
                    if let (Some(adapter_port), Some(runtime_port)) = (
                        endpoint["runtime_adapter_port"]
                            .as_u64()
                            .and_then(|port| u16::try_from(port).ok()),
                        endpoint["runtime_udp_port"]
                            .as_u64()
                            .and_then(|port| u16::try_from(port).ok()),
                    ) {
                        state
                            .vpcs
                            .set_nio_ports(node_id, adapter_port, runtime_port)
                            .await;
                    }
                }
            }
            let startup_script = if matches!(action, NodeAction::Start | NodeAction::Reload) {
                vpcs_startup_script(state.store.pool(), project_id, node).await?
            } else {
                None
            };
            let status = state
                .vpcs
                .lifecycle(project_id, node_id, name, startup_script.as_deref(), action)
                .await
                .map_err(runtime_api_error)?;
            if matches!(action, NodeAction::Start | NodeAction::Reload) {
                if let Err(error) = attach_node_links(state, project_id, node_id, false).await {
                    let _ = detach_node_links(state, project_id, node_id).await;
                    let _ = state
                        .vpcs
                        .lifecycle(project_id, node_id, name, None, NodeAction::Stop)
                        .await;
                    return Err(error);
                }
            }
            Ok(status)
        }
        "dynamips" => {
            let vm = state
                .dynamips
                .vm_from_node(node)
                .map_err(runtime_api_error)?;
            if matches!(&action, NodeAction::Start) {
                attach_node_links(state, project_id, node_id, true).await?;
            }
            let status = state
                .dynamips
                .lifecycle(&vm, action)
                .await
                .map_err(runtime_api_error)?;
            if matches!(&action, NodeAction::Start)
                && let Err(error) = attach_node_links(state, project_id, node_id, false).await
            {
                let _ = detach_node_links(state, project_id, node_id).await;
                let _ = state.dynamips.lifecycle(&vm, NodeAction::Stop).await;
                return Err(error);
            }
            Ok(status)
        }
        "ethernet_switch" | "ethernet_hub" => Err(ApiError::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "Ethernet switches and hubs are always running",
        )),
        "cloud" | "nat" => match action {
            NodeAction::Start => {
                attach_node_links(state, project_id, node_id, false).await?;
                Ok("started".to_owned())
            }
            NodeAction::Stop | NodeAction::Reload | NodeAction::Suspend => Err(ApiError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                "Cloud and NAT nodes cannot be stopped, reloaded, or suspended",
            )),
        },
        _ => Err(ApiError::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "This node has no implemented runtime adapter",
        )),
    }
}

async fn vpcs_startup_script(
    pool: &SqlitePool,
    project_id: &str,
    node: &Value,
) -> Result<Option<String>, ApiError> {
    if let Some(script) = node
        .get("startup_script")
        .and_then(Value::as_str)
        .filter(|script| !script.is_empty())
    {
        return Ok(Some(script.to_owned()));
    }
    let Some(base_script_file) = node
        .get("base_script_file")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
    else {
        return Ok(None);
    };
    let path = FsPath::new(base_script_file);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || base_script_file.contains('\\')
    {
        return Err(ApiError::bad_request(
            "VPCS base_script_file must be a relative project file path",
        ));
    }
    let normalized = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    let row =
        sqlx::query("SELECT contents FROM project_files WHERE project_id = ? AND file_path = ?")
            .bind(project_id)
            .bind(normalized)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|row| String::from_utf8_lossy(&row.get::<Vec<u8>, _>("contents")).into_owned()))
}

async fn stop_runtime_node(
    state: &AppState,
    project_id: &str,
    node: &Value,
) -> Result<(), ApiError> {
    let Some(node_type) = node.get("node_type").and_then(Value::as_str) else {
        return Ok(());
    };
    if BuiltinRuntime::is_type(node_type) {
        let node_id = node
            .get("node_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        state.builtin.stop_node(node_id).await;
        return Ok(());
    }
    if node_type != "docker"
        && node_type != "qemu"
        && node_type != "vpcs"
        && node_type != "dynamips"
    {
        return Ok(());
    }
    match node_type {
        "docker" if node.get("container_id").and_then(Value::as_str).is_none() => return Ok(()),
        "qemu"
            if node
                .get("runtime_overlay")
                .and_then(Value::as_str)
                .is_none() =>
        {
            return Ok(());
        }
        "dynamips"
            if node
                .get("runtime_working_dir")
                .and_then(Value::as_str)
                .is_none()
                || node
                    .get("runtime_hypervisor_port")
                    .and_then(Value::as_u64)
                    .is_none() =>
        {
            return Ok(());
        }
        _ => {}
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
        "docker" => match node.get("container_id").and_then(Value::as_str) {
            Some(container_id) => state
                .runtime
                .remove_node_resources(node_id, Some(container_id))
                .await
                .map_err(runtime_api_error),
            None => Ok(()),
        },
        "qemu" => {
            if node
                .get("runtime_overlay")
                .and_then(Value::as_str)
                .is_none()
            {
                return Ok(());
            }
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
            if node
                .get("runtime_working_dir")
                .and_then(Value::as_str)
                .is_none()
                || node
                    .get("runtime_hypervisor_port")
                    .and_then(Value::as_u64)
                    .is_none()
            {
                return Ok(());
            }
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
        "ethernet_switch" | "ethernet_hub" | "cloud" | "nat" => {
            state.builtin.stop_node(node_id).await;
            Ok(())
        }
        _ => Ok(()),
    }
}

fn docker_interface_mac(node_id: &str, interface_number: u64) -> Result<String, ApiError> {
    let node_id =
        Uuid::parse_str(node_id).map_err(|_| ApiError::bad_request("Docker node ID is invalid"))?;
    let bytes = node_id.as_bytes();
    Ok(format!(
        "02:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], interface_number as u8
    ))
}

async fn cleanup_link_runtime(
    state: &AppState,
    project_id: &str,
    link: &Value,
) -> Result<(), ApiError> {
    let runtime_endpoints = link_runtime_endpoints(state.store.pool(), project_id, link)
        .await
        .unwrap_or_default();
    for endpoint in link
        .get("nodes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let node_id = endpoint["node_id"].as_str().unwrap_or_default();
        if let Some(runtime_endpoint) = runtime_endpoints
            .iter()
            .find(|candidate| candidate["node_id"].as_str() == Some(node_id))
        {
            detach_runtime_link_endpoint(state, project_id, endpoint, runtime_endpoint).await?;
        } else {
            let _ = LinkEngine::new(state.ubridge.clone())
                .detach_endpoint(endpoint)
                .await;
            if let Some(endpoint_id) = endpoint["runtime_endpoint_id"].as_str()
                && let Ok(host) = ubridge::host_veth_name(endpoint_id)
            {
                let _ = state.ubridge.delete_veth(&host).await;
            }
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
    if let Some(idle_pc) = patch.get("idlepc").and_then(Value::as_str) {
        let node = get_topology_object(
            state.store.pool(),
            &project_id.to_string(),
            "node",
            &node_id.to_string(),
        )
        .await?
        .0;
        if node["node_type"].as_str() == Some("dynamips") && !idle_pc.is_empty() {
            let vm = state
                .dynamips
                .vm_from_node(&node)
                .map_err(runtime_api_error)?;
            state
                .dynamips
                .set_idle_pc(&vm, idle_pc)
                .await
                .map_err(runtime_api_error)?;
        }
    }
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

async fn update_node_configuration(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
    Json(patch): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let project_id = project_id.to_string();
    let node_id = node_id.to_string();
    let _project_lock = state.lock_project_operations(&project_id).await;
    let node = get_topology_object(state.store.pool(), &project_id, "node", &node_id)
        .await?
        .0;
    let node_type = node["node_type"].as_str().unwrap_or_default().to_owned();
    if BuiltinRuntime::is_type(&node_type) {
        let patch = BuiltinRuntime::normalize_configuration_patch(&node, patch)
            .map_err(ApiError::bad_request)?;
        let mut updated = node.clone();
        for (key, value) in patch.as_object().expect("normalized patch is an object") {
            updated[key] = value.clone();
        }
        updated = BuiltinRuntime::normalize_node(updated).map_err(ApiError::bad_request)?;
        BuiltinRuntime::validate_host_interfaces(&updated).map_err(ApiError::bad_request)?;
        validate_builtin_configuration_links(
            state.store.pool(),
            &project_id,
            &node_id,
            &node,
            &updated,
        )
        .await?;

        let rewire_cloud =
            node_type == "cloud" && node["ports_mapping"] != updated["ports_mapping"];
        if rewire_cloud {
            detach_node_links(&state, &project_id, &node_id).await?;
        }
        let persisted = persist_node_configuration(
            state.clone(),
            &project_id,
            &node_id,
            patch.clone(),
            Map::new(),
        )
        .await?;
        let updated = persisted.0;
        if node_type == "ethernet_switch" {
            state.builtin.update_switch_configuration(&updated).await;
        }
        if rewire_cloud
            && let Err(error) = attach_node_links(&state, &project_id, &node_id, false).await
        {
            let _ = detach_node_links(&state, &project_id, &node_id).await;
            let mut restore = Map::new();
            for key in patch
                .as_object()
                .expect("normalized patch is an object")
                .keys()
            {
                if let Some(value) = node.get(key) {
                    restore.insert(key.clone(), value.clone());
                }
            }
            let _ = persist_node_configuration(
                state.clone(),
                &project_id,
                &node_id,
                Value::Object(restore),
                Map::new(),
            )
            .await;
            let _ = attach_node_links(&state, &project_id, &node_id, false).await;
            return Err(error);
        }
        return Ok(Json(updated));
    }
    let patch = validate_node_configuration_patch(&node_type, patch)?;
    let current_status = match node_type.as_str() {
        "qemu" => {
            let vm = state.qemu.vm_from_node(&node).map_err(runtime_api_error)?;
            state.qemu.status(&vm).await.map_err(runtime_api_error)?
        }
        "docker" => {
            let container_id = node["container_id"]
                .as_str()
                .ok_or_else(|| ApiError::conflict("Docker node has no runtime container"))?;
            state
                .runtime
                .container_status(container_id)
                .await
                .map_err(runtime_api_error)?
        }
        "vpcs" => {
            let node_id = node["node_id"]
                .as_str()
                .ok_or_else(|| ApiError::conflict("VPCS node has no node ID"))?;
            state
                .vpcs
                .status(node_id)
                .await
                .map_err(runtime_api_error)?
        }
        "dynamips" => {
            let vm = state
                .dynamips
                .vm_from_node(&node)
                .map_err(runtime_api_error)?;
            state
                .dynamips
                .status(&vm)
                .await
                .map_err(runtime_api_error)?
        }
        _ => {
            return Err(ApiError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                "This runtime does not expose editable per-device settings",
            ));
        }
    };
    if current_status != "stopped" {
        return Err(ApiError::conflict(
            "Stop this device before changing runtime settings",
        ));
    }
    if let Some(adapters) = patch["adapters"].as_u64() {
        validate_adapter_count_against_links(
            state.store.pool(),
            &project_id,
            &node_id,
            &node_type,
            adapters,
        )
        .await?;
    }

    let mut updated = node;
    for (key, value) in patch.as_object().expect("validated patch is an object") {
        updated[key] = value.clone();
    }
    let mut extra_fields = Map::new();
    if node_type == "docker" {
        validate_docker_extra_configs_api(
            updated.get("extra_configs"),
            updated
                .get("startup_config_content")
                .and_then(Value::as_str),
        )?;
        detach_node_links(&state, &project_id, &node_id).await?;
        let options = DockerTemplateOptions {
            environment: updated["environment"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            extra_hosts: updated["extra_hosts"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            extra_volumes: updated["extra_volumes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            extra_configs: validate_docker_extra_configs_api(
                updated.get("extra_configs"),
                updated
                    .get("startup_config_content")
                    .and_then(Value::as_str),
            )?,
            startup_config_content: updated
                .get("startup_config_content")
                .and_then(Value::as_str)
                .map(str::to_owned),
            console_type: updated["console_type"]
                .as_str()
                .unwrap_or("telnet")
                .to_owned(),
            console_resolution: updated["console_resolution"]
                .as_str()
                .unwrap_or("1024x768")
                .to_owned(),
            console_http_port: updated["console_http_port"]
                .as_u64()
                .and_then(|port| u16::try_from(port).ok()),
            console_http_path: updated["console_http_path"].as_str().map(str::to_owned),
            aux_type: updated["aux_type"].as_str().unwrap_or("none").to_owned(),
            custom_adapters: serde_json::from_value(updated["custom_adapters"].clone()).map_err(
                |_| {
                    ApiError::bad_request("custom_adapters must be an array of adapter definitions")
                },
            )?,
            ..DockerTemplateOptions::default()
        };
        let old_container_id = updated["container_id"]
            .as_str()
            .ok_or_else(|| ApiError::conflict("Docker node has no runtime container"))?;
        let (container_id, image_digest) = state
            .runtime
            .reconfigure_container(
                &project_id,
                &node_id,
                old_container_id,
                updated["image"].as_str().unwrap_or_default(),
                updated["name"].as_str().unwrap_or("Docker node"),
                updated["start_command"].as_str(),
                updated["memory"].as_u64(),
                updated["cpus"].as_f64(),
                &options,
            )
            .await
            .map_err(runtime_api_error)?;
        extra_fields.insert("container_id".to_owned(), json!(container_id));
        extra_fields.insert("image_digest".to_owned(), json!(image_digest));
        extra_fields.insert(
            "console".to_owned(),
            json!(state.runtime.console_port(&container_id).await),
        );
    }
    persist_node_configuration(state, &project_id, &node_id, patch, extra_fields).await
}

fn validate_node_configuration_patch(node_type: &str, patch: Value) -> Result<Value, ApiError> {
    let patch = patch
        .as_object()
        .ok_or_else(|| ApiError::bad_request("device configuration must be an object"))?;
    if patch.is_empty() {
        return Err(ApiError::bad_request("device configuration is empty"));
    }
    let allowed: &[&str] = match node_type {
        "qemu" => &[
            "name",
            "memory",
            "cpus",
            "adapters",
            "adapter_type",
            "hda_disk_interface",
            "boot_priority",
        ],
        "docker" => &[
            "name",
            "memory",
            "cpus",
            "adapters",
            "start_command",
            "environment",
            "extra_hosts",
            "extra_volumes",
            "extra_configs",
            "startup_config_content",
            "console_type",
            "console_resolution",
            "console_http_port",
            "console_http_path",
            "aux_type",
            "custom_adapters",
        ],
        "vpcs" => &["name", "startup_script", "console_type"],
        "dynamips" => &["name", "startup_config"],
        _ => return Err(ApiError::bad_request("unsupported per-device runtime")),
    };
    let mut validated = Map::new();
    for (key, value) in patch {
        if !allowed.contains(&key.as_str()) {
            return Err(ApiError::bad_request(format!(
                "'{key}' cannot be changed in this device configuration"
            )));
        }
        let value = match key.as_str() {
            "name" => {
                let name = value
                    .as_str()
                    .map(str::trim)
                    .filter(|name| !name.is_empty() && name.len() <= 128)
                    .ok_or_else(|| ApiError::bad_request("device name must be 1–128 characters"))?;
                json!(name)
            }
            "memory" => {
                let memory = value
                    .as_u64()
                    .filter(|memory| {
                        if node_type == "qemu" {
                            (16..=1_048_576).contains(memory)
                        } else {
                            *memory <= 1_048_576
                        }
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request("memory is outside the supported range")
                    })?;
                json!(memory)
            }
            "cpus" => {
                if node_type == "qemu" {
                    let cpus = value
                        .as_u64()
                        .filter(|cpus| (1..=256).contains(cpus))
                        .ok_or_else(|| ApiError::bad_request("QEMU CPUs must be from 1 to 256"))?;
                    json!(cpus)
                } else {
                    let cpus = value
                        .as_f64()
                        .filter(|cpus| cpus.is_finite() && (0.0..=256.0).contains(cpus))
                        .ok_or_else(|| {
                            ApiError::bad_request("Docker CPU limit must be from 0 to 256")
                        })?;
                    json!(cpus)
                }
            }
            "adapters" => {
                let adapters = value
                    .as_u64()
                    .filter(|adapters| (1..=99).contains(adapters))
                    .ok_or_else(|| ApiError::bad_request("adapters must be from 1 to 99"))?;
                json!(adapters)
            }
            "adapter_type" => {
                let model = value
                    .as_str()
                    .filter(|model| {
                        matches!(*model, "e1000" | "virtio-net-pci" | "rtl8139" | "vmxnet3")
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request("unsupported QEMU network adapter model")
                    })?;
                json!(model)
            }
            "hda_disk_interface" => {
                let interface = value
                    .as_str()
                    .filter(|interface| matches!(*interface, "ide" | "scsi" | "virtio" | "sd"))
                    .ok_or_else(|| {
                        ApiError::bad_request("unsupported QEMU primary disk interface")
                    })?;
                json!(interface)
            }
            "boot_priority" => {
                let priority = value
                    .as_str()
                    .filter(|priority| matches!(*priority, "c" | "d" | "n" | "dc" | "cd"))
                    .ok_or_else(|| ApiError::bad_request("unsupported QEMU boot priority"))?;
                json!(priority)
            }
            "start_command" | "environment" | "extra_hosts" => {
                let text = value
                    .as_str()
                    .filter(|text| text.len() <= 65_536)
                    .ok_or_else(|| {
                        ApiError::bad_request(format!("{key} must be text under 64 KiB"))
                    })?;
                json!(text)
            }
            "startup_config_content" => {
                let text = value
                    .as_str()
                    .filter(|text| text.len() <= 1024 * 1024)
                    .ok_or_else(|| {
                        ApiError::bad_request("startup_config_content must be text under 1 MiB")
                    })?;
                json!(text)
            }
            "extra_configs" => {
                let configs = validate_docker_extra_configs_api(Some(value), None)?;
                serde_json::to_value(configs).map_err(internal_json_error)?
            }
            "startup_script" | "startup_config" => {
                let text = value
                    .as_str()
                    .filter(|text| text.len() <= 1_048_576)
                    .ok_or_else(|| {
                        ApiError::bad_request(format!("{key} must be text under 1 MiB"))
                    })?;
                json!(text)
            }
            "extra_volumes" => {
                let volumes = value.as_array().ok_or_else(|| {
                    ApiError::bad_request("extra_volumes must be an array of container paths")
                })?;
                if volumes.len() > 64 || volumes.iter().any(|volume| volume.as_str().is_none()) {
                    return Err(ApiError::bad_request(
                        "extra_volumes must contain at most 64 text paths",
                    ));
                }
                json!(volumes)
            }
            "console_type" => {
                let value = value
                    .as_str()
                    .filter(|value| {
                        if node_type == "vpcs" {
                            matches!(*value, "telnet" | "ssh" | "none")
                        } else {
                            matches!(*value, "none" | "telnet" | "vnc" | "http" | "https")
                        }
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request(format!("unsupported {node_type} console_type"))
                    })?;
                json!(value)
            }
            "console_resolution" => {
                let value = value
                    .as_str()
                    .filter(|value| {
                        value.split_once('x').is_some_and(|(width, height)| {
                            !width.is_empty()
                                && !height.is_empty()
                                && width.bytes().all(|byte| byte.is_ascii_digit())
                                && height.bytes().all(|byte| byte.is_ascii_digit())
                        })
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request("console_resolution must use WIDTHxHEIGHT")
                    })?;
                json!(value)
            }
            "console_http_port" => {
                let port = value
                    .as_u64()
                    .filter(|port| (1..=65535).contains(port))
                    .ok_or_else(|| {
                        ApiError::bad_request("console_http_port must be from 1 to 65535")
                    })?;
                json!(port)
            }
            "console_http_path" => {
                let path = value
                    .as_str()
                    .filter(|path| {
                        path.starts_with('/') && !path.bytes().any(|byte| byte.is_ascii_control())
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request("console_http_path must be an absolute URL path")
                    })?;
                json!(path)
            }
            "aux_type" => {
                let value = value
                    .as_str()
                    .filter(|value| matches!(*value, "none" | "telnet" | "ssh"))
                    .ok_or_else(|| ApiError::bad_request("unsupported Docker aux_type"))?;
                json!(value)
            }
            "custom_adapters" => {
                if !value.is_array() || value.as_array().is_some_and(|adapters| adapters.len() > 99)
                {
                    return Err(ApiError::bad_request(
                        "custom_adapters must be an array of at most 99 entries",
                    ));
                }
                serde_json::from_value::<Vec<crate::runtime::DockerCustomAdapter>>(value.clone())
                    .map_err(|_| {
                    ApiError::bad_request("custom_adapters contain an invalid adapter definition")
                })?;
                json!(value)
            }
            _ => unreachable!("keys were restricted above"),
        };
        validated.insert(key.clone(), value);
    }
    Ok(Value::Object(validated))
}

async fn validate_adapter_count_against_links(
    pool: &SqlitePool,
    project_id: &str,
    node_id: &str,
    node_type: &str,
    adapters: u64,
) -> Result<(), ApiError> {
    let links = list_objects(pool, project_id, "link").await?.0;
    for link in links {
        let Some(endpoints) = link["nodes"].as_array() else {
            continue;
        };
        for endpoint in endpoints {
            if endpoint["node_id"].as_str() != Some(node_id) {
                continue;
            }
            let configured_port = if node_type == "docker" {
                endpoint["port_number"].as_u64()
            } else {
                endpoint["adapter_number"].as_u64()
            }
            .unwrap_or_default();
            if configured_port >= adapters {
                return Err(ApiError::conflict(format!(
                    "Adapter count cannot be reduced to {adapters}; a link is using port {configured_port}"
                )));
            }
        }
    }
    Ok(())
}

async fn validate_builtin_configuration_links(
    pool: &SqlitePool,
    project_id: &str,
    node_id: &str,
    current: &Value,
    updated: &Value,
) -> Result<(), ApiError> {
    let node_type = updated["node_type"].as_str().unwrap_or_default();
    let current_count = current["ports_mapping"]
        .as_array()
        .map(Vec::len)
        .unwrap_or_default();
    let updated_count = updated["ports_mapping"]
        .as_array()
        .map(Vec::len)
        .unwrap_or_default();
    let links = list_objects(pool, project_id, "link").await?.0;
    for link in links {
        let Some(endpoints) = link["nodes"].as_array() else {
            continue;
        };
        for endpoint in endpoints {
            if endpoint["node_id"].as_str() != Some(node_id) {
                continue;
            }
            let port = endpoint["port_number"].as_u64().unwrap_or_default() as usize;
            if node_type == "ethernet_switch" && current_count != updated_count {
                return Err(ApiError::conflict(
                    "Cannot change the port count of an Ethernet switch that is already connected",
                ));
            }
            if port >= updated_count {
                return Err(ApiError::conflict(format!(
                    "Cannot remove port {port} while it is connected"
                )));
            }
        }
    }
    Ok(())
}

async fn persist_node_configuration(
    state: AppState,
    project_id: &str,
    node_id: &str,
    patch: Value,
    extra_fields: Map<String, Value>,
) -> Result<Json<Value>, ApiError> {
    let mut tx = state.store.pool().begin().await?;
    ensure_project_exists(&mut tx, project_id).await?;
    let row = sqlx::query(
        "SELECT object_json FROM topology_objects WHERE project_id = ? AND kind = 'node' AND object_id = ?",
    )
    .bind(project_id)
    .bind(node_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::not_found(format!("Node {node_id} does not exist")))?;
    let mut node = decode_json(row.get::<String, _>("object_json"))?;
    let object = as_object_mut(&mut node, "stored node is invalid")?;
    for (key, value) in patch.as_object().expect("validated patch is an object") {
        object.insert(key.clone(), value.clone());
    }
    for (key, value) in extra_fields {
        object.insert(key, value);
    }
    let status = if BuiltinRuntime::is_type(
        object
            .get("node_type")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    ) {
        "started"
    } else {
        "stopped"
    };
    object.insert("status".to_owned(), json!(status));
    let name = required_string(object, "name")?.to_owned();
    sqlx::query(
        "UPDATE topology_objects SET name = ?, object_json = ? WHERE project_id = ? AND kind = 'node' AND object_id = ?",
    )
    .bind(name)
    .bind(serde_json::to_string(&node).map_err(internal_json_error)?)
    .bind(project_id)
    .bind(node_id)
    .execute(&mut *tx)
    .await?;
    let event = Store::append_event(&mut tx, project_id, "node.updated", &node).await?;
    tx.commit().await?;
    state.store.publish(event);
    Ok(Json(node))
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
    let node = get_topology_object(
        state.store.pool(),
        &project_id.to_string(),
        "node",
        &node_id.to_string(),
    )
    .await?
    .0;
    if node.get("node_type").and_then(Value::as_str) == Some("vpcs") {
        return Err(ApiError::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "VPCS nodes do not support suspend",
        ));
    }
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
    let _project_lock = state.lock_project_operations(&project_id).await;
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
    let project_id = project_id.to_string();
    let link_id = link_id.to_string();
    let _project_lock = state.lock_project_operations(&project_id).await;
    let current = get_topology_object(state.store.pool(), &project_id, "link", &link_id)
        .await?
        .0;
    let patch = normalize_link_update(&current, patch)?;
    let mut candidate = current.clone();
    if let (Some(candidate), Some(patch)) = (candidate.as_object_mut(), patch.as_object()) {
        for (key, value) in patch {
            candidate.insert(key.clone(), value.clone());
        }
    }
    if patch.get("filters").is_some() || patch.get("suspend").is_some() {
        let runtime_endpoints =
            link_runtime_endpoints(state.store.pool(), &project_id, &candidate).await?;
        for endpoint in candidate["nodes"].as_array().into_iter().flatten() {
            let Some(runtime_endpoint) = runtime_endpoints
                .iter()
                .find(|candidate| candidate["node_id"].as_str() == endpoint["node_id"].as_str())
            else {
                continue;
            };
            if runtime_endpoint_status(&state, &project_id, runtime_endpoint).await? == "stopped" {
                continue;
            }
            attach_runtime_link_endpoint(
                &state,
                &project_id,
                &candidate,
                endpoint,
                runtime_endpoint,
            )
            .await?;
        }
    }
    update_topology_object(state, project_id, link_id, "link", "link_id", patch).await
}

fn normalize_link_update(current: &Value, patch: Value) -> Result<Value, ApiError> {
    let object = patch
        .as_object()
        .ok_or_else(|| ApiError::bad_request("link update must be an object"))?;
    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "filters" | "suspend" | "link_style" | "nodes" | "show_filters_icon" | "label"
        ) {
            return Err(ApiError::bad_request(format!(
                "Link field '{key}' cannot be updated"
            )));
        }
    }
    let mut normalized = patch;
    if let Some(filters) = normalized.get("filters") {
        normalized["filters"] = validate_link_filters(filters)?;
    }
    if normalized
        .get("suspend")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err(ApiError::bad_request("suspend must be a boolean"));
    }
    if normalized
        .get("link_style")
        .is_some_and(|value| !value.is_object())
    {
        return Err(ApiError::bad_request("link_style must be an object"));
    }
    if let Some(nodes) = normalized.get("nodes") {
        let requested = nodes
            .as_array()
            .filter(|nodes| nodes.len() == 2)
            .ok_or_else(|| ApiError::bad_request("GNS3 links have exactly two endpoints"))?;
        let mut merged = current["nodes"]
            .as_array()
            .cloned()
            .ok_or_else(|| ApiError::conflict("Stored link has invalid endpoints"))?;
        for requested_endpoint in requested {
            let node_id = requested_endpoint["node_id"]
                .as_str()
                .ok_or_else(|| ApiError::bad_request("Link endpoint requires node_id"))?;
            let existing = merged
                .iter_mut()
                .find(|endpoint| endpoint["node_id"].as_str() == Some(node_id))
                .ok_or_else(|| {
                    ApiError::conflict("Delete and recreate the link to change runtime endpoints")
                })?;
            for identity in ["adapter_number", "port_number"] {
                if requested_endpoint
                    .get(identity)
                    .is_some_and(|value| value != &existing[identity])
                {
                    return Err(ApiError::conflict(
                        "Delete and recreate the link to change runtime endpoints",
                    ));
                }
            }
            if let Some(label) = requested_endpoint.get("label") {
                existing["label"] = label.clone();
            }
        }
        normalized["nodes"] = json!(merged);
    }
    Ok(normalized)
}

fn validate_link_filters(filters: &Value) -> Result<Value, ApiError> {
    let filters = filters
        .as_object()
        .ok_or_else(|| ApiError::bad_request("filters must be an object"))?;
    let mut active = serde_json::Map::new();
    for (kind, values) in filters {
        let values = values
            .as_array()
            .ok_or_else(|| ApiError::bad_request(format!("filter '{kind}' must be an array")))?;
        match kind.as_str() {
            "frequency_drop" => {
                if values.len() != 1 {
                    return Err(ApiError::bad_request("frequency_drop expects one value"));
                }
                let frequency = filter_integer(kind, &values[0], -1, 32767)?;
                if frequency != 0 {
                    active.insert(kind.clone(), json!([frequency]));
                }
            }
            "packet_loss" => {
                if values.len() != 1 {
                    return Err(ApiError::bad_request("packet_loss expects one value"));
                }
                let chance = filter_integer(kind, &values[0], 0, 100)?;
                if chance != 0 {
                    active.insert(kind.clone(), json!([chance]));
                }
            }
            "delay" => {
                if !(1..=3).contains(&values.len()) {
                    return Err(ApiError::bad_request(
                        "delay expects latency, optional jitter, and optional distribution",
                    ));
                }
                let latency = filter_integer(kind, &values[0], 0, 32767)?;
                let jitter = if values.len() > 1 {
                    filter_integer(kind, &values[1], 0, 32767)?
                } else {
                    0
                };
                if latency == 0 && jitter == 0 {
                    continue;
                }
                if latency == 0 {
                    return Err(ApiError::bad_request(
                        "delay latency must be at least 1 ms when jitter is enabled",
                    ));
                }
                let mut normalized = vec![json!(latency), json!(jitter)];
                if let Some(distribution) = values.get(2) {
                    let distribution = distribution
                        .as_str()
                        .unwrap_or_default()
                        .trim()
                        .to_ascii_lowercase();
                    if !matches!(
                        distribution.as_str(),
                        "" | "uniform" | "normal" | "pareto" | "paretonormal"
                    ) {
                        return Err(ApiError::bad_request(
                            "delay distribution must be uniform, normal, pareto, or paretonormal",
                        ));
                    }
                    if distribution.is_empty() {
                        normalized.push(json!(""));
                    } else {
                        normalized.push(json!(distribution));
                    }
                }
                active.insert(kind.clone(), Value::Array(normalized));
            }
            "corrupt" => {
                if values.len() != 1 {
                    return Err(ApiError::bad_request("corrupt expects one value"));
                }
                let chance = filter_integer(kind, &values[0], 0, 100)?;
                if chance != 0 {
                    active.insert(kind.clone(), json!([chance]));
                }
            }
            "bpf" => {
                if values.len() != 1 {
                    return Err(ApiError::bad_request("bpf expects one expression"));
                }
                let expression = values[0]
                    .as_str()
                    .ok_or_else(|| ApiError::bad_request("bpf expression must be text"))?;
                if expression.len() > 4096 || expression.chars().any(char::is_control) {
                    return Err(ApiError::bad_request(
                        "bpf expression is too long or contains a control character",
                    ));
                }
                if !expression.trim().is_empty() {
                    active.insert(kind.clone(), json!([expression]));
                }
            }
            _ => {
                return Err(ApiError::bad_request(format!(
                    "Unsupported link filter '{kind}'"
                )));
            }
        }
    }
    Ok(Value::Object(active))
}

fn filter_integer(kind: &str, value: &Value, minimum: i64, maximum: i64) -> Result<i64, ApiError> {
    let integer = value
        .as_i64()
        .or_else(|| value.as_str().and_then(|value| value.trim().parse().ok()))
        .ok_or_else(|| ApiError::bad_request(format!("{kind} filter values must be integers")))?;
    if integer < minimum || integer > maximum {
        return Err(ApiError::bad_request(format!(
            "{kind} filter values must be between {minimum} and {maximum}"
        )));
    }
    Ok(integer)
}

async fn available_link_filters(
    State(state): State<AppState>,
    Path((project_id, link_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Vec<Value>>, ApiError> {
    let _ = get_topology_object(
        state.store.pool(),
        &project_id.to_string(),
        "link",
        &link_id.to_string(),
    )
    .await?;
    Ok(Json(vec![
        json!({ "type": "frequency_drop", "name": "Frequency drop", "description": "Drop every Nth packet; -1 drops all packets.", "parameters": [{"name":"Frequency","minimum":-1,"maximum":32767,"type":"int","unit":"th packet"}] }),
        json!({ "type": "packet_loss", "name": "Packet loss", "description": "Chance for a packet to be lost.", "parameters": [{"name":"Chance","minimum":0,"maximum":100,"type":"int","unit":"%"}] }),
        json!({ "type": "delay", "name": "Delay", "description": "Delay packets with optional jitter and distribution.", "parameters": [{"name":"Latency","minimum":1,"maximum":32767,"type":"int","unit":"ms"},{"name":"Jitter (-/+)","minimum":0,"maximum":32767,"type":"int","unit":"ms"},{"name":"Distribution","type":"str","unit":"uniform|normal|pareto|paretonormal"}] }),
        json!({ "type": "corrupt", "name": "Corrupt", "description": "Chance for a packet to be corrupted.", "parameters": [{"name":"Chance","minimum":0,"maximum":100,"type":"int","unit":"%"}] }),
        json!({ "type": "bpf", "name": "Berkeley Packet Filter (BPF)", "description": "Drop packets matching a BPF expression.", "parameters": [{"name":"Filters","type":"text"}] }),
    ]))
}

#[derive(Deserialize)]
struct LinkCaptureRequest {
    capture_file_name: Option<String>,
    data_link_type: Option<String>,
    #[serde(default)]
    wireshark: bool,
}

async fn start_link_capture(
    State(state): State<AppState>,
    Path((project_id, link_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<LinkCaptureRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if request.wireshark {
        return Err(ApiError::new(
            StatusCode::NOT_IMPLEMENTED,
            "Web Wireshark is not available",
        ));
    }
    if request.data_link_type.as_deref().unwrap_or("DLT_EN10MB") != "DLT_EN10MB" {
        return Err(ApiError::bad_request(
            "Ethernet link capture requires DLT_EN10MB",
        ));
    }
    let project_id = project_id.to_string();
    let link_id = link_id.to_string();
    let _project_lock = state.lock_project_operations(&project_id).await;
    let link = get_topology_object(state.store.pool(), &project_id, "link", &link_id)
        .await?
        .0;
    if link["capturing"].as_bool().unwrap_or(false) {
        return Err(ApiError::conflict("Link capture is already active"));
    }
    let file_name = safe_capture_file_name(
        request
            .capture_file_name
            .as_deref()
            .unwrap_or(&format!("{link_id}.pcap")),
    )?;
    let directory = capture_directory(&project_id);
    std::fs::create_dir_all(&directory).map_err(|error| {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not create capture directory: {error}"),
        )
    })?;
    let path = canonical_capture_file_path(&project_id, &file_name, None, false)?;
    let path_text = path.to_string_lossy().into_owned();
    let runtimes = link_runtime_endpoints(state.store.pool(), &project_id, &link).await?;
    let mut selected = None;
    for endpoint in link["nodes"].as_array().into_iter().flatten() {
        let node_id = endpoint["node_id"].as_str().unwrap_or_default();
        let runtime = runtimes
            .iter()
            .find(|runtime| runtime["node_id"].as_str() == Some(node_id))
            .ok_or_else(|| ApiError::conflict("Link runtime endpoint is missing"))?;
        if runtime_endpoint_status(&state, &project_id, runtime).await? == "started" {
            selected = Some((endpoint.clone(), runtime.clone()));
            break;
        }
    }
    let (endpoint, runtime_endpoint) = selected
        .ok_or_else(|| ApiError::conflict("Start a node on this link before capturing packets"))?;
    attach_runtime_link_endpoint(&state, &project_id, &link, &endpoint, &runtime_endpoint).await?;
    LinkEngine::new(state.ubridge.clone())
        .start_capture(&endpoint, &path_text)
        .await
        .map_err(runtime_api_error)?;
    let updated = update_topology_object(
        state,
        project_id,
        link_id,
        "link",
        "link_id",
        json!({
            "capturing": true,
            "capture_file_name": file_name,
            "capture_file_path": path_text,
            "capture_endpoint_id": endpoint["runtime_endpoint_id"],
            "wireshark": false
        }),
    )
    .await?;
    Ok((StatusCode::CREATED, updated))
}

async fn stop_link_capture(
    State(state): State<AppState>,
    Path((project_id, link_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let project_id = project_id.to_string();
    let link_id = link_id.to_string();
    let _project_lock = state.lock_project_operations(&project_id).await;
    let link = get_topology_object(state.store.pool(), &project_id, "link", &link_id)
        .await?
        .0;
    if let Some(endpoint_id) = link["capture_endpoint_id"].as_str()
        && let Some(endpoint) = link["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|endpoint| endpoint["runtime_endpoint_id"].as_str() == Some(endpoint_id))
    {
        LinkEngine::new(state.ubridge.clone())
            .stop_capture(endpoint)
            .await
            .map_err(runtime_api_error)?;
    }
    let _ = update_topology_object(
        state,
        project_id,
        link_id,
        "link",
        "link_id",
        json!({ "capturing": false, "capture_endpoint_id": null, "wireshark": false }),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn stream_link_capture(
    State(state): State<AppState>,
    Path((project_id, link_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    link_capture_response(&state, &project_id.to_string(), &link_id.to_string(), true).await
}

async fn download_link_capture(
    State(state): State<AppState>,
    Path((project_id, link_id)): Path<(Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    link_capture_response(&state, &project_id.to_string(), &link_id.to_string(), false).await
}

async fn link_capture_response(
    state: &AppState,
    project_id: &str,
    link_id: &str,
    require_active: bool,
) -> Result<Response, ApiError> {
    let link = get_topology_object(state.store.pool(), project_id, "link", link_id)
        .await?
        .0;
    if require_active && !link["capturing"].as_bool().unwrap_or(false) {
        return Err(ApiError::conflict("This link has no active packet capture"));
    }
    let file_name = safe_capture_file_name(
        link["capture_file_name"]
            .as_str()
            .ok_or_else(|| ApiError::not_found("Link has no capture file name"))?,
    )?;
    let stored_path = link["capture_file_path"]
        .as_str()
        .ok_or_else(|| ApiError::not_found("Link has no capture file"))?;
    let path =
        canonical_capture_file_path(project_id, &file_name, Some(stored_path), !require_active)?;
    let body = capture_file_body(
        path,
        require_active.then(|| {
            (
                state.store.pool().clone(),
                project_id.to_owned(),
                link_id.to_owned(),
            )
        }),
    );
    let mut response = body.into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        "application/vnd.tcpdump.pcap"
            .parse()
            .expect("valid media type"),
    );
    if !require_active && let Ok(value) = format!("attachment; filename=\"{file_name}\"").parse() {
        response
            .headers_mut()
            .insert(axum::http::header::CONTENT_DISPOSITION, value);
    }
    Ok(response)
}

fn capture_file_body(path: PathBuf, follow: Option<(SqlitePool, String, String)>) -> Body {
    let stream = futures_util::stream::unfold(
        (None::<tokio::fs::File>, false),
        move |(mut file, finished)| {
            let path = path.clone();
            let follow = follow.clone();
            async move {
                if finished {
                    return None;
                }
                loop {
                    if file.is_none() {
                        match tokio::fs::File::open(&path).await {
                            Ok(opened) => file = Some(opened),
                            Err(error)
                                if error.kind() == std::io::ErrorKind::NotFound
                                    && follow.is_some() =>
                            {
                                match capture_is_active(
                                    follow.as_ref().expect("checked capture state"),
                                )
                                .await
                                {
                                    Ok(true) => sleep(Duration::from_millis(100)).await,
                                    Ok(false) => return None,
                                    Err(error) => return Some((Err(error), (None, true))),
                                }
                                continue;
                            }
                            Err(error) => return Some((Err(error), (None, true))),
                        }
                    }
                    let mut buffer = vec![0_u8; 16 * 1024];
                    match file
                        .as_mut()
                        .expect("capture file was opened")
                        .read(&mut buffer)
                        .await
                    {
                        Ok(0) => match follow.as_ref() {
                            Some(capture) => match capture_is_active(capture).await {
                                Ok(true) => sleep(Duration::from_millis(100)).await,
                                Ok(false) => return None,
                                Err(error) => return Some((Err(error), (file, true))),
                            },
                            None => return None,
                        },
                        Ok(read) => {
                            buffer.truncate(read);
                            return Some((Ok(Bytes::from(buffer)), (file, false)));
                        }
                        Err(error) => return Some((Err(error), (file, true))),
                    }
                }
            }
        },
    );
    Body::from_stream(stream)
}

async fn capture_is_active(
    (pool, project_id, link_id): &(SqlitePool, String, String),
) -> Result<bool, std::io::Error> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(json_extract(object_json, '$.capturing'), 0)
         FROM topology_objects WHERE project_id = ? AND kind = 'link' AND object_id = ?",
    )
    .bind(project_id)
    .bind(link_id)
    .fetch_optional(pool)
    .await
    .map(|capturing| capturing == Some(1))
    .map_err(std::io::Error::other)
}

fn capture_directory(project_id: &str) -> std::path::PathBuf {
    std::env::var_os("NETWORK_SIMULATOR_CAPTURE_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("data/captures"))
        .join(project_id)
}

fn canonical_capture_file_path(
    project_id: &str,
    file_name: &str,
    stored_path: Option<&str>,
    require_exists: bool,
) -> Result<PathBuf, ApiError> {
    canonical_capture_file_path_in(
        &capture_directory(project_id),
        file_name,
        stored_path,
        require_exists,
    )
}

fn canonical_capture_file_path_in(
    directory: &FsPath,
    file_name: &str,
    stored_path: Option<&str>,
    require_exists: bool,
) -> Result<PathBuf, ApiError> {
    let file_name = safe_capture_file_name(file_name)?;
    let directory = directory
        .canonicalize()
        .map_err(|_| ApiError::not_found("Project capture directory is not available"))?;
    let expected = directory.join(&file_name);
    let candidate = stored_path
        .map(PathBuf::from)
        .unwrap_or_else(|| expected.clone());
    let canonical = match candidate.canonicalize() {
        Ok(path) => path,
        Err(error)
            if !require_exists
                && error.kind() == std::io::ErrorKind::NotFound
                && std::fs::symlink_metadata(&candidate).is_err() =>
        {
            let parent = candidate
                .parent()
                .and_then(|parent| parent.canonicalize().ok())
                .ok_or_else(|| ApiError::not_found("Capture file path is not available"))?;
            parent.join(
                candidate
                    .file_name()
                    .ok_or_else(|| ApiError::not_found("Capture file path is invalid"))?,
            )
        }
        Err(_) => return Err(ApiError::not_found("Capture file is not available")),
    };
    if canonical != expected || !canonical.starts_with(&directory) {
        return Err(ApiError::not_found(
            "Capture file path is outside the project capture directory",
        ));
    }
    if require_exists && !canonical.is_file() {
        return Err(ApiError::not_found("Capture file is not available"));
    }
    Ok(canonical)
}

fn safe_capture_file_name(file_name: &str) -> Result<String, ApiError> {
    let path = FsPath::new(file_name);
    if file_name.is_empty()
        || file_name.len() > 255
        || path.file_name().and_then(|name| name.to_str()) != Some(file_name)
        || file_name == "."
        || file_name == ".."
        || file_name.chars().any(char::is_control)
    {
        return Err(ApiError::bad_request(
            "capture_file_name must be a simple file name",
        ));
    }
    Ok(file_name.to_owned())
}

async fn delete_link(
    State(state): State<AppState>,
    Path((project_id, link_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let project_id = project_id.to_string();
    let link_id = link_id.to_string();
    let _project_lock = state.lock_project_operations(&project_id).await;
    let link = get_topology_object(state.store.pool(), &project_id, "link", &link_id)
        .await?
        .0;
    cleanup_link_runtime(&state, &project_id, &link).await?;
    let response = delete_topology_object(state.clone(), project_id, link_id, "link").await?;
    LinkEngine::new(state.ubridge.clone())
        .release_link_ports(&link)
        .await;
    Ok(response)
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
    let _project_lock = state.lock_project_operations(&project_id).await;
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
    let activity = state.project_activity.clone();
    Ok(websocket.on_upgrade(move |socket| async move {
        let _activity = activity.open_websocket(project_id.clone());
        websocket_session(socket, state, project_id, query.after).await;
    }))
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
    let activity = state.project_activity.clone();
    Ok(websocket.on_upgrade(move |socket| async move {
        let _activity = activity.open_websocket(project_id);
        qemu_console_session(socket, serial).await;
    }))
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
    let console_type = node["console_type"].as_str().unwrap_or("telnet");
    if console_type != "telnet" {
        let detail = if matches!(console_type, "http" | "https" | "vnc") {
            "use the Docker console TCP port"
        } else {
            "the Docker WebSocket route supports telnet consoles"
        };
        return Err(ApiError::conflict(format!(
            "Docker {console_type} console is configured; {detail}"
        )));
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
    let runtime = state.runtime.clone();
    let container_id = container_id.to_owned();
    let activity = state.project_activity.clone();
    Ok(websocket.on_upgrade(move |socket| async move {
        let _activity = activity.open_websocket(project_id);
        docker_console_session(socket, process, runtime, container_id, true).await;
    }))
}

async fn docker_aux_console(
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
            "Auxiliary Docker consoles are available for Docker nodes only",
        ));
    }
    if node["aux_type"].as_str().unwrap_or("none") == "none" {
        return Err(ApiError::conflict(
            "Configure a Docker auxiliary console before opening it",
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
            "Start the Docker node before opening its auxiliary console",
        ));
    }
    let process = state
        .runtime
        .attach_aux_process(container_id)
        .map_err(runtime_api_error)?;
    let runtime = state.runtime.clone();
    let container_id = container_id.to_owned();
    let activity = state.project_activity.clone();
    Ok(websocket.on_upgrade(move |socket| async move {
        let _activity = activity.open_websocket(project_id);
        docker_console_session(socket, process, runtime, container_id, false).await;
    }))
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
    match node["console_type"].as_str().unwrap_or("telnet") {
        "none" => {
            return Err(ApiError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                "VPCS console is disabled for this node",
            ));
        }
        "telnet" | "ssh" => {}
        _ => {
            return Err(ApiError::bad_request(
                "VPCS console_type must be telnet, ssh, or none",
            ));
        }
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
    let activity = state.project_activity.clone();
    Ok(websocket.on_upgrade(move |socket| async move {
        let _activity = activity.open_websocket(project_id);
        vpcs_console_session(socket, console).await;
    }))
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
    let activity = state.project_activity.clone();
    Ok(websocket.on_upgrade(move |socket| async move {
        let _activity = activity.open_websocket(project_id);
        vpcs_console_session(socket, console).await;
    }))
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

async fn docker_console_session(
    socket: WebSocket,
    mut process: tokio::process::Child,
    runtime: DockerRuntime,
    container_id: String,
    resize_container_tty: bool,
) {
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
                    if resize_container_tty
                        && let Some((rows, columns)) = parse_docker_resize_control(text.as_str())
                    {
                        if let Err(error) = runtime.resize_tty(&container_id, rows, columns).await {
                            tracing::warn!(container_id, error = %error.message(), "Could not resize Docker console TTY");
                        }
                        continue;
                    }
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

fn parse_docker_resize_control(text: &str) -> Option<(u16, u16)> {
    let message: Value = serde_json::from_str(text).ok()?;
    if message.get("type")?.as_str()? != "resize" {
        return None;
    }
    let rows = u16::try_from(message.get("rows")?.as_u64()?).ok()?;
    let columns = u16::try_from(message.get("columns")?.as_u64()?).ok()?;
    (rows > 0 && columns > 0).then_some((rows, columns))
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
#[path = "tests/project_activity_tests.rs"]
mod project_activity_tests;

#[cfg(test)]
mod tests {
    use axum::{
        body::Body,
        extract::{Path, State},
        http::{HeaderMap, Method, Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use serde_json::Value;
    use tempfile::TempDir;
    use tokio::time::timeout;
    use tower::ServiceExt;

    use super::{
        ApiError, AppState, attach_runtime_nio_with_rollback, auth, canonical_capture_file_path_in,
        dynamips_options_from_template, link_filters_for_endpoint, parse_docker_resize_control,
        router, safe_capture_file_name, suspend_node, validate_link_filters,
        validate_node_configuration_patch, vpcs_startup_script,
    };
    use crate::storage::Store;
    use crate::{
        dynamips::DynamipsRuntime, qemu::QemuRuntime, runtime::DockerRuntime, vpcs::VpcsRuntime,
    };

    async fn test_router() -> (axum::Router, TempDir) {
        test_router_with_vpcs(false).await
    }

    async fn test_router_with_vpcs(vpcs_available: bool) -> (axum::Router, TempDir) {
        let directory = TempDir::new().expect("temporary test directory");
        let database = directory.path().join("controller.sqlite");
        let store = Store::connect(database.to_str().expect("UTF-8 temporary path"))
            .await
            .expect("test database connects");
        let docker = directory.path().join("docker-test-double");
        let state_file = directory.path().join("container-state");
        let docker_arguments = directory.path().join("docker-arguments.log");
        let script = format!(
            "#!/bin/sh\nstate_file='{}'\nprintf '%s\\n' \"$*\" >> '{}'\ncase \"$1\" in\n  info) echo 29.6.2 ;;\n  image) if [ \"$2\" = ls ]; then printf '{{\"Repository\":\"alpine\",\"Tag\":\"latest\",\"ID\":\"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"}}\\n'; else echo sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef; fi ;;\n  create) echo 0123456789abcdef; echo created > \"$state_file\" ;;\n  inspect) cat \"$state_file\" ;;\n  start|unpause|restart) echo running > \"$state_file\" ;;\n  stop) echo exited > \"$state_file\" ;;\n  pause) echo paused > \"$state_file\" ;;\n  network) if [ \"$2\" = create ]; then echo test-network-id; fi; exit 0 ;;\n  ps) exit 0 ;;\n  *) exit 0 ;;\nesac\n",
            state_file.display(),
            docker_arguments.display()
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
        let qemu = directory.path().join("qemu-system-x86_64");
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
        let vpcs_binary = if vpcs_available {
            let binary = directory.path().join("vpcs-test-double");
            std::fs::write(
                &binary,
                "#!/bin/sh\nif [ \"$1\" = -v ]; then echo 'VPCS version 0.8.5'; exit 0; fi\nexit 1\n",
            )
            .expect("write VPCS test double");
            std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755))
                .expect("make VPCS test double executable");
            binary
        } else {
            std::env::var_os("NETWORK_SIMULATOR_TEST_VPCS")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| directory.path().join("vpcs-test-double-not-installed"))
        };
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
                VpcsRuntime::with_paths(vpcs_binary, directory.path().join("vpcs-state")),
                DynamipsRuntime::with_paths(
                    directory.path().join("dynamips-test-double-not-installed"),
                    directory.path().join("dynamips-images"),
                    directory.path().join("dynamips-state"),
                ),
                crate::marketplace::MarketplaceCatalog::with_directory(
                    directory.path().join("appliances"),
                ),
            )),
            directory,
        )
    }

    #[tokio::test]
    async fn project_operation_locks_serialize_one_project_without_blocking_another() {
        let directory = TempDir::new().expect("temporary lock-test directory");
        let store = Store::connect(
            directory
                .path()
                .join("lock-test.sqlite")
                .to_str()
                .expect("UTF-8 database path"),
        )
        .await
        .expect("test database connects");
        let state = AppState::new(store);
        let first = state.lock_project_operations("project-a").await;
        let _other_project = state.lock_project_operations("project-b").await;
        assert!(
            timeout(
                std::time::Duration::from_millis(20),
                state.lock_project_operations("project-a")
            )
            .await
            .is_err()
        );
        drop(first);
        let _released = timeout(
            std::time::Duration::from_secs(1),
            state.lock_project_operations("project-a"),
        )
        .await
        .expect("same-project lock becomes available after release");
    }

    #[tokio::test]
    async fn vpcs_base_script_file_is_loaded_from_project_files_and_inline_script_wins() {
        let directory = TempDir::new().expect("temporary test directory");
        let store = Store::connect(
            directory
                .path()
                .join("vpcs-config.sqlite")
                .to_str()
                .expect("UTF-8 database path"),
        )
        .await
        .expect("test database connects");
        let project_id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO projects(project_id, name, status, project_json) VALUES(?, ?, ?, ?)",
        )
        .bind(&project_id)
        .bind("VPCS script lab")
        .bind("opened")
        .bind("{}")
        .execute(store.pool())
        .await
        .unwrap();
        sqlx::query("INSERT INTO project_files(project_id, file_path, contents) VALUES(?, ?, ?)")
            .bind(&project_id)
            .bind("configs/base.vpc")
            .bind(b"ip 192.0.2.1/24\n".as_slice())
            .execute(store.pool())
            .await
            .unwrap();
        let node = serde_json::json!({
            "startup_script": "",
            "base_script_file": "configs/base.vpc"
        });
        assert!(matches!(
            vpcs_startup_script(store.pool(), &project_id, &node).await,
            Ok(Some(script)) if script == "ip 192.0.2.1/24\n"
        ));
        let inline = serde_json::json!({
            "startup_script": "ip 198.51.100.2/24",
            "base_script_file": "configs/base.vpc"
        });
        assert!(matches!(
            vpcs_startup_script(store.pool(), &project_id, &inline).await,
            Ok(Some(script)) if script == "ip 198.51.100.2/24"
        ));
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

    async fn request_bytes(
        app: &axum::Router,
        method: Method,
        uri: String,
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("router returns a response");
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("response body reads")
            .to_bytes()
            .to_vec();
        (status, headers, bytes)
    }

    async fn create_custom_from_runtime(app: &axum::Router, runtime: &str, name: &str) -> Value {
        let (status, manager) = request(
            app,
            Method::GET,
            "/v3/marketplace/gns3/manager".into(),
            None,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "appliance manager response: {manager}"
        );
        let source_template_id =
            manager["custom_image_options"]
                .as_array()
                .unwrap()
                .iter()
                .find(|template| template["template_type"] == runtime)
                .unwrap_or_else(|| panic!("test runtime {runtime} has no installed image option"))
                ["template_id"]
                .as_str()
                .expect("runtime image has a template ID");
        let (status, appliance) = request(
            app,
            Method::POST,
            "/v3/marketplace/gns3/custom".into(),
            Some(serde_json::json!({
                "name": name,
                "source_template_id": source_template_id,
                "runtime_settings": {}
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        appliance
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
        // Unclassified /v3 paths are denied before routing (deny-by-default auth
        // classification), so an unknown API path yields a JSON 403, never the SPA shell.
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert!(
            response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.starts_with("application/json"))
        );
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
            { "node_id": node_a["node_id"], "adapter_number": 0, "port_number": 0, "label": { "text": "A" } },
            { "node_id": node_b["node_id"], "adapter_number": 0, "port_number": 0, "label": { "text": "B" } }
        ]);
        let (status, error) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/links"),
            Some(serde_json::json!({ "nodes": [endpoints[0], endpoints[1], endpoints[0]] })),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(error["message"].as_str().unwrap().contains("exactly two"));

        let (status, link) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/links"),
            Some(serde_json::json!({
                "nodes": endpoints,
                "suspend": true,
                "filters": { "delay": [10, 5, "normal"], "packet_loss": [10] },
                "link_style": { "width": 2 },
                "capturing": true,
                "capture_file_name": "client-controlled.pcap",
                "capture_file_path": "/etc/passwd"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(link["nodes"].as_array().map(Vec::len), Some(2));
        assert_eq!(
            link["nodes"][0]["runtime_peer_link_port"],
            link["nodes"][1]["runtime_link_port"]
        );
        assert_eq!(
            link["nodes"][1]["runtime_peer_link_port"],
            link["nodes"][0]["runtime_link_port"]
        );
        assert_eq!(link["nodes"][0]["label"]["text"], "A");
        assert_eq!(link["nodes"][1]["label"]["text"], "B");
        assert_eq!(link["suspend"], true);
        assert_eq!(link["capturing"], false);
        assert!(link["capture_file_name"].is_null());
        assert!(link["capture_file_path"].is_null());
        assert_eq!(
            link["filters"]["delay"],
            serde_json::json!([10, 5, "normal"])
        );
        assert_eq!(link["link_style"]["width"], 2);
        let (status, _) = request(
            &app,
            Method::PUT,
            format!(
                "/v3/projects/{project_id}/links/{}",
                link["link_id"].as_str().unwrap()
            ),
            Some(serde_json::json!({ "capture_file_path": "/etc/passwd" })),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

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
        let (app, directory) = test_router().await;
        create_custom_from_runtime(&app, "docker", "Alpine custom").await;
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

        let node_id = node["node_id"].as_str().unwrap();
        let (status, configured) = request(
            &app,
            Method::PUT,
            format!("/v3/projects/{project_id}/nodes/{node_id}/config"),
            Some(serde_json::json!({
                "memory": 256,
                "cpus": 1.5,
                "adapters": 2,
                "environment": "LAB_ROLE=router",
                "extra_hosts": "gateway:192.0.2.1",
                "extra_volumes": ["/etc/lab"]
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(configured["memory"], 256);
        assert_eq!(configured["cpus"], 1.5);
        assert_eq!(configured["adapters"], 2);
        assert_eq!(configured["environment"], "LAB_ROLE=router");
        assert_eq!(configured["extra_hosts"], "gateway:192.0.2.1");
        assert_eq!(configured["extra_volumes"], serde_json::json!(["/etc/lab"]));
        let docker_arguments =
            std::fs::read_to_string(directory.path().join("docker-arguments.log"))
                .expect("Docker create arguments were recorded");
        let creates = docker_arguments
            .lines()
            .filter(|line| line.starts_with("create "))
            .collect::<Vec<_>>();
        assert!(
            creates.len() >= 2,
            "the edit replaces the stopped container"
        );
        assert!(creates.iter().all(|line| line.contains("--network none")));
        assert!(creates.iter().all(|line| line.contains("--privileged")));
        assert!(creates.iter().any(|line| line.contains("--memory 256m")));
        assert!(creates.iter().any(|line| line.contains("--cpus 1.5")));
        assert!(
            creates
                .iter()
                .any(|line| line.contains("--env LAB_ROLE=router"))
        );

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
        let (status, error) = request(
            &app,
            Method::PUT,
            format!("/v3/projects/{project_id}/nodes/{node_id}/config"),
            Some(serde_json::json!({ "memory": 512 })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("Stop this device")
        );

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
    async fn saved_runtime_templates_create_nodes_with_their_settings() {
        let (app, directory) = test_router_with_vpcs(true).await;
        let (_, project) = request(
            &app,
            Method::POST,
            "/v3/projects".into(),
            Some(serde_json::json!({ "name": "Saved template lab" })),
        )
        .await;
        let project_id = project["project_id"].as_str().unwrap();
        let qemu_image = directory.path().join("qemu-images/router.qcow2");

        let (status, vpcs_template) = request(
            &app,
            Method::POST,
            "/v3/templates".into(),
            Some(serde_json::json!({
                "name": "Saved VPCS",
                "template_type": "vpcs",
                "adapters": 1,
                "startup_script": "ip 192.0.2.10/24",
                "base_script_file": "configs/base.vpc",
                "console_type": "telnet",
                "console_auto_start": true
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        let (status, docker_template) = request(
            &app,
            Method::POST,
            "/v3/templates".into(),
            Some(serde_json::json!({
                "name": "Saved Docker",
                "template_type": "docker",
                "image": "alpine:latest",
                "start_command": "sleep 321",
                "adapters": 3,
                "memory": 256,
                "cpus": 1.5,
                "environment": "ROLE=router",
                "extra_hosts": "gateway:192.0.2.1",
                "extra_volumes": ["/etc/lab"],
                "console_type": "telnet",
                "console_resolution": "1280x720",
                "console_http_port": 8080,
                "console_http_path": "/console",
                "aux_type": "none",
                "custom_adapters": []
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        let (status, qemu_template) = request(
            &app,
            Method::POST,
            "/v3/templates".into(),
            Some(serde_json::json!({
                "name": "Saved QEMU",
                "template_type": "qemu",
                "hda_disk_image": qemu_image,
                "adapters": 2,
                "memory": 96,
                "cpus": 2,
                "platform": "x86_64",
                "adapter_type": "e1000",
                "hda_disk_interface": "ide",
                "boot_priority": "c",
                "linked_clone": false
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        let (status, vpcs_node) = request(
            &app,
            Method::POST,
            format!(
                "/v3/projects/{project_id}/templates/{}",
                vpcs_template["template_id"].as_str().unwrap()
            ),
            Some(serde_json::json!({ "name": "Map VPCS", "x": 12, "y": 34 })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(vpcs_node["startup_script"], "ip 192.0.2.10/24");
        assert_eq!(vpcs_node["base_script_file"], "configs/base.vpc");
        assert_eq!(vpcs_node["console_auto_start"], true);
        assert_eq!(vpcs_node["x"], 12);

        let (status, docker_node) = request(
            &app,
            Method::POST,
            format!(
                "/v3/projects/{project_id}/templates/{}",
                docker_template["template_id"].as_str().unwrap()
            ),
            Some(serde_json::json!({ "name": "Map Docker" })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(docker_node["image"], "alpine:latest");
        assert_eq!(docker_node["start_command"], "sleep 321");
        assert_eq!(docker_node["adapters"], 3);
        assert_eq!(docker_node["memory"], 256);
        assert_eq!(docker_node["cpus"], 1.5);
        assert_eq!(docker_node["environment"], "ROLE=router");
        assert_eq!(docker_node["extra_hosts"], "gateway:192.0.2.1");
        assert_eq!(
            docker_node["extra_volumes"],
            serde_json::json!(["/etc/lab"])
        );
        assert_eq!(docker_node["console_resolution"], "1280x720");

        let (status, qemu_node) = request(
            &app,
            Method::POST,
            format!(
                "/v3/projects/{project_id}/templates/{}",
                qemu_template["template_id"].as_str().unwrap()
            ),
            Some(serde_json::json!({ "name": "Map QEMU" })),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "QEMU template add: {qemu_node}"
        );
        assert_eq!(
            qemu_node["hda_disk_image"].as_str(),
            Some(qemu_image.to_str().unwrap())
        );
        assert_eq!(qemu_node["adapters"], 2);
        assert_eq!(qemu_node["memory"], 96);
        assert_eq!(qemu_node["cpus"], 2);
        assert_eq!(qemu_node["maxcpus"], 2);
        assert_eq!(qemu_node["linked_clone"], false);
    }

    #[tokio::test]
    async fn project_lifecycle_batches_skip_inapplicable_nodes_and_continue_after_failures() {
        let (app, directory) = test_router().await;
        let (_, project) = request(
            &app,
            Method::POST,
            "/v3/projects".into(),
            Some(serde_json::json!({ "name": "Batch lifecycle lab" })),
        )
        .await;
        let project_id = project["project_id"].as_str().unwrap();
        let (_, templates) = request(&app, Method::GET, "/v3/templates".into(), None).await;
        for (node_type, name) in [
            ("ethernet_switch", "A switch"),
            ("ethernet_hub", "B hub"),
            ("cloud", "C cloud"),
            ("nat", "D NAT"),
        ] {
            let template = templates
                .as_array()
                .unwrap()
                .iter()
                .find(|template| template["template_type"] == node_type)
                .unwrap();
            let (status, _) = request(
                &app,
                Method::POST,
                format!(
                    "/v3/projects/{project_id}/templates/{}",
                    template["template_id"].as_str().unwrap()
                ),
                Some(serde_json::json!({ "name": name, "x": 0, "y": 0 })),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "create {node_type}");
        }

        let (status, _docker_node) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes"),
            Some(serde_json::json!({
                "name": "Z Docker",
                "node_type": "docker",
                "image": "alpine:latest",
                "adapters": 1
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        for (action, expected_runtime_state) in [
            ("start", "running"),
            ("suspend", "paused"),
            ("start", "running"),
            ("reload", "running"),
            ("stop", "exited"),
        ] {
            let (status, _) = request(
                &app,
                Method::POST,
                format!("/v3/projects/{project_id}/nodes/{action}"),
                Some(serde_json::json!({})),
            )
            .await;
            assert_eq!(status, StatusCode::NO_CONTENT, "project {action} all");
            let runtime_state = std::fs::read_to_string(directory.path().join("container-state"))
                .expect("Docker test runtime state is written");
            assert_eq!(
                runtime_state.trim(),
                expected_runtime_state,
                "Docker after {action} all"
            );
        }

        let (status, missing_qemu) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes"),
            Some(serde_json::json!({
                "name": "Y missing QEMU",
                "node_type": "qemu",
                "hda_disk_image": directory.path().join("qemu-images/router.qcow2"),
                "adapters": 0,
                "memory": 64,
                "cpus": 1
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        std::fs::remove_file(missing_qemu["runtime_overlay"].as_str().unwrap()).unwrap();

        let (status, error) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes/start"),
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(error["message"].as_str().unwrap().contains("overlay"));
        let runtime_state = std::fs::read_to_string(directory.path().join("container-state"))
            .expect("Docker test runtime state is written after the batch failure");
        assert_eq!(runtime_state.trim(), "running");
        assert!(!super::controller_routes::bulk_node_action_applies(
            &serde_json::json!({ "node_type": "vpcs", "status": "started" }),
            crate::runtime::NodeAction::Suspend
        ));
    }

    #[tokio::test]
    #[ignore = "requires the upstream VPCS executable; set NETWORK_SIMULATOR_TEST_VPCS and run with --ignored"]
    async fn vpcs_template_creates_nodes_and_point_to_point_links_through_the_controller() {
        let (app, _directory) = test_router().await;
        create_custom_from_runtime(&app, "vpcs", "VPCS").await;
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
            crate::marketplace::MarketplaceCatalog::with_directory(
                directory.path().join("appliances"),
            ),
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

            // Create after both runtimes are already running to exercise live
            // endpoint attachment through the controller route.
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
                || link["nodes"][0]["runtime_link_port"].as_u64().is_none()
                || link["nodes"][1]["runtime_link_port"].as_u64().is_none()
                || link["nodes"][0]["runtime_ubridge_segment"].is_string()
            {
                return Err(format!(
                    "live mixed UDP NIO link create returned {status}: {link}"
                ));
            }

            let container_id = docker_node["container_id"]
                .as_str()
                .ok_or_else(|| "Docker node has no container ID".to_owned())?;
            let docker_binary = env::var_os("NETWORK_SIMULATOR_DOCKER")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("docker"));
            for args in [
                vec!["ip", "addr", "add", "10.0.0.2/24", "dev", "eth0"],
                vec!["ip", "link", "set", "eth0", "up"],
            ] {
                let output = Command::new(&docker_binary)
                    .args(["exec", container_id])
                    .args(args)
                    .output()
                    .await
                    .map_err(|error| format!("could not configure Alpine eth0: {error}"))?;
                if !output.status.success() {
                    return Err(format!(
                        "could not configure Alpine eth0: {}",
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
    #[ignore = "requires Docker, upstream VPCS/uBridge, and uBridge CAP_NET_ADMIN/CAP_NET_RAW"]
    async fn builtin_switch_live_connects_three_hosts_and_isolates_access_vlans() {
        use std::{env, path::Path, path::PathBuf};

        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            process::Command,
            time::{Duration, timeout},
        };

        async fn require_status(
            app: &axum::Router,
            method: Method,
            uri: String,
            body: Option<Value>,
            expected: StatusCode,
        ) -> Result<Value, String> {
            let (status, response) = request(app, method, uri.clone(), body).await;
            if status != expected {
                return Err(format!("{uri} returned {status}: {response}"));
            }
            Ok(response)
        }

        async fn docker_command(
            binary: &Path,
            container: &str,
            arguments: &[&str],
        ) -> Result<std::process::Output, String> {
            Command::new(binary)
                .args(["exec", container])
                .args(arguments)
                .output()
                .await
                .map_err(|error| format!("Docker command failed to run: {error}"))
        }

        async fn vpcs_ping(
            console: &mut tokio::net::TcpStream,
            address: &str,
        ) -> Result<bool, String> {
            for _ in 0..2 {
                console
                    .write_all(format!("ping {address} -c 1\r").as_bytes())
                    .await
                    .map_err(|error| error.to_string())?;
                let mut output = Vec::new();
                timeout(Duration::from_secs(8), async {
                    loop {
                        let mut chunk = [0_u8; 2048];
                        let length = console.read(&mut chunk).await?;
                        output.extend_from_slice(&chunk[..length]);
                        let text = String::from_utf8_lossy(&output);
                        if text.contains("bytes from") || text.contains("VPCS>") {
                            break Ok::<(), std::io::Error>(());
                        }
                    }
                })
                .await
                .map_err(|_| format!("VPCS ping to {address} timed out"))?
                .map_err(|error| error.to_string())?;
                if String::from_utf8_lossy(&output).contains("bytes from") {
                    return Ok(true);
                }
            }
            Ok(false)
        }

        let directory = TempDir::new().expect("temporary builtin live test state");
        let store = Store::connect(
            directory
                .path()
                .join("controller.sqlite")
                .to_str()
                .expect("UTF-8 test database path"),
        )
        .await
        .expect("test database connects");
        let vpcs_binary = env::var_os("NETWORK_SIMULATOR_TEST_VPCS")
            .expect("NETWORK_SIMULATOR_TEST_VPCS names upstream VPCS");
        let ubridge_binary = env::var_os("NETWORK_SIMULATOR_TEST_UBRIDGE")
            .expect("NETWORK_SIMULATOR_TEST_UBRIDGE names capable upstream uBridge");
        let vpcs = VpcsRuntime::with_paths(vpcs_binary, directory.path().join("vpcs-state"));
        let ubridge = crate::ubridge::UbridgeRuntime::with_paths(
            PathBuf::from(ubridge_binary),
            directory.path().join("ubridge.sock"),
        );
        let app = router(AppState::with_runtimes_and_ubridge(
            store,
            DockerRuntime::new(),
            QemuRuntime::new(),
            vpcs.clone(),
            DynamipsRuntime::new(),
            ubridge.clone(),
            crate::marketplace::MarketplaceCatalog::with_directory(
                directory.path().join("appliances"),
            ),
        ));
        let docker_binary = env::var_os("NETWORK_SIMULATOR_DOCKER")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("docker"));
        let mut project_id: Option<String> = None;
        let mut switch_node_id: Option<String> = None;
        let mut vpcs_node_id: Option<String> = None;
        let mut link_endpoints = Vec::<Value>::new();
        let mut docker_hosts = Vec::<(String, String, String, String)>::new();

        let run: Result<(), String> = async {
            let project = require_status(
                &app,
                Method::POST,
                "/v3/projects".into(),
                Some(serde_json::json!({ "name": "Builtin switch live test" })),
                StatusCode::CREATED,
            )
            .await?;
            let current_project_id = project["project_id"]
                .as_str()
                .ok_or_else(|| "project has no ID".to_owned())?
                .to_owned();
            project_id = Some(current_project_id.clone());

            let switch = require_status(
                &app,
                Method::POST,
                format!("/v3/projects/{current_project_id}/nodes"),
                Some(serde_json::json!({
                    "name":"SW1",
                    "node_type":"ethernet_switch",
                    "ports_mapping":[
                        {"type":"access","vlan":10},
                        {"type":"access","vlan":10},
                        {"type":"access","vlan":10},
                        {"type":"access","vlan":20}
                    ]
                })),
                StatusCode::CREATED,
            )
            .await?;
            let switch_id = switch["node_id"]
                .as_str()
                .ok_or_else(|| "switch has no ID".to_owned())?
                .to_owned();
            switch_node_id = Some(switch_id.clone());

            let mut hosts = Vec::<(String, String, String)>::new();
            for (name, address) in [
                ("Docker A", "10.0.10.1/24"),
                ("Docker B", "10.0.10.3/24"),
                ("Docker VLAN20", "10.0.10.4/24"),
            ] {
                let node = require_status(
                    &app,
                    Method::POST,
                    format!("/v3/projects/{current_project_id}/nodes"),
                    Some(serde_json::json!({
                        "name":name,
                        "node_type":"docker",
                        "image":"alpine:latest",
                        "start_command":"/bin/sh -c 'sleep 3600'",
                        "adapters":1
                    })),
                    StatusCode::CREATED,
                )
                .await?;
                let node_id = node["node_id"]
                    .as_str()
                    .ok_or_else(|| format!("{name} has no ID"))?
                    .to_owned();
                let container_id = node["container_id"]
                    .as_str()
                    .ok_or_else(|| format!("{name} has no container ID"))?
                    .to_owned();
                hosts.push((node_id.clone(), address.to_owned(), name.to_owned()));
                docker_hosts.push((container_id, node_id, address.to_owned(), name.to_owned()));
            }
            let vpcs_node = require_status(
                &app,
                Method::POST,
                format!("/v3/projects/{current_project_id}/nodes"),
                Some(serde_json::json!({
                    "name":"VPCS C",
                    "node_type":"vpcs",
                    "startup_script":"ip 10.0.10.2/24"
                })),
                StatusCode::CREATED,
            )
            .await?;
            let vpcs_id = vpcs_node["node_id"]
                .as_str()
                .ok_or_else(|| "VPCS node has no ID".to_owned())?
                .to_owned();
            vpcs_node_id = Some(vpcs_id.clone());
            hosts.insert(
                2,
                (
                    vpcs_id.clone(),
                    "10.0.10.2/24".to_owned(),
                    "VPCS C".to_owned(),
                ),
            );

            for (port_number, (node_id, _, name)) in hosts.iter().enumerate() {
                let link = require_status(
                    &app,
                    Method::POST,
                    format!("/v3/projects/{current_project_id}/links"),
                    Some(serde_json::json!({ "nodes": [
                        {"node_id":switch_id,"adapter_number":0,"port_number":port_number},
                        {"node_id":node_id,"adapter_number":0,"port_number":0}
                    ]})),
                    StatusCode::CREATED,
                )
                .await
                .map_err(|error| format!("{name} switch link: {error}"))?;
                link_endpoints.extend(link["nodes"].as_array().into_iter().flatten().cloned());
            }

            for (node_id, _, name) in &hosts {
                let started = require_status(
                    &app,
                    Method::POST,
                    format!("/v3/projects/{current_project_id}/nodes/{node_id}/start"),
                    Some(serde_json::json!({})),
                    StatusCode::OK,
                )
                .await
                .map_err(|error| format!("start {name}: {error}"))?;
                if started["status"] != "started" {
                    return Err(format!("{name} did not start: {started}"));
                }
            }

            for (container, _, address, name) in &docker_hosts {
                for arguments in [
                    vec!["ip", "link", "set", "eth0", "up"],
                    vec!["ip", "addr", "add", address, "dev", "eth0"],
                ] {
                    let output = docker_command(&docker_binary, container, &arguments).await?;
                    if !output.status.success() {
                        return Err(format!(
                            "configure {name}: {}",
                            String::from_utf8_lossy(&output.stderr).trim()
                        ));
                    }
                }
            }

            let mut console = vpcs
                .connect_console(&vpcs_id)
                .await
                .map_err(|error| error.message().to_owned())?;
            let mut prompt = Vec::new();
            timeout(Duration::from_secs(10), async {
                loop {
                    let mut chunk = [0_u8; 2048];
                    let length = console.read(&mut chunk).await?;
                    prompt.extend_from_slice(&chunk[..length]);
                    if String::from_utf8_lossy(&prompt).contains("VPCS>") {
                        break Ok::<(), std::io::Error>(());
                    }
                }
            })
            .await
            .map_err(|_| "VPCS console prompt timed out")?
            .map_err(|error| error.to_string())?;

            for (container, _, _, name) in &docker_hosts[..2] {
                let targets = if name == "Docker A" {
                    ["10.0.10.2", "10.0.10.3"]
                } else {
                    ["10.0.10.1", "10.0.10.2"]
                };
                for target in targets {
                    let output = Command::new(&docker_binary)
                        .args(["exec", container, "ping", "-c", "1", "-W", "2", target])
                        .output()
                        .await
                        .map_err(|error| format!("Docker ping could not run: {error}"))?;
                    if !output.status.success()
                        || !String::from_utf8_lossy(&output.stdout).contains("1 packets received")
                    {
                        return Err(format!(
                            "{name} could not ping {target}: {}{}",
                            String::from_utf8_lossy(&output.stdout),
                            String::from_utf8_lossy(&output.stderr)
                        ));
                    }
                }
            }
            for target in ["10.0.10.1", "10.0.10.3"] {
                if !vpcs_ping(&mut console, target).await? {
                    return Err(format!("VPCS could not ping {target} on the switch"));
                }
            }
            let isolated = Command::new(&docker_binary)
                .args([
                    "exec",
                    &docker_hosts[2].0,
                    "ping",
                    "-c",
                    "1",
                    "-W",
                    "2",
                    "10.0.10.1",
                ])
                .output()
                .await
                .map_err(|error| format!("VLAN isolation ping could not run: {error}"))?;
            if isolated.status.success() {
                return Err("VLAN20 access port reached the VLAN10 Docker host".to_owned());
            }
            Ok(())
        }
        .await;

        let cleanup = if let Some(project_id) = project_id.as_deref() {
            let (status, body) = request(
                &app,
                Method::DELETE,
                format!("/v3/projects/{project_id}"),
                None,
            )
            .await;
            (status == StatusCode::NO_CONTENT)
                .then_some(())
                .ok_or_else(|| format!("builtin live test cleanup returned {status}: {body}"))
        } else {
            Ok(())
        };
        for endpoint in &link_endpoints {
            let endpoint_id = endpoint["runtime_endpoint_id"].as_str().unwrap_or_default();
            assert!(
                !ubridge.has_link_endpoint(endpoint_id).await,
                "uBridge endpoint remained after cleanup: {endpoint_id}"
            );
            if let Some(node_id) = endpoint["node_id"].as_str()
                && docker_hosts
                    .iter()
                    .any(|(_, docker_node, _, _)| docker_node == node_id)
            {
                let host = crate::ubridge::host_veth_name(endpoint_id).expect("Docker veth name");
                let output = Command::new("ip")
                    .args(["link", "show", "dev", &host])
                    .output()
                    .await
                    .expect("inspect cleaned Docker veth");
                assert!(
                    !output.status.success(),
                    "Docker host veth remained: {host}"
                );
            }
        }
        if let Some(node_id) = vpcs_node_id.as_deref() {
            assert_eq!(
                vpcs.status(node_id).await.expect("inspect cleaned VPCS"),
                "stopped"
            );
        }
        assert!(run.is_ok(), "builtin switch live test failed: {run:?}");
        assert!(
            cleanup.is_ok(),
            "builtin switch cleanup failed: {cleanup:?}"
        );
        let _ = switch_node_id;
    }

    #[tokio::test]
    #[ignore = "requires Docker, upstream VPCS/uBridge, and uBridge CAP_NET_ADMIN/CAP_NET_RAW"]
    async fn builtin_hub_live_forwards_docker_vpcs_traffic_and_cleans_links() {
        use std::{env, path::PathBuf};

        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            process::Command,
            time::{Duration, timeout},
        };

        let directory = TempDir::new().expect("temporary builtin hub live test state");
        let store = Store::connect(
            directory
                .path()
                .join("controller.sqlite")
                .to_str()
                .expect("UTF-8 test database path"),
        )
        .await
        .expect("test database connects");
        let vpcs_binary = env::var_os("NETWORK_SIMULATOR_TEST_VPCS")
            .expect("NETWORK_SIMULATOR_TEST_VPCS names upstream VPCS");
        let ubridge_binary = env::var_os("NETWORK_SIMULATOR_TEST_UBRIDGE")
            .expect("NETWORK_SIMULATOR_TEST_UBRIDGE names capable upstream uBridge");
        let vpcs = VpcsRuntime::with_paths(vpcs_binary, directory.path().join("vpcs-state"));
        let ubridge = crate::ubridge::UbridgeRuntime::with_paths(
            PathBuf::from(ubridge_binary),
            directory.path().join("ubridge.sock"),
        );
        let app = router(AppState::with_runtimes_and_ubridge(
            store,
            DockerRuntime::new(),
            QemuRuntime::new(),
            vpcs.clone(),
            DynamipsRuntime::new(),
            ubridge.clone(),
            crate::marketplace::MarketplaceCatalog::with_directory(
                directory.path().join("appliances"),
            ),
        ));
        let docker_binary = env::var_os("NETWORK_SIMULATOR_DOCKER")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("docker"));
        let mut project_id = None;
        let mut docker_node_id = None;
        let mut docker_container_id = None;
        let mut vpcs_node_id = None;
        let mut endpoints = Vec::new();

        let run: Result<(), String> = async {
            let (status, project) = request(
                &app,
                Method::POST,
                "/v3/projects".into(),
                Some(serde_json::json!({"name":"Builtin hub live test"})),
            )
            .await;
            if status != StatusCode::CREATED {
                return Err(format!("create project returned {status}: {project}"));
            }
            let pid = project["project_id"]
                .as_str()
                .ok_or_else(|| "project has no ID".to_owned())?
                .to_owned();
            project_id = Some(pid.clone());

            let (status, hub) = request(
                &app,
                Method::POST,
                format!("/v3/projects/{pid}/nodes"),
                Some(serde_json::json!({"name":"HUB1","node_type":"ethernet_hub"})),
            )
            .await;
            if status != StatusCode::CREATED {
                return Err(format!("create hub returned {status}: {hub}"));
            }
            let hub_id = hub["node_id"]
                .as_str()
                .ok_or_else(|| "hub has no ID".to_owned())?
                .to_owned();

            let (status, docker_node) = request(
                &app,
                Method::POST,
                format!("/v3/projects/{pid}/nodes"),
                Some(serde_json::json!({
                    "name":"Docker hub host","node_type":"docker","image":"alpine:latest",
                    "start_command":"/bin/sh -c 'sleep 3600'","adapters":1
                })),
            )
            .await;
            if status != StatusCode::CREATED {
                return Err(format!(
                    "create Docker node returned {status}: {docker_node}"
                ));
            }
            let docker_id = docker_node["node_id"]
                .as_str()
                .ok_or_else(|| "Docker node has no ID".to_owned())?
                .to_owned();
            let container_id = docker_node["container_id"]
                .as_str()
                .ok_or_else(|| "Docker node has no container ID".to_owned())?
                .to_owned();
            docker_node_id = Some(docker_id.clone());
            docker_container_id = Some(container_id.clone());

            let (status, vpcs_node) = request(
                &app,
                Method::POST,
                format!("/v3/projects/{pid}/nodes"),
                Some(serde_json::json!({
                    "name":"VPCS hub host","node_type":"vpcs","startup_script":"ip 10.20.0.2/24"
                })),
            )
            .await;
            if status != StatusCode::CREATED {
                return Err(format!("create VPCS node returned {status}: {vpcs_node}"));
            }
            let vpcs_id = vpcs_node["node_id"]
                .as_str()
                .ok_or_else(|| "VPCS node has no ID".to_owned())?
                .to_owned();
            vpcs_node_id = Some(vpcs_id.clone());

            for (port, host_id) in [(0_u64, docker_id.as_str()), (1, vpcs_id.as_str())] {
                let (status, link) = request(
                    &app,
                    Method::POST,
                    format!("/v3/projects/{pid}/links"),
                    Some(serde_json::json!({"nodes":[
                        {"node_id":hub_id,"adapter_number":0,"port_number":port},
                        {"node_id":host_id,"adapter_number":0,"port_number":0}
                    ]})),
                )
                .await;
                if status != StatusCode::CREATED {
                    return Err(format!("create hub link returned {status}: {link}"));
                }
                endpoints.extend(link["nodes"].as_array().into_iter().flatten().cloned());
            }

            for (node_id, name) in [(&docker_id, "Docker hub host"), (&vpcs_id, "VPCS hub host")] {
                let (status, body) = request(
                    &app,
                    Method::POST,
                    format!("/v3/projects/{pid}/nodes/{node_id}/start"),
                    Some(serde_json::json!({})),
                )
                .await;
                if status != StatusCode::OK || body["status"] != "started" {
                    return Err(format!("start {name} returned {status}: {body}"));
                }
            }

            for arguments in [
                vec!["exec", &container_id, "ip", "link", "set", "eth0", "up"],
                vec![
                    "exec",
                    &container_id,
                    "ip",
                    "addr",
                    "add",
                    "10.20.0.1/24",
                    "dev",
                    "eth0",
                ],
            ] {
                let output = Command::new(&docker_binary)
                    .args(arguments)
                    .output()
                    .await
                    .map_err(|error| format!("Docker command failed: {error}"))?;
                if !output.status.success() {
                    return Err(format!(
                        "configure Docker hub host failed: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    ));
                }
            }

            let mut console = vpcs
                .connect_console(&vpcs_id)
                .await
                .map_err(|error| error.message().to_owned())?;
            let mut prompt = Vec::new();
            timeout(Duration::from_secs(10), async {
                loop {
                    let mut chunk = [0_u8; 2048];
                    let length = console.read(&mut chunk).await?;
                    prompt.extend_from_slice(&chunk[..length]);
                    if String::from_utf8_lossy(&prompt).contains("VPCS>") {
                        break Ok::<(), std::io::Error>(());
                    }
                }
            })
            .await
            .map_err(|_| "VPCS console prompt timed out")?
            .map_err(|error| error.to_string())?;
            for attempt in 0..2 {
                console
                    .write_all(b"ping 10.20.0.1 -c 1\r")
                    .await
                    .map_err(|error| error.to_string())?;
                let mut output = Vec::new();
                timeout(Duration::from_secs(8), async {
                    loop {
                        let mut chunk = [0_u8; 2048];
                        let length = console.read(&mut chunk).await?;
                        output.extend_from_slice(&chunk[..length]);
                        let text = String::from_utf8_lossy(&output);
                        if text.contains("bytes from") || text.contains("VPCS>") {
                            break Ok::<(), std::io::Error>(());
                        }
                    }
                })
                .await
                .map_err(|_| "VPCS hub ping timed out")?
                .map_err(|error| error.to_string())?;
                if String::from_utf8_lossy(&output).contains("bytes from") {
                    break;
                }
                if attempt == 1 {
                    return Err(format!(
                        "VPCS could not ping Docker through hub: {}",
                        String::from_utf8_lossy(&output)
                    ));
                }
            }

            let output = Command::new(&docker_binary)
                .args([
                    "exec",
                    &container_id,
                    "ping",
                    "-c",
                    "1",
                    "-W",
                    "2",
                    "10.20.0.2",
                ])
                .output()
                .await
                .map_err(|error| format!("Docker ping could not run: {error}"))?;
            if !output.status.success() {
                return Err(format!(
                    "Docker could not ping VPCS through hub: {}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
            Ok(())
        }
        .await;

        let cleanup = if let Some(pid) = project_id.as_deref() {
            let (status, body) =
                request(&app, Method::DELETE, format!("/v3/projects/{pid}"), None).await;
            (status == StatusCode::NO_CONTENT)
                .then_some(())
                .ok_or_else(|| format!("hub live test cleanup returned {status}: {body}"))
        } else {
            Ok(())
        };
        for endpoint in &endpoints {
            let endpoint_id = endpoint["runtime_endpoint_id"].as_str().unwrap_or_default();
            assert!(
                !ubridge.has_link_endpoint(endpoint_id).await,
                "uBridge endpoint remained after hub cleanup: {endpoint_id}"
            );
        }
        if let Some(node_id) = vpcs_node_id.as_deref() {
            assert_eq!(
                vpcs.status(node_id).await.expect("inspect VPCS cleanup"),
                "stopped"
            );
        }
        if let Some(node_id) = docker_node_id.as_deref() {
            if let Some(endpoint) = endpoints
                .iter()
                .find(|endpoint| endpoint["node_id"] == node_id)
            {
                let host = crate::ubridge::host_veth_name(
                    endpoint["runtime_endpoint_id"].as_str().unwrap_or_default(),
                )
                .expect("Docker host veth name");
                let output = Command::new("ip")
                    .args(["link", "show", "dev", &host])
                    .output()
                    .await
                    .expect("inspect cleaned Docker veth");
                assert!(
                    !output.status.success(),
                    "Docker host veth remained: {host}"
                );
            }
        }
        if let Some(container_id) = docker_container_id.as_deref() {
            let output = Command::new(&docker_binary)
                .args(["inspect", container_id])
                .output()
                .await
                .expect("inspect removed Docker hub host");
            assert!(
                !output.status.success(),
                "Docker container remained: {container_id}"
            );
        }
        assert!(run.is_ok(), "builtin hub live test failed: {run:?}");
        assert!(cleanup.is_ok(), "builtin hub cleanup failed: {cleanup:?}");
    }

    #[tokio::test]
    #[ignore = "requires Docker, upstream VPCS/uBridge, KVM, and the installed MikroTik CHR image"]
    async fn chr_routes_vpcs_and_docker_over_controller_udp_nios() {
        use std::{env, path::PathBuf};

        use tokio::{
            io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
            net::UnixStream,
            process::Command,
            time::{Duration, timeout},
        };
        use uuid::Uuid;

        fn strip_ansi(input: &[u8]) -> String {
            let mut output = String::new();
            let mut index = 0;
            while index < input.len() {
                if input[index] == 0x1b {
                    index += 1;
                    if input.get(index) == Some(&b'[') {
                        index += 1;
                        while index < input.len() {
                            let byte = input[index];
                            index += 1;
                            if (0x40..=0x7e).contains(&byte) {
                                break;
                            }
                        }
                    } else if index < input.len() {
                        index += 1;
                    }
                } else {
                    let byte = input[index];
                    index += 1;
                    if byte == b'\r' || byte == b'\n' || byte.is_ascii_graphic() || byte == b' ' {
                        output.push(byte as char);
                    }
                }
            }
            output
        }

        async fn read_until(
            serial: &mut BufReader<UnixStream>,
            marker: &str,
            duration: Duration,
        ) -> Result<String, String> {
            let mut bytes = Vec::new();
            timeout(duration, async {
                loop {
                    let (read, found) = {
                        let available =
                            serial.fill_buf().await.map_err(|error| error.to_string())?;
                        if available.is_empty() {
                            return Err("QEMU serial socket closed".to_owned());
                        }
                        let previous_len = bytes.len();
                        bytes.extend_from_slice(available);
                        let marker_end = bytes
                            .windows(marker.len())
                            .position(|window| window == marker.as_bytes())
                            .map(|position| position + marker.len());
                        match marker_end {
                            Some(end) => {
                                (end.saturating_sub(previous_len).min(available.len()), true)
                            }
                            None => (available.len(), false),
                        }
                    };
                    serial.consume(read);
                    if found {
                        return Ok::<String, String>(strip_ansi(&bytes));
                    }
                }
            })
            .await
            .map_err(|_| {
                format!(
                    "timed out waiting for {marker:?}; output: {}",
                    strip_ansi(&bytes)
                )
            })?
        }

        async fn routeros_command(
            serial: &mut BufReader<UnixStream>,
            command: &str,
        ) -> Result<String, String> {
            serial
                .write_all(command.as_bytes())
                .await
                .map_err(|error| error.to_string())?;
            serial
                .write_all(b"\r")
                .await
                .map_err(|error| error.to_string())?;
            read_until(serial, &format!("{command}\r\n"), Duration::from_secs(5)).await?;
            read_until(serial, "[admin@CHR] >", Duration::from_secs(15)).await
        }

        fn qemu_test_mac(node_id: &str, adapter_number: u32) -> Result<String, String> {
            let id = Uuid::parse_str(node_id).map_err(|error| error.to_string())?;
            let bytes = id.as_bytes();
            Ok(format!(
                "52:54:00:{:02x}:{:02x}:{:02x}",
                bytes[13] ^ ((adapter_number >> 16) as u8),
                bytes[14] ^ ((adapter_number >> 8) as u8),
                bytes[15] ^ (adapter_number as u8)
            ))
        }

        fn qemu_interface_for_mac(output: &str, mac: &str) -> Option<String> {
            let mut name = None;
            for line in output.lines() {
                if let Some((_, value)) = line.split_once("name=") {
                    let value = value.trim_start_matches('"');
                    let end = value
                        .find('"')
                        .or_else(|| value.find(char::is_whitespace))
                        .unwrap_or(value.len());
                    name = Some(value[..end].to_owned());
                }
                if line
                    .to_ascii_lowercase()
                    .contains(&mac.to_ascii_lowercase())
                {
                    return name;
                }
            }
            None
        }

        let directory = TempDir::new().expect("temporary CHR test state");
        let store = Store::connect(
            directory
                .path()
                .join("controller.sqlite")
                .to_str()
                .expect("UTF-8 test database path"),
        )
        .await
        .expect("test database connects");
        sqlx::query("UPDATE auth_config SET auth_enabled = 0 WHERE singleton = 1")
            .execute(store.pool())
            .await
            .expect("keep test authentication disabled");
        sqlx::query(
            "INSERT INTO auth_users (user_id, username, full_name, password_hash, is_superadmin) VALUES (?, 'live-test-admin', 'Live test administrator', 'unused', 1)",
        )
        .bind(Uuid::new_v4().to_string())
        .execute(store.pool())
        .await
        .expect("seed loopback superadmin principal for the auth middleware");
        let qemu = QemuRuntime::new();
        let docker = DockerRuntime::new();
        let vpcs_binary = env::var_os("NETWORK_SIMULATOR_TEST_VPCS")
            .expect("NETWORK_SIMULATOR_TEST_VPCS names upstream VPCS");
        let ubridge_binary = env::var_os("NETWORK_SIMULATOR_TEST_UBRIDGE")
            .expect("NETWORK_SIMULATOR_TEST_UBRIDGE names capable upstream uBridge");
        let vpcs = VpcsRuntime::with_paths(vpcs_binary, directory.path().join("vpcs-state"));
        let ubridge = crate::ubridge::UbridgeRuntime::with_paths(
            PathBuf::from(ubridge_binary),
            directory.path().join("ubridge.sock"),
        );
        let app = router(AppState::with_runtimes_and_ubridge(
            store,
            docker.clone(),
            qemu.clone(),
            vpcs.clone(),
            DynamipsRuntime::new(),
            ubridge.clone(),
            crate::marketplace::MarketplaceCatalog::with_directory(
                directory.path().join("appliances"),
            ),
        ));
        let mut project_id: Option<String> = None;
        let mut qemu_vm = None;
        let mut docker_container: Option<String> = None;
        let mut vpcs_node_id: Option<String> = None;
        let mut docker_node_id: Option<String> = None;
        let mut link_endpoints = Vec::<Value>::new();
        let mut link_ids = Vec::<String>::new();
        let test_started_at = std::time::Instant::now();
        let mut links_created_at_ms = None;
        let mut chr_start_returned_at_ms = None;

        let run: Result<(), String> = async {
            create_custom_from_runtime(&app, "qemu", "Live CHR").await;
            let (status, templates) = request(&app, Method::GET, "/v3/templates".into(), None).await;
            if status != StatusCode::OK {
                return Err(format!("template list returned {status}: {templates}"));
            }
            let template_id = templates.as_array().into_iter().flatten()
                .find(|template| template["name"] == "Live CHR · Default")
                .and_then(|template| template["template_id"].as_str())
                .ok_or_else(|| format!("CHR template is missing: {templates}"))?;
            let (status, project) = request(
                &app,
                Method::POST,
                "/v3/projects".into(),
                Some(serde_json::json!({ "name": "Live CHR NIO matrix" })),
            ).await;
            if status != StatusCode::CREATED {
                return Err(format!("project create returned {status}: {project}"));
            }
            let current_project_id = project["project_id"].as_str().ok_or("project has no ID")?.to_owned();
            project_id = Some(current_project_id.clone());

            let (status, chr) = request(
                &app,
                Method::POST,
                format!("/v3/projects/{current_project_id}/templates/{template_id}"),
                Some(serde_json::json!({
                    "name": "CHR",
                    "memory": 512,
                    "cpus": 1,
                    "adapters": 3,
                    "adapter_type": "virtio-net-pci"
                })),
            ).await;
            if status != StatusCode::CREATED {
                return Err(format!("CHR node create returned {status}: {chr}"));
            }
            let chr_node_id = chr["node_id"].as_str().ok_or("CHR node has no ID")?;
            let (status, configured_chr) = request(
                &app,
                Method::PUT,
                format!("/v3/projects/{current_project_id}/nodes/{chr_node_id}/config"),
                Some(serde_json::json!({ "adapters": 3 })),
            ).await;
            if status != StatusCode::OK {
                return Err(format!("CHR adapter configuration returned {status}: {configured_chr}"));
            }
            qemu_vm = Some(qemu.vm_from_node(&configured_chr).map_err(|error| error.message().to_owned())?);

            let (status, vpcs_node) = request(
                &app,
                Method::POST,
                format!("/v3/projects/{current_project_id}/nodes"),
                Some(serde_json::json!({
                    "name": "VPCS peer",
                    "node_type": "vpcs",
                    "startup_script": "ip 10.0.1.2/24"
                })),
            ).await;
            if status != StatusCode::CREATED {
                return Err(format!("VPCS node create returned {status}: {vpcs_node}"));
            }
            vpcs_node_id = vpcs_node["node_id"].as_str().map(str::to_owned);
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
            ).await;
            if status != StatusCode::CREATED {
                return Err(format!("Docker node create returned {status}: {docker_node}"));
            }
            docker_node_id = docker_node["node_id"].as_str().map(str::to_owned);
            docker_container = docker_node["container_id"].as_str().map(str::to_owned);

            let vpcs_link = serde_json::json!({ "nodes": [
                { "node_id": chr["node_id"], "adapter_number": 0, "port_number": 0 },
                { "node_id": vpcs_node["node_id"], "adapter_number": 0, "port_number": 0 }
            ] });
            let (status, vpcs_link) = request(
                &app,
                Method::POST,
                format!("/v3/projects/{current_project_id}/links"),
                Some(vpcs_link),
            ).await;
            if status != StatusCode::CREATED {
                return Err(format!("CHR↔VPCS link returned {status}: {vpcs_link}"));
            }
            link_ids.push(vpcs_link["link_id"].as_str().ok_or("VPCS link has no ID")?.to_owned());
            link_endpoints.extend(vpcs_link["nodes"].as_array().into_iter().flatten().cloned());

            let docker_link = serde_json::json!({ "nodes": [
                { "node_id": chr["node_id"], "adapter_number": 1, "port_number": 0 },
                { "node_id": docker_node["node_id"], "adapter_number": 0, "port_number": 0 }
            ] });
            let (status, docker_link) = request(
                &app,
                Method::POST,
                format!("/v3/projects/{current_project_id}/links"),
                Some(docker_link),
            ).await;
            if status != StatusCode::CREATED {
                return Err(format!("CHR↔Docker link returned {status}: {docker_link}"));
            }
            link_ids.push(docker_link["link_id"].as_str().ok_or("Docker link has no ID")?.to_owned());
            link_endpoints.extend(docker_link["nodes"].as_array().into_iter().flatten().cloned());
            links_created_at_ms = Some(test_started_at.elapsed().as_millis());

            for node in [&chr, &vpcs_node, &docker_node] {
                let node_id = node["node_id"].as_str().ok_or("node has no ID")?;
                let (status, started) = request(
                    &app,
                    Method::POST,
                    format!("/v3/projects/{current_project_id}/nodes/{node_id}/start"),
                    Some(serde_json::json!({})),
                ).await;
                if status != StatusCode::OK || started["status"] != "started" {
                    return Err(format!("node start returned {status}: {started}"));
                }
                if node_id == chr_node_id {
                    chr_start_returned_at_ms = Some(test_started_at.elapsed().as_millis());
                }
            }

            let vm = qemu_vm.as_ref().ok_or("CHR runtime model missing")?;
            let serial_socket = UnixStream::connect(&vm.serial_socket)
                .await
                .map_err(|error| format!("CHR serial socket did not open: {error}"))?;
            let mut serial = BufReader::new(serial_socket);
            read_until(&mut serial, "CHR Login:", Duration::from_secs(180)).await?;
            let chr_login_prompt_at_ms = test_started_at.elapsed().as_millis();
            serial.write_all(b"admin\r").await.map_err(|error| error.to_string())?;
            read_until(&mut serial, "Password:", Duration::from_secs(10)).await?;
            serial.write_all(b"\r").await.map_err(|error| error.to_string())?;
            read_until(
                &mut serial,
                "Do you want to see the software license? [Y/n]:",
                Duration::from_secs(30),
            ).await?;
            serial.write_all(b"n\r").await.map_err(|error| error.to_string())?;
            read_until(&mut serial, "new password>", Duration::from_secs(20)).await?;
            serial.write_all(b"\x03").await.map_err(|error| error.to_string())?;
            read_until(&mut serial, "[admin@CHR] >", Duration::from_secs(20)).await?;

            let interface_output = routeros_command(&mut serial, "/interface ethernet print detail").await?;
            let interface0 = qemu_interface_for_mac(
                &interface_output,
                &qemu_test_mac(chr["node_id"].as_str().ok_or("CHR node has no ID")?, 0)?,
            );
            let interface1 = qemu_interface_for_mac(
                &interface_output,
                &qemu_test_mac(chr["node_id"].as_str().ok_or("CHR node has no ID")?, 1)?,
            );
            let (Some(interface0), Some(interface1)) = (interface0, interface1) else {
                return Err(format!("CHR did not expose both linked interfaces: {interface_output}"));
            };
            eprintln!("CHR link MAC mapping: adapter 0={interface0}, adapter 1={interface1}; interfaces: {interface_output}");
            routeros_command(&mut serial, &format!("/ip address add address=10.0.1.1/24 interface={interface0}")).await?;
            routeros_command(&mut serial, &format!("/ip address add address=10.0.2.1/24 interface={interface1}")).await?;
            routeros_command(&mut serial, &format!("/tool sniffer set filter-interface={interface1}")).await?;
            routeros_command(&mut serial, "/tool sniffer start").await?;

            let container_id = docker_container.as_deref().ok_or("Docker node has no container ID")?;
            let docker_binary = env::var_os("NETWORK_SIMULATOR_DOCKER")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("docker"));
            for args in [
                vec!["ip", "addr", "add", "10.0.2.2/24", "dev", "eth0"],
                vec!["ip", "link", "set", "eth0", "up"],
            ] {
                let output = Command::new(&docker_binary)
                    .args(["exec", container_id])
                    .args(args)
                    .output()
                    .await
                    .map_err(|error| format!("could not configure Alpine eth0: {error}"))?;
                if !output.status.success() {
                    return Err(format!("could not configure Alpine eth0: {}", String::from_utf8_lossy(&output.stderr).trim()));
                }
            }
            let capture_paths = link_endpoints
                .iter()
                .filter(|endpoint| {
                    endpoint["node_id"].as_str()
                        == Some(docker_node_id.as_deref().unwrap_or_default())
                        || (endpoint["node_id"].as_str() == Some(chr_node_id)
                            && endpoint["adapter_number"].as_u64() == Some(1))
                })
                .map(|endpoint| {
                    let endpoint_id = endpoint["runtime_endpoint_id"]
                        .as_str()
                        .unwrap_or_default();
                    (
                        endpoint_id.to_owned(),
                        directory.path().join(format!("{endpoint_id}.pcap")),
                    )
                })
                .collect::<Vec<_>>();
            for (endpoint_id, path) in &capture_paths {
                ubridge
                    .start_link_capture(endpoint_id, &path.to_string_lossy())
                    .await
                    .map_err(|error| {
                        format!("could not capture CHR/Docker link: {}", error.message())
                    })?;
            }
            let first_alpine_probe_at_ms = test_started_at.elapsed().as_millis();
            // A live CHR run can drop the first echo immediately after fresh
            // ARP resolution even though the ARP exchange and later echoes are
            // valid. Bound this reachability check to three one-packet probes;
            // each probe has a four-second deadline.
            let mut docker_ping_ok = false;
            let mut docker_ping_attempts = 0;
            let mut docker_ping_output = String::new();
            let mut first_docker_ping_output = String::new();
            for attempt in 0..1 {
                docker_ping_attempts = attempt + 1;
                let output = Command::new(&docker_binary)
                    .args([
                        "exec",
                        container_id,
                        "ping",
                        "-c",
                        "1",
                        "-W",
                        "4",
                        "10.0.2.1",
                    ])
                    .output()
                    .await
                    .map_err(|error| format!("could not ping CHR from Alpine: {error}"))?;
                docker_ping_output = format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                if attempt == 0 {
                    first_docker_ping_output.clone_from(&docker_ping_output);
                }
                docker_ping_ok = output.status.success()
                    && docker_ping_output.contains("1 packets received");
                if docker_ping_ok {
                    break;
                }
            }
            let _ = routeros_command(&mut serial, "/tool sniffer stop").await;
            let chr_guest_capture = routeros_command(&mut serial, "/tool sniffer packet print detail without-paging").await?;
            let mut network_debug = String::new();
            if !docker_ping_ok {
                network_debug.push_str(&format!("\nCHR guest-side packet capture:\n{chr_guest_capture}"));
                network_debug.push_str(&format!(
                    "\nAlpine used {docker_ping_attempts} bounded probes; first probe output:\n{first_docker_ping_output}\nTest timing from setup: CHR/VPCS/Docker links created +{} ms; CHR start request returned +{} ms; CHR login prompt +{} ms; first Alpine probe +{} ms. Link records existed before startup, but the API launches QEMU with no NIC and hot-attaches link NIOs over QMP before the start request returns.\nQEMU NIC model: virtio-net-pci; expected adapter MACs: {}, {}",
                    links_created_at_ms.unwrap_or_default(),
                    chr_start_returned_at_ms.unwrap_or_default(),
                    chr_login_prompt_at_ms,
                    first_alpine_probe_at_ms,
                    qemu_test_mac(chr_node_id, 0)?,
                    qemu_test_mac(chr_node_id, 1)?
                ));
                let retry = Command::new(&docker_binary)
                    .args(["exec", container_id, "ping", "-c", "3", "-W", "1", "10.0.2.1"])
                    .output()
                    .await
                    .map_err(|error| error.to_string())?;
                network_debug.push_str(&format!(
                    "\nAlpine three-packet retry:\n{}{}",
                    String::from_utf8_lossy(&retry.stdout),
                    String::from_utf8_lossy(&retry.stderr)
                ));
                docker_ping_ok = retry.status.success()
                    && String::from_utf8_lossy(&retry.stdout).contains("3 packets received");
                eprintln!(
                    "First Alpine probe:\n{first_docker_ping_output}\nCHR guest-side capture:\n{chr_guest_capture}\nRetry:\n{}{}",
                    String::from_utf8_lossy(&retry.stdout),
                    String::from_utf8_lossy(&retry.stderr)
                );
                let chr_arp = routeros_command(&mut serial, "/ip arp print detail").await?;
                let chr_interfaces =
                    routeros_command(&mut serial, "/interface ethernet print detail").await?;
                let chr_stats = routeros_command(&mut serial, "/interface ethernet print stats").await?;
                let chr_firewall = routeros_command(
                    &mut serial,
                    "/ip firewall filter print detail stats",
                )
                .await?;
                let chr_interface_lists =
                    routeros_command(&mut serial, "/interface list member print detail").await?;
                network_debug.push_str(&format!(
                    "\nCHR ARP before reverse ping:\n{chr_arp}\nCHR interfaces:\n{chr_interfaces}\nCHR interface stats:\n{chr_stats}\nCHR IPv4 firewall rules and counters:\n{chr_firewall}\nCHR interface list members:\n{chr_interface_lists}"
                ));
                for endpoint in link_endpoints.iter().filter(|endpoint| {
                    endpoint["node_id"].as_str()
                        == Some(docker_node_id.as_deref().unwrap_or_default())
                }) {
                    let endpoint_id = endpoint["runtime_endpoint_id"].as_str().unwrap_or_default();
                    let host = crate::ubridge::host_veth_name(endpoint_id)
                        .map_err(|error| error.message().to_owned())?;
                    let link = Command::new("ip")
                        .args(["-s", "-d", "link", "show", "dev", &host])
                        .output()
                        .await
                        .map_err(|error| error.to_string())?;
                    let offloads = Command::new("ethtool")
                        .args(["-k", &host])
                        .output()
                        .await
                        .map(|output| {
                            format!(
                                "{}{}",
                                String::from_utf8_lossy(&output.stdout),
                                String::from_utf8_lossy(&output.stderr)
                            )
                        })
                        .unwrap_or_else(|error| format!("ethtool unavailable: {error}"));
                    let fdb = Command::new("bridge")
                        .args(["fdb", "show", "dev", &host])
                        .output()
                        .await
                        .map_err(|error| error.to_string())?;
                    let guest = Command::new(&docker_binary)
                        .args([
                            "exec",
                            container_id,
                            "sh",
                            "-c",
                            "ip link show dev eth0; ip -s link show dev eth0; ip neigh show dev eth0",
                        ])
                        .output()
                        .await
                        .map_err(|error| error.to_string())?;
                    network_debug.push_str(&format!(
                        "\nHost veth {host}:\n{}\nHost veth offloads:\n{offloads}\nDocker eth0/link neighbors:\n{}{}\nLinux bridge FDB for this endpoint:\n{}",
                        String::from_utf8_lossy(&link.stdout),
                        String::from_utf8_lossy(&guest.stdout),
                        String::from_utf8_lossy(&guest.stderr),
                        String::from_utf8_lossy(&fdb.stdout)
                    ));
                }
                for (endpoint_id, path) in &capture_paths {
                    let _ = ubridge.stop_link_capture(endpoint_id).await;
                    let capture = Command::new("tcpdump")
                        .args(["-nn", "-e", "-tt", "-vvv", "-xx", "-r"])
                        .arg(path)
                        .arg("arp or icmp")
                        .output()
                        .await
                        .map_err(|error| error.to_string())?;
                    network_debug.push_str(&format!(
                        "\nuBridge capture {endpoint_id}:\n{}{}",
                        String::from_utf8_lossy(&capture.stdout),
                        String::from_utf8_lossy(&capture.stderr)
                    ));
                }
            }
            let chr_ping_output = routeros_command(&mut serial, "/ping 10.0.2.2 count=1").await?;

            let mut vpcs_console = vpcs.connect_console(vpcs_node["node_id"].as_str().ok_or("VPCS node has no ID")?)
                .await.map_err(|error| error.message().to_owned())?;
            let mut vpcs_output = Vec::new();
            timeout(Duration::from_secs(5), async {
                loop {
                    let mut chunk = [0_u8; 2048];
                    let length = vpcs_console.read(&mut chunk).await?;
                    vpcs_output.extend_from_slice(&chunk[..length]);
                    if String::from_utf8_lossy(&vpcs_output).contains("VPCS>") { break Ok::<(), std::io::Error>(()); }
                }
            }).await.map_err(|_| "VPCS console prompt timed out")?
                .map_err(|error| error.to_string())?;
            vpcs_output.clear();
            let mut vpcs_ping_ok = false;
            for _ in 0..3 {
                vpcs_console.write_all(b"ping 10.0.1.1 -c 1\r").await.map_err(|error| error.to_string())?;
                let mut attempt_output = Vec::new();
                timeout(Duration::from_secs(8), async {
                    loop {
                        let mut chunk = [0_u8; 2048];
                        let length = vpcs_console.read(&mut chunk).await?;
                        attempt_output.extend_from_slice(&chunk[..length]);
                        let text = String::from_utf8_lossy(&attempt_output);
                        if text.contains("bytes from 10.0.1.1") || text.contains("VPCS>") {
                            break Ok::<(), std::io::Error>(());
                        }
                    }
                }).await.map_err(|_| format!("VPCS↔CHR ping timed out: {}", String::from_utf8_lossy(&attempt_output)))?
                    .map_err(|error| error.to_string())?;
                vpcs_ping_ok = String::from_utf8_lossy(&attempt_output).contains("bytes from 10.0.1.1");
                vpcs_output.extend_from_slice(&attempt_output);
                if vpcs_ping_ok { break; }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            if !docker_ping_ok || !chr_ping_output.contains("received=1") || !vpcs_ping_ok {
                return Err(format!(
                    "QEMU connectivity failed; Alpine ping: {docker_ping_output}; CHR ping: {chr_ping_output}; VPCS ping: {}{network_debug}",
                    String::from_utf8_lossy(&vpcs_output),
                ));
            }

            let (status, hot_peer) = request(
                &app,
                Method::POST,
                format!("/v3/projects/{current_project_id}/nodes"),
                Some(serde_json::json!({
                    "name": "Hot added NIC peer",
                    "node_type": "vpcs",
                    "startup_script": "ip 10.0.3.2/24"
                })),
            ).await;
            if status != StatusCode::CREATED {
                return Err(format!("hot-add peer create returned {status}: {hot_peer}"));
            }
            let hot_peer_id = hot_peer["node_id"].as_str().ok_or("hot-add peer has no ID")?;
            let (status, started) = request(
                &app,
                Method::POST,
                format!("/v3/projects/{current_project_id}/nodes/{hot_peer_id}/start"),
                Some(serde_json::json!({})),
            ).await;
            if status != StatusCode::OK || started["status"] != "started" {
                return Err(format!("hot-add peer start returned {status}: {started}"));
            }
            let (status, hot_link) = request(
                &app,
                Method::POST,
                format!("/v3/projects/{current_project_id}/links"),
                Some(serde_json::json!({ "nodes": [
                    { "node_id": chr_node_id, "adapter_number": 2, "port_number": 0 },
                    { "node_id": hot_peer_id, "adapter_number": 0, "port_number": 0 }
                ] })),
            ).await;
            if status != StatusCode::CREATED {
                return Err(format!("live QEMU NIC hot-add returned {status}: {hot_link}"));
            }
            link_ids.push(hot_link["link_id"].as_str().ok_or("hot-add link has no ID")?.to_owned());

            let docker_link_path = format!("/v3/projects/{current_project_id}/links/{}", link_ids[1]);
            for (suspend, expected_success) in [(true, false), (false, true)] {
                let (status, body) = request(
                    &app,
                    Method::PUT,
                    docker_link_path.clone(),
                    Some(serde_json::json!({"suspend":suspend})),
                ).await;
                if status != StatusCode::OK {
                    return Err(format!("live link suspend={suspend} returned {status}: {body}"));
                }
                let probe = Command::new(&docker_binary)
                    .args(["exec", container_id, "ping", "-c", "1", "-W", "2", "10.0.2.1"])
                    .output().await.map_err(|error| error.to_string())?;
                if probe.status.success() != expected_success {
                    return Err(format!(
                        "live link suspend={suspend} had unexpected ping result: {}{}",
                        String::from_utf8_lossy(&probe.stdout),
                        String::from_utf8_lossy(&probe.stderr)
                    ));
                }
            }

            for endpoint in &link_endpoints {
                let endpoint_id = endpoint["runtime_endpoint_id"].as_str().unwrap_or_default();
                let _ = ubridge.stop_link_capture(endpoint_id).await;
            }
            let (status, capture) = request(
                &app,
                Method::POST,
                format!("{docker_link_path}/capture/start"),
                Some(serde_json::json!({"capture_file_name":"live-route.pcap"})),
            ).await;
            if status != StatusCode::CREATED {
                return Err(format!("live capture start returned {status}: {capture}"));
            }
            let probe = Command::new(&docker_binary)
                .args(["exec", container_id, "ping", "-c", "1", "-W", "3", "10.0.2.1"])
                .output().await.map_err(|error| error.to_string())?;
            if !probe.status.success() {
                return Err(format!("ping during live packet capture failed: {}", String::from_utf8_lossy(&probe.stdout)));
            }
            let (status, _) = request(
                &app,
                Method::POST,
                format!("{docker_link_path}/capture/stop"),
                Some(serde_json::json!({})),
            ).await;
            if status != StatusCode::NO_CONTENT {
                return Err(format!("live capture stop returned {status}"));
            }
            let (status, headers, bytes) = request_bytes(
                &app,
                Method::GET,
                format!("{docker_link_path}/capture/file"),
            ).await;
            if status != StatusCode::OK
                || !headers.get("content-type").and_then(|value| value.to_str().ok()).unwrap_or_default().contains("pcap")
                || bytes.len() < 40
            {
                return Err(format!("live capture download was invalid: status={status}, bytes={}", bytes.len()));
            }

            for link_id in &link_ids {
                let (status, body) = request(
                    &app,
                    Method::DELETE,
                    format!("/v3/projects/{current_project_id}/links/{link_id}"),
                    None,
                ).await;
                if status != StatusCode::NO_CONTENT {
                    return Err(format!("live link deletion returned {status}: {body}"));
                }
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
                .ok_or_else(|| format!("temporary CHR project cleanup returned {status}: {body}"))
        } else {
            Ok(())
        };
        if let Some(vm) = qemu_vm.as_ref() {
            assert_eq!(
                qemu.status(vm).await.expect("inspect cleaned CHR"),
                "stopped"
            );
            assert!(
                !vm.pid_file.exists(),
                "QEMU pid file remained after project deletion"
            );
        }
        for endpoint in &link_endpoints {
            assert!(
                !ubridge
                    .has_link_endpoint(endpoint["runtime_endpoint_id"].as_str().unwrap_or_default())
                    .await
            );
        }
        if let Some(node_id) = vpcs_node_id.as_deref() {
            assert_eq!(
                vpcs.status(node_id).await.expect("inspect cleaned VPCS"),
                "stopped"
            );
        }
        if let Some(container_id) = docker_container.as_deref() {
            let docker_binary = env::var_os("NETWORK_SIMULATOR_DOCKER")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("docker"));
            let inspect = Command::new(&docker_binary)
                .args(["inspect", container_id])
                .output()
                .await
                .expect("inspect cleaned Alpine container");
            assert!(
                !inspect.status.success(),
                "Alpine container remained after project deletion"
            );
        }
        if let Some(node_id) = docker_node_id.as_deref() {
            for endpoint in link_endpoints
                .iter()
                .filter(|endpoint| endpoint["node_id"].as_str() == Some(node_id))
            {
                let host = crate::ubridge::host_veth_name(
                    endpoint["runtime_endpoint_id"].as_str().unwrap(),
                )
                .expect("Docker host veth name");
                let inspect = Command::new("ip")
                    .args(["link", "show", "dev", &host])
                    .output()
                    .await
                    .expect("inspect cleaned Docker host veth");
                assert!(
                    !inspect.status.success(),
                    "Docker host veth remained after project deletion: {host}"
                );
            }
        }
        assert!(run.is_ok(), "live CHR NIO matrix failed: {run:?}");
        assert!(cleanup.is_ok(), "live CHR cleanup failed: {cleanup:?}");
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
        create_custom_from_runtime(&app, "qemu", "Test Router").await;
        let (status, templates) = request(&app, Method::GET, "/v3/templates".into(), None).await;
        assert_eq!(status, StatusCode::OK);
        let template = templates
            .as_array()
            .unwrap()
            .iter()
            .find(|template| template["template_type"] == "qemu")
            .unwrap_or_else(|| {
                panic!("custom QEMU appliance did not create a palette template: {templates:#}")
            });
        let template_id = template["template_id"].as_str().unwrap();
        assert_eq!(template["name"], "Test Router · Default");

        let (_, project) = request(
            &app,
            Method::POST,
            "/v3/projects".into(),
            Some(serde_json::json!({ "name": "QEMU lifecycle lab" })),
        )
        .await;
        let project_id = project["project_id"].as_str().unwrap();
        let mut qemu_nodes = Vec::new();
        for name in ["Router A"] {
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
        let node_a = qemu_nodes[0]["node_id"].as_str().unwrap();
        let (status, configured) = request(
            &app,
            Method::PUT,
            format!("/v3/projects/{project_id}/nodes/{node_a}/config"),
            Some(serde_json::json!({
                "memory": 1024,
                "cpus": 2,
                "adapters": 2,
                "adapter_type": "virtio-net-pci",
                "hda_disk_interface": "virtio",
                "boot_priority": "dc"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(configured["memory"], 1024);
        assert_eq!(configured["cpus"], 2);
        assert_eq!(configured["adapter_type"], "virtio-net-pci");
        assert_eq!(configured["hda_disk_interface"], "virtio");
        assert_eq!(configured["boot_priority"], "dc");

        let console_path = format!(
            "/v3/compute/projects/{project_id}/qemu/nodes/{}/console/ws",
            qemu_nodes[0]["node_id"].as_str().unwrap()
        );
        assert_eq!(
            websocket_status(&app, console_path.clone()).await,
            StatusCode::CONFLICT
        );

        let (status, node) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes/{node_a}/start"),
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(node["status"], "started");
        let (status, error) = request(
            &app,
            Method::PUT,
            format!("/v3/projects/{project_id}/nodes/{node_a}/config"),
            Some(serde_json::json!({ "memory": 2048 })),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("Stop this device")
        );
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

        let args = std::fs::read_to_string(directory.path().join("qemu-arguments.log"))
            .expect("QEMU launch arguments were recorded");
        assert!(args.contains("-nic none"));
        assert!(args.contains("-chardev socket,id=serial0,path="));
        assert!(args.contains("server=on,wait=off -serial chardev:serial0"));
        assert!(args.contains("-m 1024 -smp cpus=2,maxcpus=2,sockets=1 -boot order=dc"));
        assert!(args.contains("if=virtio,index=0,media=disk,id=drive0,format=qcow2"));
        for node in &qemu_nodes {
            let node_id = node["node_id"].as_str().unwrap();
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
                .all(|template| template["template_type"] != "qemu"),
            "an uploaded disk image must not create a palette template: {templates:#}"
        );
        let (_, manager) = request(
            &app,
            Method::GET,
            "/v3/marketplace/gns3/manager".into(),
            None,
        )
        .await;
        let source_template_id = manager["custom_image_options"]
            .as_array()
            .unwrap()
            .iter()
            .find(|template| template["hda_disk_image"] == uploaded["image"])
            .unwrap()["template_id"]
            .as_str()
            .unwrap();
        let (status, _) = request(
            &app,
            Method::POST,
            "/v3/marketplace/gns3/custom".into(),
            Some(serde_json::json!({
                "name": "Uploaded custom disk",
                "source_template_id": source_template_id
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
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
            Some(serde_json::json!({ "name": "Fake device", "node_type": "not-a-runtime" })),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("Unsupported node type")
        );
    }

    #[tokio::test]
    async fn builtin_templates_create_persistent_nodes_with_gns3_ports() {
        let (app, _directory) = test_router().await;
        let (_, project) = request(
            &app,
            Method::POST,
            "/v3/projects".into(),
            Some(serde_json::json!({ "name": "Builtin nodes" })),
        )
        .await;
        let project_id = project["project_id"].as_str().unwrap();

        let (status, templates) = request(&app, Method::GET, "/v3/templates".into(), None).await;
        assert_eq!(status, StatusCode::OK);
        let switch_template = templates
            .as_array()
            .unwrap()
            .iter()
            .find(|template| template["template_type"] == "ethernet_switch")
            .unwrap();
        assert_eq!(
            switch_template["ports_mapping"].as_array().unwrap().len(),
            8
        );
        let template_id = switch_template["template_id"].as_str().unwrap();

        let (status, switch) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/templates/{template_id}"),
            Some(serde_json::json!({
                "name": "SW1",
                "ports_mapping": [
                    {"type":"access", "vlan":10},
                    {"type":"access", "vlan":20}
                ]
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(switch["node_type"], "ethernet_switch");
        assert_eq!(switch["status"], "started");
        assert_eq!(switch["ports_mapping"][0]["name"], "Ethernet0");
        assert_eq!(switch["ports_mapping"][1]["port_number"], 1);
        assert_eq!(switch["ports_mapping"][1]["vlan"], 20);

        let (status, nodes) = request(
            &app,
            Method::GET,
            format!("/v3/projects/{project_id}/nodes"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(nodes.as_array().unwrap().len(), 1);
        assert_eq!(nodes[0]["node_id"], switch["node_id"]);
    }

    #[tokio::test]
    async fn builtin_http_lifecycle_matches_always_running_nodes() {
        let (app, _directory) = test_router().await;
        let (_, project) = request(
            &app,
            Method::POST,
            "/v3/projects".into(),
            Some(serde_json::json!({ "name": "Always on builtins" })),
        )
        .await;
        let project_id = project["project_id"].as_str().unwrap();
        let (status, switch) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes"),
            Some(serde_json::json!({ "name": "SW1", "node_type": "ethernet_switch" })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let switch_id = switch["node_id"].as_str().unwrap();
        for action in ["start", "stop", "suspend", "reload"] {
            let (status, _) = request(
                &app,
                Method::POST,
                format!("/v3/projects/{project_id}/nodes/{switch_id}/{action}"),
                None,
            )
            .await;
            assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "switch {action}");
        }

        let (status, cloud) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes"),
            Some(serde_json::json!({
                "name": "Cloud1",
                "node_type": "cloud",
                "ports_mapping": [{"type":"ethernet","interface":"virbr0"}]
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let cloud_id = cloud["node_id"].as_str().unwrap();
        let (status, started) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes/{cloud_id}/start"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(started["status"], "started");
        let (status, _) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes/{cloud_id}/stop"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn builtin_node_http_validation_rejects_bad_vlan_and_cloud_nio_values() {
        let (app, _directory) = test_router().await;
        let (_, project) = request(
            &app,
            Method::POST,
            "/v3/projects".into(),
            Some(serde_json::json!({ "name": "Builtin validation" })),
        )
        .await;
        let project_id = project["project_id"].as_str().unwrap();
        let (status, error) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes"),
            Some(serde_json::json!({
                "name":"Bad switch",
                "node_type":"ethernet_switch",
                "ports_mapping":[{"type":"access","vlan":4095}]
            })),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(error["message"].as_str().unwrap().contains("VLAN"));

        let (status, error) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes"),
            Some(serde_json::json!({
                "name":"Bad cloud",
                "node_type":"cloud",
                "ports_mapping":[{"type":"udp","lport":4001,"rhost":"127.0.0.1;false","rport":4002}]
            })),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(error["message"].as_str().unwrap().contains("remote host"));

        let (status, error) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes"),
            Some(serde_json::json!({
                "name":"Loopback cloud",
                "node_type":"cloud",
                "ports_mapping":[{"type":"ethernet","interface":"lo"}]
            })),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("allow_host_interfaces")
        );

        let (status, allowlisted_cloud) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes"),
            Some(serde_json::json!({
                "name":"Explicit loopback cloud",
                "node_type":"cloud",
                "ports_mapping":[{"type":"ethernet","interface":"lo"}],
                "allow_host_interfaces":["lo"]
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(
            allowlisted_cloud["allow_host_interfaces"],
            serde_json::json!(["lo"])
        );

        let (status, error) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes"),
            Some(serde_json::json!({
                "name":"Missing host interface",
                "node_type":"cloud",
                "ports_mapping":[{"type":"ethernet","interface":"nosuch0"}]
            })),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("does not exist")
        );

        let (status, error) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes"),
            Some(serde_json::json!({
                "name":"Unapproved NAT interface",
                "node_type":"nat",
                "ports_mapping":[{"type":"ethernet","interface":"lo"}]
            })),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            error["message"]
                .as_str()
                .unwrap()
                .contains("configured default")
        );

        let (status, allowlisted_nat) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes"),
            Some(serde_json::json!({
                "name":"Explicit interface NAT",
                "node_type":"nat",
                "ports_mapping":[{"type":"ethernet","interface":"lo"}],
                "allow_host_interfaces":["lo"]
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(
            allowlisted_nat["allow_host_interfaces"],
            serde_json::json!(["lo"])
        );
    }

    #[tokio::test]
    async fn builtin_configuration_uses_flat_fields_and_preserves_always_running_status() {
        let (app, _directory) = test_router().await;
        let (_, project) = request(
            &app,
            Method::POST,
            "/v3/projects".into(),
            Some(serde_json::json!({ "name": "Builtin configuration" })),
        )
        .await;
        let project_id = project["project_id"].as_str().unwrap();

        let (_, switch) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes"),
            Some(serde_json::json!({
                "name":"SW1","node_type":"ethernet_switch",
                "ports_mapping":[
                    {"type":"access","vlan":1},
                    {"type":"access","vlan":1}
                ]
            })),
        )
        .await;
        let switch_id = switch["node_id"].as_str().unwrap();
        let (status, configured_switch) = request(
            &app,
            Method::PUT,
            format!("/v3/projects/{project_id}/nodes/{switch_id}/config"),
            Some(serde_json::json!({
                "name":"SW1-updated",
                "ports_mapping":[
                    {"type":"access","vlan":10},
                    {"type":"dot1q","vlan":10}
                ],
                "console_type":"telnet"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(configured_switch["name"], "SW1-updated");
        assert_eq!(configured_switch["status"], "started");
        assert_eq!(configured_switch["ports_count"], 2);
        assert_eq!(configured_switch["ports_mapping"][1]["type"], "dot1q");

        let (_, hub) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes"),
            Some(serde_json::json!({"name":"Hub1","node_type":"ethernet_hub"})),
        )
        .await;
        let hub_id = hub["node_id"].as_str().unwrap();
        let (status, configured_hub) = request(
            &app,
            Method::PUT,
            format!("/v3/projects/{project_id}/nodes/{hub_id}/config"),
            Some(serde_json::json!({
                "properties": {
                    "ports_mapping": [
                        {"name":"old","port_number":9},
                        {"name":"old2","port_number":9}
                    ]
                }
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(configured_hub["status"], "started");
        assert_eq!(configured_hub["ports_mapping"].as_array().unwrap().len(), 2);
        assert!(configured_hub["properties"]["ports_mapping"].is_null());

        let (_, cloud) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes"),
            Some(serde_json::json!({
                "name":"Cloud1","node_type":"cloud",
                "ports_mapping":[{"type":"udp","lport":4100,"rhost":"127.0.0.1","rport":4101}]
            })),
        )
        .await;
        let cloud_id = cloud["node_id"].as_str().unwrap();
        let (status, configured_cloud) = request(
            &app,
            Method::PUT,
            format!("/v3/projects/{project_id}/nodes/{cloud_id}/config"),
            Some(serde_json::json!({
                "remote_console_host":"192.0.2.5",
                "remote_console_port":2323,
                "remote_console_type":"telnet",
                "remote_console_http_path":"/console"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(configured_cloud["status"], "started");
        assert_eq!(configured_cloud["remote_console_host"], "192.0.2.5");
        assert_eq!(configured_cloud["remote_console_port"], 2323);

        let (_, nat) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes"),
            Some(serde_json::json!({"name":"NAT1","node_type":"nat"})),
        )
        .await;
        let nat_id = nat["node_id"].as_str().unwrap();
        let (status, configured_nat) = request(
            &app,
            Method::PUT,
            format!("/v3/projects/{project_id}/nodes/{nat_id}/config"),
            Some(serde_json::json!({"name":"NAT1-updated"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(configured_nat["name"], "NAT1-updated");
        assert_eq!(configured_nat["status"], "started");
        assert_eq!(configured_nat["ports_mapping"][0]["interface"], "virbr0");
    }

    #[test]
    fn validates_only_runtime_settings_supported_by_each_runtime() {
        let qemu = validate_node_configuration_patch(
            "qemu",
            serde_json::json!({
                "memory": 2048,
                "cpus": 4,
                "adapters": 3,
                "adapter_type": "virtio-net-pci",
                "hda_disk_interface": "scsi",
                "boot_priority": "dc"
            }),
        )
        .unwrap_or_else(|_| panic!("QEMU's implemented settings should be accepted"));
        assert_eq!(qemu["memory"], 2048);
        assert!(
            validate_node_configuration_patch(
                "qemu",
                serde_json::json!({ "adapter_type": "unsupported-nic" })
            )
            .is_err()
        );

        let docker = validate_node_configuration_patch(
            "docker",
            serde_json::json!({
                "memory": 0,
                "cpus": 0.5,
                "adapters": 4,
                "environment": "LAB_MODE=router",
                "extra_hosts": "gateway:192.0.2.1",
                "extra_volumes": ["/etc/lab"],
                "console_type": "none"
            }),
        )
        .unwrap_or_else(|_| panic!("Docker's implemented settings should be accepted"));
        assert_eq!(docker["cpus"], 0.5);
        assert_eq!(docker["console_type"], "none");
        assert!(
            validate_node_configuration_patch(
                "docker",
                serde_json::json!({ "runtime_overlay": "/tmp/image.qcow2" })
            )
            .is_err()
        );

        let vpcs = validate_node_configuration_patch(
            "vpcs",
            serde_json::json!({
                "startup_script": "ip 192.0.2.2/24",
                "console_type": "ssh"
            }),
        )
        .unwrap_or_else(|_| {
            panic!("VPCS startup scripts and GNS3 console types should be configurable")
        });
        assert_eq!(vpcs["startup_script"], "ip 192.0.2.2/24");
        assert_eq!(vpcs["console_type"], "ssh");
        for console_type in ["telnet", "ssh", "none"] {
            assert!(
                validate_node_configuration_patch(
                    "vpcs",
                    serde_json::json!({"console_type": console_type})
                )
                .is_ok()
            );
        }
        assert!(
            validate_node_configuration_patch("vpcs", serde_json::json!({"console_type": "vnc"}))
                .is_err()
        );
        let dynamips = validate_node_configuration_patch(
            "dynamips",
            serde_json::json!({ "startup_config": "hostname R1\n" }),
        )
        .unwrap_or_else(|_| panic!("Dynamips startup config should be configurable"));
        assert_eq!(dynamips["startup_config"], "hostname R1\n");
        assert!(
            validate_node_configuration_patch("dynamips", serde_json::json!({ "ram": 512 }))
                .is_err()
        );
    }

    #[tokio::test]
    async fn vpcs_suspend_matches_gns3_method_not_allowed_behavior() {
        let directory = TempDir::new().expect("temporary VPCS suspend test directory");
        let store = Store::connect(
            directory
                .path()
                .join("vpcs-suspend.sqlite")
                .to_str()
                .expect("UTF-8 database path"),
        )
        .await
        .expect("test database connects");
        let project_id = uuid::Uuid::new_v4();
        let node_id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO projects(project_id,name,status,project_json) VALUES(?,?,?,?)")
            .bind(project_id.to_string())
            .bind("VPCS suspend")
            .bind("opened")
            .bind(serde_json::json!({"name":"VPCS suspend"}).to_string())
            .execute(store.pool())
            .await
            .unwrap();
        let node = serde_json::json!({
            "node_id": node_id,
            "project_id": project_id,
            "node_type": "vpcs",
            "name": "PC1",
            "status": "started"
        });
        sqlx::query("INSERT INTO topology_objects(project_id,kind,object_id,name,object_json) VALUES(?,?,?,?,?)")
            .bind(project_id.to_string())
            .bind("node")
            .bind(node_id.to_string())
            .bind("PC1")
            .bind(node.to_string())
            .execute(store.pool())
            .await
            .unwrap();

        let state = AppState::new(store);
        let error = match suspend_node(State(state), Path((project_id, node_id))).await {
            Ok(_) => panic!("VPCS suspend should be rejected"),
            Err(error) => error,
        };
        assert_eq!(error.status, StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn forwarded_proxy_headers_keep_bearer_authentication_enforced() {
        let directory = TempDir::new().expect("temporary proxy-auth test directory");
        let store = Store::connect_unseeded(
            directory
                .path()
                .join("proxy-auth.sqlite")
                .to_str()
                .expect("UTF-8 database path"),
        )
        .await
        .expect("test database connects without auth bootstrap");
        let initial_password_path = directory.path().join("initial-password.txt");
        auth::initialize(store.pool(), true, &initial_password_path)
            .await
            .expect("auth bootstrap completes");
        let password = std::fs::read_to_string(initial_password_path)
            .expect("initial password file is readable");
        let app = router(AppState::new(store));

        let login = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v3/access/users/authenticate")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "username":"admin",
                            "password":password.trim()
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(login.status(), StatusCode::OK);
        let login_body: Value =
            serde_json::from_slice(&login.into_body().collect().await.unwrap().to_bytes()).unwrap();
        let token = login_body["access_token"].as_str().unwrap();

        let without_token = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v3/access/users/me")
                    .header("x-forwarded-for", "198.51.100.27")
                    .header("x-forwarded-proto", "https")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(without_token.status(), StatusCode::UNAUTHORIZED);

        let with_token = app
            .oneshot(
                Request::builder()
                    .uri("/v3/access/users/me")
                    .header("x-forwarded-for", "198.51.100.27")
                    .header("x-forwarded-proto", "https")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(with_token.status(), StatusCode::OK);
    }

    #[test]
    fn dynamips_template_creation_replaces_null_runtime_id_before_deserialization() {
        let mut template = serde_json::json!({
            "dynamips_id": null,
            "platform": "c3725",
            "slot0": "GT96100-FE",
            "slot1": null,
            "wic0": null,
            "mac_addr": "",
            "system_id": "FTX0945W0MY",
            "iomem": null
        })
        .as_object()
        .expect("template JSON is an object")
        .clone();

        let options = dynamips_options_from_template(&mut template, 7).unwrap_or_else(|_| {
            panic!("allocated runtime ID should make the template settings valid")
        });

        assert_eq!(options.dynamips_id, 7);
        assert_eq!(template["dynamips_id"], 7);
        assert_eq!(options.slots[0].as_deref(), Some("GT96100-FE"));
        assert_eq!(options.wics[0], None);
        assert_eq!(options.mac_addr, None);
        assert_eq!(options.iomem, None);
    }

    #[test]
    fn parses_docker_console_resize_control_frames() {
        assert_eq!(
            parse_docker_resize_control(r#"{"type":"resize","rows":40,"columns":120}"#),
            Some((40, 120))
        );
        assert_eq!(
            parse_docker_resize_control(r#"{"type":"resize","rows":0,"columns":120}"#),
            None
        );
        assert_eq!(
            parse_docker_resize_control(r#"{"type":"input","rows":40,"columns":120}"#),
            None
        );
        assert_eq!(parse_docker_resize_control("show version"), None);
    }

    #[test]
    fn capture_paths_are_canonicalized_inside_the_project_directory() {
        let directory = TempDir::new().expect("temporary capture root");
        let project_dir = directory.path().join("project");
        std::fs::create_dir_all(&project_dir).expect("create project capture directory");
        let capture = project_dir.join("traffic.pcap");
        std::fs::write(&capture, b"pcap").expect("write capture");
        let outside = directory.path().join("outside.pcap");
        std::fs::write(&outside, b"private").expect("write outside file");
        assert_eq!(
            canonical_capture_file_path_in(
                &project_dir,
                "traffic.pcap",
                Some(capture.to_str().unwrap()),
                true,
            )
            .unwrap(),
            capture.canonicalize().unwrap()
        );
        assert!(
            canonical_capture_file_path_in(
                &project_dir,
                "traffic.pcap",
                Some(outside.to_str().unwrap()),
                true,
            )
            .is_err()
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, project_dir.join("linked.pcap"))
                .expect("create capture symlink");
            assert!(
                canonical_capture_file_path_in(
                    &project_dir,
                    "linked.pcap",
                    Some(project_dir.join("linked.pcap").to_str().unwrap()),
                    true,
                )
                .is_err()
            );
        }
        assert!(safe_capture_file_name("x\nhypervisor stop\n").is_err());
    }

    #[test]
    fn frequency_drop_is_normalized_and_filters_select_one_relay_endpoint() {
        let filters = validate_link_filters(&serde_json::json!({ "frequency_drop": [3] }))
            .expect("frequency_drop is supported");
        assert_eq!(filters["frequency_drop"], serde_json::json!([3]));
        assert!(validate_link_filters(&serde_json::json!({ "bpf": ["ip\ntcp"] })).is_err());

        let link = serde_json::json!({
            "filters": { "packet_loss": [10] },
            "nodes": [
                { "runtime_endpoint_id": "first" },
                { "runtime_endpoint_id": "second" }
            ]
        });
        assert_eq!(
            link_filters_for_endpoint(&link, &link["nodes"][0]),
            serde_json::json!({ "packet_loss": [10] })
        );
        assert_eq!(
            link_filters_for_endpoint(&link, &link["nodes"][1]),
            serde_json::json!({})
        );
    }

    #[tokio::test]
    async fn failed_relay_attach_runs_runtime_nio_cleanup() {
        let attach_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let cleanup_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let cleanup = cleanup_count.clone();
        let result = attach_runtime_nio_with_rollback(
            true,
            || {
                let attach_count = attach_count.clone();
                async move {
                    attach_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                }
            },
            || async { Err(ApiError::conflict("uBridge attach failed")) },
            || async move {
                cleanup.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            },
        )
        .await;

        assert!(result.is_err());
        assert_eq!(attach_count.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(cleanup_count.load(std::sync::atomic::Ordering::SeqCst), 1);

        let cleanup = cleanup_count.clone();
        let result = attach_runtime_nio_with_rollback(
            false,
            || async { Ok(()) },
            || async { Err(ApiError::conflict("uBridge update failed")) },
            || async move {
                cleanup.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(cleanup_count.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}

fn internal_json_error(error: serde_json::Error) -> ApiError {
    tracing::error!(%error, "stored JSON could not be decoded");
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "Stored data is invalid")
}
