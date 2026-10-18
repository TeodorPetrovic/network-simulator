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
    let state = api::AppState::new(store);
    state.reattach_running_links().await;
    let app = api::router(state);
    let listener = tokio::net::TcpListener::bind(bind_address).await?;

    tracing::info!(
        %bind_address,
        %database_path,
        qemu_images = %image_paths::qemu_images().display(),
        dynamips_images = %image_paths::dynamips_images().display(),
        appliance_definitions = %image_paths::appliance_definitions().display(),
        "network simulator controller listening"
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to listen for shutdown signal");
    }
}
