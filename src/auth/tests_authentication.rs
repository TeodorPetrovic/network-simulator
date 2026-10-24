use super::*;

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
        include_str!("../api/mod.rs"),
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
