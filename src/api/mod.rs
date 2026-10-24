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
        .route("/healthz", get(system::health))
        .route(
            "/v3/version",
            get(system::version).post(controller_routes::check_version),
        )
        .route("/v3/runtimes", get(system::runtimes))
        .route("/v3/resources/monitor", get(system::resource_monitor))
        .route(
            "/v3/docker/images/pull",
            post(runtime_templates::pull_docker_image),
        )
        .route(
            "/v3/marketplace/ishare2/images",
            get(marketplace_api::search_ishare2_images),
        )
        .route(
            "/v3/marketplace/ishare2/images/{image_type}/{image_id}/download",
            post(marketplace_api::download_ishare2_image),
        )
        .route(
            "/v3/marketplace/gns3/appliances",
            get(marketplace_api::search_gns3_appliances),
        )
        .route(
            "/v3/marketplace/gns3/appliances/{file_name}",
            get(marketplace_api::get_gns3_appliance),
        )
        .route(
            "/v3/marketplace/gns3/appliances/{file_name}/import",
            post(marketplace_api::import_gns3_appliance),
        )
        .route(
            "/v3/marketplace/gns3/custom",
            post(marketplace_api::create_custom_appliance),
        )
        .route(
            "/v3/marketplace/gns3/imported",
            get(marketplace_api::list_imported_appliances),
        )
        .route(
            "/v3/marketplace/gns3/manager",
            get(marketplace_api::get_appliance_manager),
        )
        .route(
            "/v3/marketplace/gns3/appliances/{file_name}/bindings",
            put(marketplace_api::set_appliance_image_binding)
                .delete(marketplace_api::clear_appliance_image_binding),
        )
        .route(
            "/v3/marketplace/gns3/import/{file_name}",
            post(marketplace_api::import_appliance_file),
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
            get(topology_listing::list_nodes).post(node_templates::create_node),
        )
        .route(
            "/v3/projects/{project_id}/templates/{template_id}",
            post(node_templates::create_node_from_template),
        )
        .route(
            "/v3/projects/{project_id}/nodes/{node_id}",
            get(node_handlers::get_node)
                .put(node_handlers::update_node)
                .delete(node_lifecycle::delete_node),
        )
        .route(
            "/v3/projects/{project_id}/nodes/{node_id}/config",
            put(node_handlers::update_node_configuration),
        )
        .route(
            "/v3/projects/{project_id}/nodes/{node_id}/start",
            post(node_lifecycle::start_node),
        )
        .route(
            "/v3/projects/{project_id}/nodes/{node_id}/stop",
            post(node_lifecycle::stop_node),
        )
        .route(
            "/v3/projects/{project_id}/nodes/{node_id}/suspend",
            post(node_lifecycle::suspend_node),
        )
        .route(
            "/v3/projects/{project_id}/nodes/{node_id}/reload",
            post(node_lifecycle::reload_node),
        )
        .route(
            "/v3/projects/{project_id}/links",
            get(topology_listing::list_links).post(link_create::create_link),
        )
        .route(
            "/v3/projects/{project_id}/links/{link_id}",
            get(link_details::get_link)
                .put(link_details::update_link)
                .delete(link_capture::delete_link),
        )
        .route(
            "/v3/projects/{project_id}/links/{link_id}/available_filters",
            get(link_details::available_link_filters),
        )
        .route(
            "/v3/projects/{project_id}/links/{link_id}/capture/start",
            post(link_capture::start_link_capture),
        )
        .route(
            "/v3/projects/{project_id}/links/{link_id}/capture/stop",
            post(link_capture::stop_link_capture),
        )
        .route(
            "/v3/projects/{project_id}/links/{link_id}/capture/stream",
            get(link_capture::stream_link_capture),
        )
        .route(
            "/v3/projects/{project_id}/links/{link_id}/capture/file",
            get(link_capture::download_link_capture),
        )
        .route(
            "/v3/projects/{project_id}/drawings",
            get(topology_listing::list_drawings).post(topology::create_drawing),
        )
        .route(
            "/v3/projects/{project_id}/drawings/{drawing_id}",
            get(topology::get_drawing)
                .put(topology::update_drawing)
                .delete(topology::delete_drawing),
        )
        .route(
            "/v3/projects/{project_id}/topology",
            get(topology::get_topology_snapshot),
        )
        .route(
            "/v3/projects/{project_id}/notifications/ws",
            get(notifications::project_notifications),
        )
        .route(
            "/v3/compute/projects/{project_id}/qemu/nodes/{node_id}/console/ws",
            get(consoles::qemu_console),
        )
        .route(
            "/v3/compute/projects/{project_id}/docker/nodes/{node_id}/console/ws",
            get(consoles::docker_console),
        )
        .route(
            "/v3/compute/projects/{project_id}/docker/nodes/{node_id}/aux_console/ws",
            get(consoles::docker_aux_console),
        )
        .route(
            "/v3/compute/projects/{project_id}/vpcs/nodes/{node_id}/console/ws",
            get(consoles::vpcs_console),
        )
        .route(
            "/v3/compute/projects/{project_id}/dynamips/nodes/{node_id}/console/ws",
            get(consoles::dynamips_console),
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
            post(marketplace_api::upload_dynamips_image),
        )
        .layer(RequestBodyLimitLayer::new(MAX_DYNAMIPS_IMAGE_BYTES));
    let qemu_image_upload = Router::new()
        .route(
            "/v3/qemu/images/{file_name}",
            post(marketplace_api::upload_qemu_image),
        )
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

mod system;

mod consoles;
mod docker_node;
mod dynamips_node;
mod link_capture;
mod link_create;
mod link_details;
mod link_runtime;
mod marketplace_api;
mod node_configuration;
mod node_handlers;
mod node_lifecycle;
mod node_runtime;
mod node_templates;
mod notifications;
mod objects;
mod project_store;
mod projects;
mod qemu_node;
mod runtime_templates;
mod template_profiles;
mod templates;
mod topology;
mod topology_listing;
mod vpcs_node;
use consoles::parse_docker_resize_control;
use dynamips_node::dynamips_options_from_template;
use link_capture::{
    canonical_capture_file_path, canonical_capture_file_path_in, safe_capture_file_name,
};
use link_details::validate_link_filters;
use link_runtime::{
    attach_node_links, attach_runtime_link_endpoint, attach_runtime_nio_with_rollback,
    detach_node_links, detach_runtime_link_endpoint, link_filters_for_endpoint,
    link_runtime_endpoints, load_topology_node, node_link_entries, qemu_link_from_endpoint,
    runtime_endpoint_status,
};
use node_configuration::{
    persist_node_configuration, validate_adapter_count_against_links,
    validate_builtin_configuration_links, validate_node_configuration_patch,
};
use node_lifecycle::{change_node_runtime_state, suspend_node};
use node_runtime::{
    apply_node_lifecycle, cleanup_link_runtime, cleanup_node_runtime, docker_interface_mac,
    stop_runtime_node, vpcs_startup_script,
};
use node_templates::create_node;
use objects::{list_objects, list_objects_in_tx, refresh_node_statuses};
use project_store::{ensure_project_exists, ensure_project_exists_pool, fetch_project};
use runtime_templates::{
    base_runtime_templates, runtime_api_error, valid_container_path,
    validate_docker_extra_configs_api,
};
use template_profiles::validate_template_profile_patch;
use templates::{available_templates, reset_template_profile, update_template_profile};
use topology::{
    create_topology_object, delete_node_and_connected_links, delete_topology_object,
    get_topology_object, update_topology_object,
};
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
