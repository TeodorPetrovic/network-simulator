//! Cross-origin access for the desktop client.
//!
//! The Tauri desktop app serves its UI from its own origin and talks to a remote controller, so
//! the browser engine sends CORS preflights. Authentication uses bearer tokens (never cookies),
//! therefore credentials are not allowed and only an explicit origin allow-list is honoured.
//! Same-origin browser use is unaffected.

use axum::http::{HeaderValue, Method, header};
use tower_http::cors::{AllowOrigin, CorsLayer};

/// Origins the Tauri webview reports on Linux/macOS (`tauri://localhost`) and Windows/Android
/// (`http(s)://tauri.localhost`).
const DESKTOP_ORIGINS: [&str; 3] = [
    "tauri://localhost",
    "http://tauri.localhost",
    "https://tauri.localhost",
];

/// `NETWORK_SIMULATOR_CORS_ORIGINS` is a comma-separated list of additional exact origins
/// (for example a Vite dev server). `*` is deliberately not supported.
pub(super) fn layer() -> CorsLayer {
    layer_with(std::env::var("NETWORK_SIMULATOR_CORS_ORIGINS").ok().as_deref())
}

fn layer_with(extra: Option<&str>) -> CorsLayer {
    let mut allowed: Vec<HeaderValue> = DESKTOP_ORIGINS
        .iter()
        .map(|origin| HeaderValue::from_static(origin))
        .collect();
    for origin in extra.unwrap_or_default().split(',') {
        let origin = origin.trim().trim_end_matches('/');
        if origin.is_empty() || origin == "*" {
            continue;
        }
        if let Ok(value) = HeaderValue::from_str(origin) {
            allowed.push(value);
        }
    }
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(allowed))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::ACCEPT,
        ])
        .expose_headers([header::CONTENT_DISPOSITION, header::CONTENT_LENGTH])
        .max_age(std::time::Duration::from_secs(600))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, http::Request, routing::get};
    use tower::ServiceExt;

    fn app(extra: Option<&str>) -> Router {
        Router::new()
            .route("/x", get(|| async { "ok" }))
            .layer(layer_with(extra))
    }

    async fn preflight(app: Router, origin: &str) -> axum::response::Response {
        app.oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/x")
                .header("origin", origin)
                .header("access-control-request-method", "GET")
                .header("access-control-request-headers", "authorization")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn desktop_origins_are_allowed() {
        for origin in DESKTOP_ORIGINS {
            let response = preflight(app(None), origin).await;
            assert_eq!(
                response.headers().get("access-control-allow-origin").unwrap(),
                origin
            );
        }
    }

    #[tokio::test]
    async fn unknown_origins_are_rejected_unless_configured() {
        let response = preflight(app(None), "https://evil.example").await;
        assert!(response.headers().get("access-control-allow-origin").is_none());
        let response = preflight(app(Some("http://127.0.0.1:5173/, *")), "http://127.0.0.1:5173").await;
        assert_eq!(
            response.headers().get("access-control-allow-origin").unwrap(),
            "http://127.0.0.1:5173"
        );
        let response = preflight(app(Some("*")), "https://evil.example").await;
        assert!(response.headers().get("access-control-allow-origin").is_none());
    }
}
