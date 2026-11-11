//! Pop-out windows. Browsers' `window.open` is not available in a desktop webview, so the UI asks
//! the shell to create real native windows (consoles, captures, Web Wireshark) instead.

use std::sync::atomic::{AtomicU32, Ordering};

use tauri::{AppHandle, WebviewUrl, WebviewWindowBuilder};

static NEXT: AtomicU32 = AtomicU32::new(1);

#[tauri::command]
pub async fn open_window(app: AppHandle, url: String, title: String, width: f64, height: f64) -> Result<String, String> {
    // Only in-app routes: external links go through the system browser, never a window with IPC access.
    if !url.starts_with('/') || url.starts_with("//") { return Err("only in-app routes can be opened in a window".into()); }
    let label = format!("popout-{}", NEXT.fetch_add(1, Ordering::Relaxed));
    WebviewWindowBuilder::new(&app, &label, WebviewUrl::App(url.trim_start_matches('/').into()))
        .title(title).inner_size(width.clamp(320.0, 3000.0), height.clamp(240.0, 2000.0)).build()
        .map_err(|e| e.to_string())?;
    Ok(label)
}
