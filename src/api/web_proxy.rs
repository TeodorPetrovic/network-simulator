//! Same-port reverse proxy for Docker HTTP consoles.
//!
//! Remote browsers only need to reach the simulator's own port: an authenticated
//! `POST .../web_session` returns an unguessable `/proxy/{token}/…` URL, and every
//! HTTP or WebSocket request below it is bridged into the container with `docker exec … nc`.
use super::*;
use axum::extract::Request;
use http_body_util::BodyExt;
use hyper_util::rt::TokioIo;
use std::time::Duration;

const SESSION_TTL: Duration = Duration::from_secs(12 * 60 * 60);
const MAX_SESSIONS: usize = 512;
const MAX_REWRITE_BYTES: usize = 16 * 1024 * 1024;

struct WebSession {
    project_id: String,
    node_id: String,
    expires: Instant,
}

#[derive(Clone, Default)]
pub(crate) struct WebSessions(Arc<StdMutex<HashMap<String, WebSession>>>);

impl WebSessions {
    fn create(&self, project_id: String, node_id: String) -> String {
        let mut bytes = [0u8; 32];
        let _ = ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes);
        let token: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        let mut sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        sessions.retain(|_, session| session.expires > now);
        if sessions.len() >= MAX_SESSIONS
            && let Some(oldest) = sessions
                .iter()
                .min_by_key(|(_, session)| session.expires)
                .map(|(key, _)| key.clone())
        {
            sessions.remove(&oldest);
        }
        sessions.insert(
            token.clone(),
            WebSession { project_id, node_id, expires: now + SESSION_TTL },
        );
        token
    }

    fn lookup(&self, token: &str) -> Option<(String, String)> {
        let sessions = self.0.lock().unwrap_or_else(|e| e.into_inner());
        sessions
            .get(token)
            .filter(|session| session.expires > Instant::now())
            .map(|session| (session.project_id.clone(), session.node_id.clone()))
    }
}

pub(crate) async fn create_web_session(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    let (project_id, node_id) = (project_id.to_string(), node_id.to_string());
    let node = get_topology_object(state.store.pool(), &project_id, "node", &node_id)
        .await?
        .0;
    if node["node_type"] != "docker" || !matches!(node["console_type"].as_str(), Some("http" | "https")) {
        return Err(ApiError::conflict(
            "A web proxy session is available only for Docker nodes with an HTTP or HTTPS console",
        ));
    }
    let path = node["console_http_path"].as_str().filter(|p| p.starts_with('/')).unwrap_or("/");
    let token = state.web_sessions.create(project_id, node_id);
    Ok(Json(json!({ "url": format!("/proxy/{token}{path}"), "expires_in": SESSION_TTL.as_secs() })))
}

pub(crate) async fn proxy_root(
    State(state): State<AppState>,
    Path(token): Path<String>,
    request: Request,
) -> Response {
    proxy(state, token, String::new(), request).await
}

pub(crate) async fn proxy_path(
    State(state): State<AppState>,
    Path((token, rest)): Path<(String, String)>,
    request: Request,
) -> Response {
    proxy(state, token, rest, request).await
}

fn plain(status: StatusCode, message: &str) -> Response {
    (status, message.to_owned()).into_response()
}

async fn proxy(state: AppState, token: String, rest: String, mut request: Request) -> Response {
    let Some((project_id, node_id)) = state.web_sessions.lookup(&token) else {
        return plain(StatusCode::UNAUTHORIZED, "This web console link has expired. Open the console again.");
    };
    if rest.is_empty() && request.uri().path().ends_with(&token) {
        let query = request.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
        return (StatusCode::TEMPORARY_REDIRECT, [(header::LOCATION, format!("/proxy/{token}/{query}"))]).into_response();
    }
    let node = match get_topology_object(state.store.pool(), &project_id, "node", &node_id).await {
        Ok(node) => node.0,
        Err(_) => return plain(StatusCode::NOT_FOUND, "Node no longer exists."),
    };
    let Some(container_id) = node["container_id"].as_str().map(str::to_owned) else {
        return plain(StatusCode::CONFLICT, "Node has no runtime container.");
    };
    match state.runtime.container_status(&container_id).await {
        Ok(status) if status == "started" => {}
        _ => return plain(StatusCode::CONFLICT, "Node is not started."),
    }
    let target_port = node["console_http_port"].as_u64().and_then(|p| u16::try_from(p).ok()).unwrap_or(80);
    let prefix = format!("/proxy/{token}");
    let stream = match state.runtime.open_container_stream(&container_id, target_port) {
        Ok(stream) => stream,
        Err(error) => return runtime_api_error(error).into_response(),
    };
    let (mut sender, connection) = match hyper::client::conn::http1::handshake(TokioIo::new(stream)).await {
        Ok(parts) => parts,
        Err(error) => return plain(StatusCode::BAD_GATEWAY, &format!("Could not reach the container web server: {error}")),
    };
    tokio::spawn(async move {
        let _ = connection.with_upgrades().await;
    });

    let upgrade_requested = request
        .headers()
        .get(header::CONNECTION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.to_ascii_lowercase().contains("upgrade"));
    let client_upgrade = upgrade_requested.then(|| hyper::upgrade::on(&mut request));

    let (parts, body) = request.into_parts();
    let upstream_path = format!("/{rest}");
    let upstream_uri = match parts.uri.query() {
        Some(query) => format!("{upstream_path}?{query}"),
        None => upstream_path,
    };
    let upstream_host = format!("127.0.0.1:{target_port}");
    let mut upstream = hyper::Request::builder().method(parts.method).uri(upstream_uri);
    for (name, value) in &parts.headers {
        if matches!(name.as_str(), "host" | "accept-encoding" | "origin" | "referer")
            || name.as_str().starts_with("x-forwarded-")
        {
            continue;
        }
        upstream = upstream.header(name, value);
    }
    upstream = upstream.header(header::HOST, upstream_host.as_str());
    if parts.headers.contains_key(header::ORIGIN) {
        upstream = upstream.header(header::ORIGIN, format!("http://{upstream_host}"));
    }
    let Ok(upstream) = upstream.body(body) else {
        return plain(StatusCode::BAD_REQUEST, "Invalid proxied request.");
    };
    let mut response = match sender.send_request(upstream).await {
        Ok(response) => response,
        Err(error) => return plain(StatusCode::BAD_GATEWAY, &format!("The container web server did not answer: {error}")),
    };

    if response.status() == StatusCode::SWITCHING_PROTOCOLS
        && let Some(client_upgrade) = client_upgrade
    {
        let upstream_upgrade = hyper::upgrade::on(&mut response);
        tokio::spawn(async move {
            if let (Ok(client), Ok(server)) = (client_upgrade.await, upstream_upgrade.await) {
                let (mut client, mut server) = (TokioIo::new(client), TokioIo::new(server));
                let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
            }
        });
        let (parts, _) = response.into_parts();
        return Response::from_parts(parts, Body::empty());
    }

    let (mut parts, incoming) = response.into_parts();
    rewrite_response_headers(&mut parts.headers, &prefix, &upstream_host);
    let content_type = parts
        .headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let is_html = content_type.starts_with("text/html");
    let is_text = is_html
        || content_type.starts_with("text/css")
        || content_type.contains("javascript");
    if !is_text || parts.headers.contains_key(header::CONTENT_ENCODING) {
        return Response::from_parts(parts, Body::new(incoming));
    }
    let bytes = match http_body_util::Limited::new(incoming, MAX_REWRITE_BYTES).collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => return plain(StatusCode::BAD_GATEWAY, "Could not read the container response."),
    };
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    text = rewrite_absolute_paths(&text, &prefix, is_html);
    if is_html {
        text = inject_shim(&text, &prefix);
    }
    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.remove(header::TRANSFER_ENCODING);
    Response::from_parts(parts, Body::from(text))
}

fn rewrite_response_headers(headers: &mut HeaderMap, prefix: &str, upstream_host: &str) {
    if let Some(location) = headers.get(header::LOCATION).and_then(|v| v.to_str().ok()).map(str::to_owned) {
        let stripped = location
            .strip_prefix(&format!("http://{upstream_host}"))
            .or_else(|| location.strip_prefix("http://127.0.0.1"))
            .unwrap_or(&location);
        if stripped.starts_with('/') && !stripped.starts_with("//") {
            if let Ok(value) = format!("{prefix}{stripped}").parse() {
                headers.insert(header::LOCATION, value);
            }
        }
    }
    let cookies: Vec<String> = headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(|cookie| {
            cookie
                .split(';')
                .map(|part| {
                    let part = part.trim();
                    match part.get(..5).filter(|p| p.eq_ignore_ascii_case("path=")) {
                        Some(_) => format!("Path={prefix}{}", &part[5..]),
                        None => part.to_owned(),
                    }
                })
                .collect::<Vec<_>>()
                .join("; ")
        })
        .collect();
    if !cookies.is_empty() {
        headers.remove(header::SET_COOKIE);
        for cookie in cookies {
            if let Ok(value) = cookie.parse() {
                headers.append(header::SET_COOKIE, value);
            }
        }
    }
}

/// Prefix root-relative URLs in attributes (`href="/x"`) and CSS `url(/x)`.
fn rewrite_absolute_paths(text: &str, prefix: &str, html: bool) -> String {
    let mut out = text.to_owned();
    if html {
        for attr in ["href", "src", "action", "poster", "data-src"] {
            for quote in ['"', '\''] {
                let from = format!("{attr}={quote}/");
                let to = format!("{attr}={quote}{prefix}/");
                out = replace_skip_double_slash(&out, &from, &to);
            }
        }
    }
    for from in ["url(/", "url(\"/", "url('/"] {
        let to = from.replacen('/', &format!("{prefix}/"), 1);
        out = replace_skip_double_slash(&out, from, &to);
    }
    out
}

/// Like `str::replace`, but leaves protocol-relative `//host` URLs and already-prefixed ones alone.
fn replace_skip_double_slash(text: &str, from: &str, to: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find(from) {
        let after = &rest[index + from.len()..];
        out.push_str(&rest[..index]);
        if after.starts_with('/') || to_prefix_present(to, after) {
            out.push_str(from);
        } else {
            out.push_str(to);
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

fn to_prefix_present(to: &str, after: &str) -> bool {
    // `to` ends with `{prefix}/`; the suffix following `from` already starts with the prefix tail?
    let prefix_tail = to.rsplit_once("/proxy/").map(|(_, tail)| tail.trim_end_matches('/'));
    prefix_tail.is_some_and(|tail| after.starts_with(&format!("proxy/{tail}")))
}

fn inject_shim(html: &str, prefix: &str) -> String {
    let shim = format!(
        "<script>(function(){{var P={prefix:?};function f(u){{return typeof u==='string'&&u[0]==='/'&&u[1]!=='/'&&u!==P&&u.indexOf(P+'/')!==0?P+u:u}}\
var F=window.fetch;if(F)window.fetch=function(i,o){{if(typeof i==='string')i=f(i);else if(typeof Request!=='undefined'&&i instanceof Request){{var u=new URL(i.url);if(u.origin===location.origin&&u.pathname.indexOf(P)!==0)i=new Request(P+u.pathname+u.search,i)}}return F.call(this,i,o)}};\
var X=XMLHttpRequest.prototype.open;XMLHttpRequest.prototype.open=function(m,u){{arguments[1]=f(String(u));return X.apply(this,arguments)}};\
var W=window.WebSocket;window.WebSocket=function(u,p){{try{{var x=new URL(u,location.href);if(x.host===location.host&&x.pathname.indexOf(P)!==0)u=x.protocol+'//'+x.host+P+x.pathname+x.search}}catch(e){{}}return p===undefined?new W(u):new W(u,p)}};\
window.WebSocket.prototype=W.prototype;window.WebSocket.CONNECTING=0;window.WebSocket.OPEN=1;window.WebSocket.CLOSING=2;window.WebSocket.CLOSED=3;\
var E=window.EventSource;if(E)window.EventSource=function(u,c){{return new E(f(String(u)),c)}};\
try{{sessionStorage.setItem('ns_proxy',P);if(location.pathname.indexOf(P)===0)history.replaceState(history.state,'',(location.pathname.slice(P.length)||'/')+location.search+location.hash)}}catch(e){{}}}})();</script>"
    );
    match html.to_ascii_lowercase().find("<head") {
        Some(start) => match html[start..].find('>') {
            Some(end) => {
                let at = start + end + 1;
                format!("{}{shim}{}", &html[..at], &html[at..])
            }
            None => format!("{shim}{html}"),
        },
        None => format!("{shim}{html}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_root_relative_urls_only() {
        let html = r#"<a href="/x"><img src='/i.png'><script src="//cdn/x.js"></script><a href="rel">"#;
        let out = rewrite_absolute_paths(html, "/proxy/t", true);
        assert!(out.contains(r#"href="/proxy/t/x""#));
        assert!(out.contains("src='/proxy/t/i.png'"));
        assert!(out.contains(r#"src="//cdn/x.js""#));
        assert!(out.contains(r#"href="rel""#));
        assert_eq!(rewrite_absolute_paths(&out, "/proxy/t", true), out);
    }

    #[test]
    fn injects_shim_into_head() {
        let out = inject_shim("<html><head><title>x</title></head>", "/proxy/t");
        assert!(out.starts_with("<html><head><script>"));
    }
}
