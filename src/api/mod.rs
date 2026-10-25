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
#[path = "controller_routes/mod.rs"]
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

pub(super) fn etag_for(value: &Value) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, value.to_string().as_bytes());
    let hex = digest.as_ref()[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("\"{hex}\"")
}

pub(super) fn check_if_match(
    headers: &axum::http::HeaderMap,
    value: &Value,
) -> Result<(), ApiError> {
    if let Some(expected) = headers.get(axum::http::header::IF_MATCH) {
        let expected = expected
            .to_str()
            .map_err(|_| ApiError::bad_request("If-Match must be a valid ETag"))?;
        let expected = expected.split(',').map(str::trim).collect::<Vec<_>>();
        if !expected.contains(&"*") && !expected.contains(&etag_for(value).as_str()) {
            return Err(ApiError::new(
                StatusCode::PRECONDITION_FAILED,
                "ETag does not match current resource",
            ));
        }
    }
    Ok(())
}

pub(super) fn etag_response(value: Value) -> Response {
    let etag = etag_for(&value);
    let mut response = Json(value).into_response();
    response.headers_mut().insert(
        axum::http::header::ETAG,
        axum::http::HeaderValue::from_str(&etag).expect("ETag is valid header value"),
    );
    response
}

#[cfg(test)]
mod etag_tests {
    use super::*;

    #[test]
    fn if_match_is_optional_and_rejects_stale_tags() {
        let value = json!({"name":"project"});
        assert!(check_if_match(&axum::http::HeaderMap::new(), &value).is_ok());
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(axum::http::header::IF_MATCH, "\"stale\"".parse().unwrap());
        assert_eq!(
            check_if_match(&headers, &value).unwrap_err().status,
            StatusCode::PRECONDITION_FAILED
        );
        headers.insert(
            axum::http::header::IF_MATCH,
            etag_for(&value).parse().unwrap(),
        );
        assert!(check_if_match(&headers, &value).is_ok());
        headers.insert(axum::http::header::IF_MATCH, "*".parse().unwrap());
        assert!(check_if_match(&headers, &value).is_ok());
    }
}

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
    auto_close_delay: Duration,
    web_wireshark: Arc<crate::web_wireshark::WebWiresharkManager>,
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
            "/v3/projects/{project_id}/nodes/batch",
            post(batch_create::create_nodes),
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
            "/v3/projects/{project_id}/nodes/{node_id}/pause",
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
            "/v3/projects/{project_id}/links/batch",
            post(batch_create::create_links),
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
            "/v3/projects/{project_id}/links/{link_id}/capture/wireshark/restart",
            post(link_capture::restart_web_wireshark),
        )
        .route(
            "/v3/projects/{project_id}/links/{link_id}/capture/web-wireshark",
            get(link_capture::web_wireshark_websocket),
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

mod errors;
use errors::{ApiError, internal_json_error};

mod activity;
mod state;
mod system;
use activity::{
    ProjectActivityTracker, configured_idle_stop_minutes, project_id_from_path,
    track_project_activity,
};

mod batch_create;
mod consoles;
mod docker_node;
mod dynamips_node;
pub(crate) mod link_capture;
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
mod template_profile_fields;
mod template_profiles;
use template_profile_fields::template_profile_fields;
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
            return Err(ApiError::port_in_use(format!(
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

mod json_helpers;
use json_helpers::{
    as_object_mut, decode_json, parse_or_generate_uuid, required_string, set_default,
};

#[cfg(test)]
#[path = "tests/project_activity_tests.rs"]
mod project_activity_tests;
#[cfg(test)]
#[path = "tests/project_lifecycle.rs"]
mod project_lifecycle;

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;
