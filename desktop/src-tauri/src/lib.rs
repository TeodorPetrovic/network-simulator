//! Network Studio desktop shell.
//!
//! The simulator's runtime (QEMU, Dynamips, Docker, uBridge, …) never runs inside this app: the UI
//! talks to a controller on a server the user picks on first launch (`/connect` in the web UI).
//! This crate only hosts the UI in a native window and exposes a few host facts to it.

use serde::Serialize;

mod tools;
mod windows;

#[derive(Serialize)]
struct HostInfo {
    app_version: &'static str,
    os: &'static str,
    arch: &'static str,
}

/// Lets the UI show which desktop build is talking to the server (useful in bug reports).
#[tauri::command]
fn host_info() -> HostInfo {
    HostInfo {
        app_version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .runtime(tauri_runtime_wry::Wry::default())
        .invoke_handler(tauri::generate_handler![host_info, tools::tool_status, tools::install_tool, tools::launch_wireshark, tools::open_in_wireshark, tools::launch_motty, windows::open_window])
        .run(tauri::generate_context!())
        .expect("error while running Network Studio");
}
