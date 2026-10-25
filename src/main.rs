#![recursion_limit = "256"]

mod api;
mod builtin;
mod dynamips;
mod image_paths;
mod links;
mod marketplace;
mod qemu;
mod runtime;
mod storage;
mod ubridge;
mod vpcs;
mod web_wireshark;

use std::{env, net::SocketAddr, path::Path};

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let database_path = env::var("NETWORK_SIMULATOR_DB")
        .unwrap_or_else(|_| "./data/network-simulator.sqlite".to_owned());
    if let Some(parent) = Path::new(&database_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }

    image_paths::create_default_directories()?;

    let bind_address: SocketAddr = env::var("NETWORK_SIMULATOR_BIND")
        .unwrap_or_else(|_| "127.0.0.1:3080".to_owned())
        .parse()?;
    let store = storage::Store::connect(&database_path).await?;
    let auth_override = env::var("NETWORK_SIMULATOR_AUTH_ENABLED")
        .ok()
        .and_then(|value| api::auth::parse_bool(&value));
    let auth_enabled = auth_override.unwrap_or(!bind_address.ip().is_loopback());
    let data_dir = Path::new(&database_path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let initial_password_path = data_dir.join("initial-admin-password.txt");
    api::auth::initialize(store.pool(), auth_enabled, &initial_password_path).await?;
    if let Some(enabled) = auth_override {
        api::auth::set_auth_enabled(store.pool(), enabled).await?;
    }
    let auth_enabled =
        api::auth::enforce_bind_auth_floor(store.pool(), !bind_address.ip().is_loopback()).await?;
    let state = api::AppState::new(store);
    state.reap_orphaned_runtimes();
    state.reconcile_node_statuses().await;
    state.open_auto_open_projects().await;
    state.reattach_running_links().await;
    let monitor_state = state.clone();
    tokio::spawn(async move {
        monitor_state.monitor_node_liveness().await;
    });
    let listener = tokio::net::TcpListener::bind(bind_address).await?;
    let bind_address = listener.local_addr()?;
    let state = state.with_bind_address(bind_address);
    let shutdown_state = state.clone();
    let app = api::router(state);

    tracing::info!(
        %bind_address,
        auth_enabled,
        %database_path,
        qemu_images = %image_paths::qemu_images().display(),
        dynamips_images = %image_paths::dynamips_images().display(),
        appliance_definitions = %image_paths::appliance_definitions().display(),
        "network simulator controller listening"
    );
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    shutdown_state.shutdown_runtimes().await;

    Ok(())
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to listen for shutdown signal");
    }
}
