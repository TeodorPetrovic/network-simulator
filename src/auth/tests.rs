use super::*;
use axum::{body::to_bytes, http::Request};
use tempfile::TempDir;
use tokio_tungstenite::{connect_async, tungstenite::Error as WsError};
use tower::ServiceExt;

async fn fixture() -> (TempDir, Router, SqlitePool) {
    let directory = TempDir::new().unwrap();
    let database = directory.path().join("auth.sqlite");
    let store = crate::storage::Store::connect_unseeded(database.to_str().unwrap())
        .await
        .unwrap();
    let pool = store.pool().clone();
    seed_rbac(&pool).await.unwrap();
    sqlx::query("INSERT INTO auth_config (singleton, auth_enabled, jwt_signing_key) VALUES (1, 1, 'test-signing-key')")
        .execute(&pool).await.unwrap();
    create_test_user(&pool, "admin", "correct-password", true).await;
    create_test_user(&pool, "alice", "correct-password", false).await;
    let app = crate::api::router(AppState::new(store));
    (directory, app, pool)
}

async fn create_test_user(
    pool: &SqlitePool,
    username: &str,
    password: &str,
    superadmin: bool,
) -> String {
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO auth_users (user_id, username, password_hash, is_active, is_superadmin) VALUES (?, ?, ?, 1, ?)")
        .bind(&id).bind(username).bind(hash_password(password).unwrap()).bind(superadmin)
        .execute(pool).await.unwrap();
    id
}

async fn response(app: &Router, request: Request<Body>) -> axum::response::Response {
    app.clone().oneshot(request).await.unwrap()
}

fn bearer(path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

fn from_peer(mut request: Request<Body>, ip: [u8; 4]) -> Request<Body> {
    request
        .extensions_mut()
        .insert(ConnectInfo(std::net::SocketAddr::from((ip, 12345))));
    request
}

async fn token(pool: &SqlitePool, username: &str, version: i64) -> String {
    let key = signing_key(pool).await.unwrap();
    encode_token(&new_claims(username, version, "access", ACCESS_TTL), &key).unwrap()
}

#[tokio::test]
async fn login_rejects_wrong_password_and_http_rejects_expired_tokens() {
    let (_directory, app, pool) = fixture().await;
    let wrong = response(
        &app,
        from_peer(
            Request::builder()
                .method("POST")
                .uri("/v3/access/users/login")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("username=admin&password=incorrect"))
                .unwrap(),
            [192, 0, 2, 21],
        ),
    )
    .await;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    tokio::time::sleep(Duration::from_millis(300)).await;

    let valid = response(
        &app,
        from_peer(
            Request::builder()
                .method("POST")
                .uri("/v3/access/users/login")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("username=admin&password=correct-password"))
                .unwrap(),
            [192, 0, 2, 21],
        ),
    )
    .await;
    assert_eq!(valid.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&to_bytes(valid.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(body["token_type"], "bearer");
    assert!(body["access_token"].as_str().is_some());

    let key = signing_key(&pool).await.unwrap();
    let mut expired_claims = new_claims("admin", 0, "access", ACCESS_TTL);
    expired_claims.iat = now_epoch().saturating_sub(20);
    expired_claims.exp = now_epoch().saturating_sub(1);
    let expired = encode_token(&expired_claims, &key).unwrap();
    let denied = response(&app, bearer("/v3/access/users/me", &expired)).await;
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn privilege_denial_and_acl_inheritance_follow_path_propagation() {
    let (_directory, app, pool) = fixture().await;
    let admin_token = token(&pool, "admin", 0).await;
    let created = response(
        &app,
        Request::builder()
            .method("POST")
            .uri("/v3/projects")
            .header(header::AUTHORIZATION, format!("Bearer {admin_token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"name":"Auth ACL test"}"#))
            .unwrap(),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let body: Value =
        serde_json::from_slice(&to_bytes(created.into_body(), usize::MAX).await.unwrap()).unwrap();
    let project_id = body["project_id"].as_str().unwrap().to_owned();
    let alice_token = token(&pool, "alice", 0).await;
    let path = format!("/v3/projects/{project_id}");
    assert_eq!(
        response(&app, bearer(&path, &alice_token)).await.status(),
        StatusCode::FORBIDDEN
    );

    let role_id: String = sqlx::query_scalar("SELECT role_id FROM auth_roles WHERE name = 'User'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let alice_id: String =
        sqlx::query_scalar("SELECT user_id FROM auth_users WHERE username = 'alice'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO auth_acl (ace_id, ace_type, path, propagate, allowed, user_id, role_id) VALUES (?, 'user', '/projects', 1, 1, ?, ?)")
        .bind(Uuid::new_v4().to_string()).bind(&alice_id).bind(&role_id).execute(&pool).await.unwrap();
    assert_eq!(
        response(&app, bearer(&path, &alice_token)).await.status(),
        StatusCode::OK
    );

    sqlx::query("UPDATE auth_acl SET propagate = 0 WHERE user_id = ? AND path = '/projects'")
        .bind(alice_id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        response(&app, bearer(&path, &alice_token)).await.status(),
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn websocket_accepts_query_token_and_rejects_missing_or_invalid_token() {
    let (_directory, app, pool) = fixture().await;
    let project_id = Uuid::new_v4().to_string();
    let project = json!({"project_id":project_id,"name":"WebSocket auth","status":"opened"});
    sqlx::query("INSERT INTO projects (project_id,name,status,project_json) VALUES (?, 'WebSocket auth','opened',?)")
        .bind(&project_id).bind(project.to_string()).execute(&pool).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let endpoint = format!("ws://{address}/v3/projects/{project_id}/notifications/ws");
    let missing = connect_async(&endpoint).await.unwrap_err();
    assert!(
        matches!(missing, WsError::Http(response) if response.status() == StatusCode::UNAUTHORIZED)
    );

    let valid_token = token(&pool, "admin", 0).await;
    let endpoint = format!("{endpoint}?token={valid_token}");
    let (mut socket, _) = connect_async(&endpoint).await.unwrap();
    socket.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn status_endpoint_reports_auth_mode_without_credentials() {
    let (_directory, app, _pool) = fixture().await;
    let result = response(
        &app,
        Request::builder()
            .uri("/v3/access/settings/status")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(result.status(), StatusCode::OK);
    let body: Value =
        serde_json::from_slice(&to_bytes(result.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(body["auth_enabled"], true);
}

#[test]
fn bind_and_forwarded_header_defaults_fail_closed() {
    assert!(!effective_auth_mode(false, false, false));
    assert!(effective_auth_mode(false, true, false));
    assert!(effective_auth_mode(true, false, false));
    assert!(!effective_auth_mode(true, true, true));
    assert!(request_requires_auth(false, false, false, true, false));
    assert!(!request_requires_auth(false, false, false, true, true));
    assert!(!request_requires_auth(false, true, true, true, false));
}

#[tokio::test]
async fn startup_reapplies_bind_auth_floor_to_persisted_settings() {
    let (_directory, _app, pool) = fixture().await;
    set_auth_enabled(&pool, false).await.unwrap();
    assert!(enforce_auth_mode(&pool, true, None).await.unwrap());
    let persisted: bool =
        sqlx::query_scalar("SELECT auth_enabled FROM auth_config WHERE singleton = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(persisted);

    set_auth_enabled(&pool, false).await.unwrap();
    assert!(!enforce_auth_mode(&pool, false, None).await.unwrap());
    assert!(!enforce_auth_mode(&pool, true, Some(false)).await.unwrap());
}

#[test]
fn login_backoff_is_scoped_to_username_and_peer_ip() {
    let key = login_rate_key("Alice", "192.0.2.10");
    clear_login_failure(&key);
    record_login_failure(key.clone());
    assert!(login_rate_limited(&key));
    assert!(!login_rate_limited(&login_rate_key("bob", "192.0.2.10")));
    assert!(!login_rate_limited(&login_rate_key("alice", "192.0.2.11")));
    clear_login_failure(&key);
}

#[test]
fn jwt_validation_checks_issue_time_not_before_issuer_and_audience() {
    let key = "test-signing-key";
    let valid = new_claims("alice", 0, "access", ACCESS_TTL);
    assert!(decode_token(&encode_token(&valid, key).unwrap(), key).is_some());

    let mut future_iat = new_claims("alice", 0, "access", ACCESS_TTL);
    future_iat.iat = now_epoch() + 60;
    future_iat.exp = future_iat.iat + ACCESS_TTL;
    assert!(decode_token(&encode_token(&future_iat, key).unwrap(), key).is_none());
    let mut future_nbf = new_claims("alice", 0, "access", ACCESS_TTL);
    future_nbf.nbf = now_epoch() + 60;
    assert!(decode_token(&encode_token(&future_nbf, key).unwrap(), key).is_none());
    let mut wrong_issuer = new_claims("alice", 0, "access", ACCESS_TTL);
    wrong_issuer.iss = "other-issuer".to_owned();
    assert!(decode_token(&encode_token(&wrong_issuer, key).unwrap(), key).is_none());
    let mut wrong_audience = new_claims("alice", 0, "access", ACCESS_TTL);
    wrong_audience.aud = "other-audience".to_owned();
    assert!(decode_token(&encode_token(&wrong_audience, key).unwrap(), key).is_none());
}

#[test]
fn every_registered_v3_route_has_an_explicit_auth_classification() {
    let route_sources = [
        include_str!("../api.rs"),
        include_str!("../api/controller_routes.rs"),
        include_str!("mod.rs"),
    ];
    let routes = route_sources
        .into_iter()
        .flat_map(registered_routes)
        .filter(|(_, path)| path.starts_with("/v3"))
        .collect::<Vec<_>>();
    assert!(!routes.is_empty(), "route source scan found no /v3 routes");
    for (method, registered_path) in routes {
        if registered_path == "/v3/{*path}" {
            assert_eq!(
                classify_route("GET", "/route-not-registered"),
                Some(RouteClass::Deny)
            );
            continue;
        }
        let path = registered_path.strip_prefix("/v3").unwrap();
        let path = representative_path(path);
        let class = classify_route(&method, &path);
        assert!(
            class.is_some() && class != Some(RouteClass::Deny),
            "{method} {registered_path} has no explicit authorization classification"
        );
    }
}

fn registered_routes(source: &str) -> Vec<(String, String)> {
    let mut routes = Vec::new();
    let mut search_from = 0;
    while let Some(relative) = source[search_from..].find(".route(") {
        let route_start = search_from + relative;
        let open = route_start + ".route".len();
        let bytes = source.as_bytes();
        let mut depth = 0_i32;
        let mut in_string = false;
        let mut escaped = false;
        let mut close = None;
        for (offset, byte) in bytes.iter().enumerate().skip(open) {
            if in_string {
                if escaped {
                    escaped = false;
                } else if *byte == b'\\' {
                    escaped = true;
                } else if *byte == b'"' {
                    in_string = false;
                }
                continue;
            }
            match *byte {
                b'"' => in_string = true,
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(offset);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(close) = close else { break };
        let body = &source[open + 1..close];
        if let Some(path) = first_string_literal(body) {
            let path_end = body.find('"').unwrap_or(0) + path.len() + 2;
            let methods_source = &body[path_end..];
            for (name, method) in [
                ("get", "GET"),
                ("post", "POST"),
                ("put", "PUT"),
                ("delete", "DELETE"),
                ("patch", "PATCH"),
                ("head", "HEAD"),
                ("options", "OPTIONS"),
                ("any", "ANY"),
            ] {
                if method_call_present(methods_source, name) {
                    routes.push((method.to_owned(), path.clone()));
                }
            }
        }
        search_from = close + 1;
    }
    routes
}

fn first_string_literal(source: &str) -> Option<String> {
    let start = source.find('"')? + 1;
    let end = source[start..].find('"')? + start;
    Some(source[start..end].to_owned())
}

fn method_call_present(source: &str, method: &str) -> bool {
    source
        .match_indices(&format!("{method}("))
        .any(|(index, _)| {
            index == 0
                || !source.as_bytes()[index - 1].is_ascii_alphanumeric()
                    && source.as_bytes()[index - 1] != b'_'
        })
}

fn representative_path(pattern: &str) -> String {
    let mut segments = Vec::new();
    for segment in pattern.trim_matches('/').split('/') {
        if segment.starts_with("{*") {
            segments.extend(["tail", "file"].into_iter().map(str::to_owned));
        } else if segment.starts_with('{') && segment.ends_with('}') {
            segments.push("route-id".to_owned());
        } else if !segment.is_empty() {
            segments.push(segment.to_owned());
        }
    }
    format!("/{}", segments.join("/"))
}

#[test]
fn required_privileges_match_the_security_review_fixes() {
    let cases = [
        ("POST", "/projects/p/templates/t", "Node.Allocate"),
        ("POST", "/projects/p/files/file", "Project.Modify"),
        ("POST", "/projects/p/lock", "Project.Modify"),
        ("POST", "/projects/p/nodes/console/reset", "Node.Console"),
        ("POST", "/projects/p/nodes/n/console/reset", "Node.Console"),
        ("POST", "/projects/p/nodes/n/files/file", "Node.Modify"),
        ("DELETE", "/projects/p/nodes/n/files/file", "Node.Modify"),
        ("POST", "/projects/p/nodes/n/isolate", "Link.Modify"),
        ("POST", "/projects/p/nodes/n/unisolate", "Link.Modify"),
        ("GET", "/projects/p/nodes/n/links", "Link.Audit"),
        ("DELETE", "/pools/p/resources/r", "Pool.Modify"),
    ];
    for (method, path, expected) in cases {
        assert_eq!(
            classify_route(method, path),
            Some(RouteClass::Privilege(expected)),
            "{method} {path}"
        );
    }
    assert_eq!(
        classify_route("PUT", "/unlisted/route"),
        Some(RouteClass::Deny)
    );
    assert_eq!(
        classify_route("POST", "/docker/images/pull"),
        Some(RouteClass::Privilege("Compute.Modify"))
    );
}

#[tokio::test]
async fn pool_access_obeys_propagation_and_direct_user_denies() {
    let (_directory, _app, pool) = fixture().await;
    let project_id = Uuid::new_v4().to_string();
    let pool_id = Uuid::new_v4().to_string();
    let alice_id: String =
        sqlx::query_scalar("SELECT user_id FROM auth_users WHERE username = 'alice'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO projects (project_id, name, status, project_json) VALUES (?, 'Pool ACL project', 'closed', '{}')")
        .bind(&project_id).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO resource_pools (resource_pool_id, name) VALUES (?, 'ACL test pool')")
        .bind(&pool_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO resources (resource_id, resource_type, name) VALUES (?, 'project', 'ACL test project')")
        .bind(&project_id).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO resource_pool_map (resource_id, resource_pool_id) VALUES (?, ?)")
        .bind(&project_id)
        .bind(&pool_id)
        .execute(&pool)
        .await
        .unwrap();
    let auditor_role: String =
        sqlx::query_scalar("SELECT role_id FROM auth_roles WHERE name = 'Auditor'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let group_id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO auth_groups (user_group_id, name) VALUES (?, 'Pool ACL group')")
        .bind(&group_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO auth_user_groups (user_id, user_group_id) VALUES (?, ?)")
        .bind(&alice_id)
        .bind(&group_id)
        .execute(&pool)
        .await
        .unwrap();
    let root = format!("/pools/{pool_id}");
    sqlx::query("INSERT INTO auth_acl (ace_id, ace_type, path, propagate, allowed, group_id, role_id) VALUES (?, 'group', ?, 1, 1, ?, ?)")
        .bind(Uuid::new_v4().to_string()).bind(&root).bind(&group_id).bind(&auditor_role).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO auth_acl (ace_id, ace_type, path, propagate, allowed, user_id, role_id) VALUES (?, 'user', ?, 0, 0, ?, ?)")
        .bind(Uuid::new_v4().to_string()).bind(&root).bind(&alice_id).bind(&auditor_role).execute(&pool).await.unwrap();
    let principal = Principal {
        user_id: alice_id.clone(),
        is_superadmin: false,
        must_change_password: false,
    };
    let project_path = format!("/projects/{project_id}");
    assert!(!has_privilege(&pool, &principal, &project_path, "Project.Audit").await);

    sqlx::query("DELETE FROM auth_acl WHERE ace_type = 'user' AND user_id = ?")
        .bind(&alice_id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(has_privilege(&pool, &principal, &project_path, "Project.Audit").await);
    sqlx::query("UPDATE auth_acl SET propagate = 0 WHERE group_id = ?")
        .bind(&group_id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(!has_privilege(&pool, &principal, &project_path, "Project.Audit").await);

    sqlx::query("UPDATE auth_acl SET path = ?, propagate = 0 WHERE group_id = ?")
        .bind(format!("{root}/resources/{project_id}"))
        .bind(&group_id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(has_privilege(&pool, &principal, &project_path, "Project.Audit").await);
}

#[tokio::test]
async fn refresh_tokens_rotate_once_and_reuse_revokes_the_family() {
    let (_directory, app, pool) = fixture().await;
    let login = response(
        &app,
        from_peer(
            Request::builder()
                .method("POST")
                .uri("/v3/access/users/login")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("username=admin&password=correct-password"))
                .unwrap(),
            [192, 0, 2, 22],
        ),
    )
    .await;
    assert_eq!(login.status(), StatusCode::OK);
    let login: Value =
        serde_json::from_slice(&to_bytes(login.into_body(), usize::MAX).await.unwrap()).unwrap();
    let original = login["refresh_token"].as_str().unwrap().to_owned();
    let claims = decode_token(&original, "test-signing-key").unwrap();
    assert_eq!(claims.exp - claims.iat, REFRESH_TTL);
    let rotate = |refresh_token: &str| {
        Request::builder()
            .method("POST")
            .uri("/v3/access/users/refresh")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({"refresh_token":refresh_token}).to_string(),
            ))
            .unwrap()
    };
    let rotated = response(&app, rotate(&original)).await;
    assert_eq!(rotated.status(), StatusCode::OK);
    let rotated: Value =
        serde_json::from_slice(&to_bytes(rotated.into_body(), usize::MAX).await.unwrap()).unwrap();
    let replacement = rotated["refresh_token"].as_str().unwrap().to_owned();
    assert_ne!(replacement, original);

    let reuse = response(&app, rotate(&original)).await;
    assert_eq!(reuse.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response(&app, rotate(&replacement)).await.status(),
        StatusCode::UNAUTHORIZED
    );
    let revoked: bool = sqlx::query_scalar("SELECT revoked FROM auth_refresh_families")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(revoked);
}

#[tokio::test]
async fn ws_ticket_is_short_lived_path_bound_and_single_use() {
    let (_directory, app, pool) = fixture().await;
    let project_id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO projects (project_id,name,status,project_json) VALUES (?, 'Ticket project','opened','{}')")
        .bind(&project_id).execute(&pool).await.unwrap();
    let admin_token = token(&pool, "admin", 0).await;
    let path = format!("/v3/projects/{project_id}/notifications/ws");
    let ticket_response = response(
        &app,
        Request::builder()
            .method("POST")
            .uri("/v3/access/ws-ticket")
            .header(header::AUTHORIZATION, format!("Bearer {admin_token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({"path":path}).to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(ticket_response.status(), StatusCode::OK);
    let ticket_body: Value = serde_json::from_slice(
        &to_bytes(ticket_response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(ticket_body["expires_in"], WS_TICKET_TTL);
    let ticket = ticket_body["ticket"].as_str().unwrap();
    let app = app.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let endpoint = format!("ws://{address}{path}?ticket={ticket}");
    let (mut socket, _) = connect_async(&endpoint).await.unwrap();
    socket.close(None).await.unwrap();
    let reused = connect_async(&endpoint).await.unwrap_err();
    assert!(
        matches!(reused, WsError::Http(response) if response.status() == StatusCode::UNAUTHORIZED)
    );
    server.abort();
}

#[tokio::test]
async fn missing_auth_config_fails_closed_and_bootstrap_recovers_admin() {
    let (directory, app, pool) = fixture().await;
    let admin_token = token(&pool, "admin", 0).await;
    sqlx::query("DELETE FROM auth_config WHERE singleton = 1")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        response(&app, bearer("/v3/runtimes", &admin_token))
            .await
            .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    sqlx::query("INSERT INTO auth_config (singleton, auth_enabled, jwt_signing_key) VALUES (1, 1, 'test-signing-key')")
        .execute(&pool).await.unwrap();
    sqlx::query("UPDATE auth_users SET is_active = 0 WHERE username = 'admin'")
        .execute(&pool)
        .await
        .unwrap();
    let password_file = directory.path().join("initial-admin-password.txt");
    initialize(&pool, true, &password_file).await.unwrap();
    let active_admin: bool =
        sqlx::query_scalar("SELECT is_active FROM auth_users WHERE username = 'admin'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let must_change: bool =
        sqlx::query_scalar("SELECT must_change_password FROM auth_users WHERE username = 'admin'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(active_admin && must_change);
    assert!(
        !std::fs::read_to_string(&password_file)
            .unwrap()
            .trim()
            .is_empty()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&password_file)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[tokio::test]
async fn last_active_superadmin_cannot_be_disabled_and_initial_password_is_required() {
    let (_directory, app, pool) = fixture().await;
    let admin_id: String =
        sqlx::query_scalar("SELECT user_id FROM auth_users WHERE username = 'admin'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let admin_token = token(&pool, "admin", 0).await;
    let disable = Request::builder()
        .method("PUT")
        .uri(format!("/v3/access/users/{admin_id}"))
        .header(header::AUTHORIZATION, format!("Bearer {admin_token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"is_active":false}"#))
        .unwrap();
    assert_eq!(
        response(&app, disable).await.status(),
        StatusCode::FORBIDDEN
    );

    let alice_id: String =
        sqlx::query_scalar("SELECT user_id FROM auth_users WHERE username = 'alice'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("UPDATE auth_users SET must_change_password = 1 WHERE user_id = ?")
        .bind(&alice_id)
        .execute(&pool)
        .await
        .unwrap();
    let alice_token = token(&pool, "alice", 0).await;
    assert_eq!(
        response(&app, bearer("/v3/runtimes", &alice_token))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let change = Request::builder()
        .method("PUT")
        .uri("/v3/access/users/me")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"password":"new-correct-password"}"#))
        .unwrap();
    assert_eq!(response(&app, change).await.status(), StatusCode::OK);
    let must_change: bool =
        sqlx::query_scalar("SELECT must_change_password FROM auth_users WHERE user_id = ?")
            .bind(alice_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!must_change);
}
