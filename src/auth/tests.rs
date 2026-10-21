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

async fn json_route(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = response(
        app,
        Request::builder()
            .method(method)
            .uri(path)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

async fn admin_create(app: &Router, admin: &str, path: &str, body: Value) -> Value {
    let (status, value) = json_route(app, "POST", path, admin, body).await;
    assert_eq!(status, StatusCode::CREATED, "POST {path}: {value}");
    value
}

async fn admin_no_content(app: &Router, admin: &str, method: &str, path: &str) {
    let (status, value) = json_route(app, method, path, admin, Value::Null).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{method} {path}: {value}");
}

async fn student_login(app: &Router, username: &str, peer: [u8; 4]) -> String {
    let login = response(
        app,
        from_peer(
            Request::builder()
                .method("POST")
                .uri("/v3/access/users/login")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!(
                    "username={username}&password=classroom-password"
                )))
                .unwrap(),
            peer,
        ),
    )
    .await;
    assert_eq!(login.status(), StatusCode::OK, "{username} login");
    let login: Value =
        serde_json::from_slice(&to_bytes(login.into_body(), usize::MAX).await.unwrap()).unwrap();
    login["access_token"].as_str().unwrap().to_owned()
}

struct ClassroomProject {
    project_id: String,
    node_id: String,
    link_id: String,
    snapshot_id: String,
}

/// Seeds a project the way an instructor would: through the API for the
/// project, snapshot and file, plus stored topology rows for one node and
/// link (node creation needs real runtimes, which this test does not use).
async fn classroom_project(
    app: &Router,
    pool: &SqlitePool,
    admin: &str,
    name: &str,
) -> ClassroomProject {
    let project = admin_create(app, admin, "/v3/projects", json!({"name":name})).await;
    let project_id = project["project_id"].as_str().unwrap().to_owned();
    let node_id = Uuid::new_v4().to_string();
    let peer_id = Uuid::new_v4().to_string();
    for (id, node_name) in [(&node_id, "PC1"), (&peer_id, "PC2")] {
        let node = json!({"node_id":id,"project_id":project_id,"node_type":"vpcs","name":node_name,"status":"stopped","ports":[]});
        sqlx::query("INSERT INTO topology_objects (project_id, kind, object_id, name, object_json) VALUES (?, 'node', ?, ?, ?)")
            .bind(&project_id).bind(id).bind(node_name).bind(node.to_string()).execute(pool).await.unwrap();
    }
    let link_id = Uuid::new_v4().to_string();
    let link = json!({"link_id":link_id,"project_id":project_id,"nodes":[
        {"node_id":node_id,"adapter_number":0,"port_number":0},
        {"node_id":peer_id,"adapter_number":0,"port_number":0}]});
    sqlx::query("INSERT INTO topology_objects (project_id, kind, object_id, name, object_json) VALUES (?, 'link', ?, NULL, ?)")
        .bind(&project_id).bind(&link_id).bind(link.to_string()).execute(pool).await.unwrap();
    let snapshot = admin_create(
        app,
        admin,
        &format!("/v3/projects/{project_id}/snapshots"),
        json!({"name":format!("{name} baseline")}),
    )
    .await;
    let snapshot_id = snapshot["snapshot_id"].as_str().unwrap().to_owned();
    let file = response(
        app,
        Request::builder()
            .method("POST")
            .uri(format!("/v3/projects/{project_id}/files/answers.txt"))
            .header(header::AUTHORIZATION, format!("Bearer {admin}"))
            .body(Body::from(format!("{name} private answers")))
            .unwrap(),
    )
    .await;
    assert!(
        file.status().is_success(),
        "project file write: {}",
        file.status()
    );
    ClassroomProject {
        project_id,
        node_id,
        link_id,
        snapshot_id,
    }
}

fn classroom_route_path(pattern: &str, target: &ClassroomProject) -> String {
    let mut segments = Vec::new();
    for segment in pattern.trim_matches('/').split('/') {
        let value = match segment {
            "{project_id}" => target.project_id.clone(),
            "{node_id}" => target.node_id.clone(),
            "{link_id}" => target.link_id.clone(),
            "{snapshot_id}" => target.snapshot_id.clone(),
            "{*file_path}" => "answers.txt".to_owned(),
            "{*image_path}" => "qemu/disk.qcow2".to_owned(),
            "{*path}" => continue,
            other if other.starts_with('{') => Uuid::new_v4().to_string(),
            other => other.to_owned(),
        };
        if !value.is_empty() {
            segments.push(value);
        }
    }
    format!("/v3/{}", segments.join("/"))
}

async fn assert_student_isolated(
    app: &Router,
    student: &str,
    who: &str,
    own: &ClassroomProject,
    other: &ClassroomProject,
) {
    let (status, projects) = json_route(app, "GET", "/v3/projects", student, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{who} lists projects");
    let listed = projects
        .as_array()
        .unwrap()
        .iter()
        .map(|project| project["project_id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        listed,
        vec![own.project_id.clone()],
        "{who} sees only own project"
    );

    for path in [
        format!("/v3/projects/{}", own.project_id),
        format!("/v3/projects/{}/nodes", own.project_id),
        format!("/v3/projects/{}/nodes/{}", own.project_id, own.node_id),
        format!("/v3/projects/{}/links", own.project_id),
        format!("/v3/projects/{}/links/{}", own.project_id, own.link_id),
        format!("/v3/projects/{}/snapshots", own.project_id),
        format!("/v3/projects/{}/files/answers.txt", own.project_id),
        format!("/v3/projects/{}/export", own.project_id),
        format!("/v3/projects/{}/topology", own.project_id),
    ] {
        assert_eq!(
            response(app, bearer(&path, student)).await.status(),
            StatusCode::OK,
            "{who} GET own {path}"
        );
    }

    // Every privileged route that names the other student's project must be
    // rejected by the gate, for every method, before any handler runs.
    let mut checked = 0;
    for rule in ROUTE_RULES {
        let RouteClass::Privilege(_) = rule.class else {
            continue;
        };
        let path = classroom_route_path(rule.path, other);
        let foreign_project = rule.path.contains("{project_id}");
        let collection = rule.method == "GET" && rule.path == "/projects";
        if collection {
            continue;
        }
        let (status, body) = json_route(app, rule.method, &path, student, json!({})).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{who} {} {path} ({}) must be forbidden: {body}",
            rule.method,
            if foreign_project {
                "other student's project"
            } else {
                "administrative route"
            },
        );
        checked += 1;
    }
    assert!(checked > 150, "route sweep covered only {checked} routes");

    for runtime in ["qemu", "docker", "vpcs", "dynamips"] {
        let path = format!(
            "/v3/compute/projects/{}/{runtime}/nodes/{}/console/ws",
            other.project_id, other.node_id
        );
        let (status, _) = json_route(
            app,
            "POST",
            "/v3/access/ws-ticket",
            student,
            json!({"path":path}),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{who} console ticket for {path}"
        );
    }
    let foreign_notifications = format!("/v3/projects/{}/notifications/ws", other.project_id);
    let (status, _) = json_route(
        app,
        "POST",
        "/v3/access/ws-ticket",
        student,
        json!({"path":foreign_notifications}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{who} notification ticket");
    let foreign_capture = format!(
        "/v3/projects/{}/links/{}/capture/stream",
        other.project_id, other.link_id
    );
    let (status, _) = json_route(
        app,
        "POST",
        "/v3/access/ws-ticket",
        student,
        json!({"path":foreign_capture}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{who} capture ticket");
}

#[tokio::test]
async fn classroom_flow_isolates_students_by_project_and_pool_acl() {
    let (_directory, app, pool) = fixture().await;
    let admin = token(&pool, "admin", 0).await;

    // The instructor builds a student role with full control inside a lab.
    let role = admin_create(
        &app,
        &admin,
        "/v3/access/roles",
        json!({"name":"Classroom student"}),
    )
    .await;
    let role_id = role["role_id"].as_str().unwrap().to_owned();
    let (status, privileges) =
        json_route(&app, "GET", "/v3/access/privileges", &admin, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    let lab_privileges = privileges
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| {
            let name = entry["name"].as_str().unwrap();
            ["Project.", "Node.", "Link.", "Snapshot.", "Drawing."]
                .iter()
                .any(|prefix| name.starts_with(prefix))
        })
        .map(|entry| entry["privilege_id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert!(
        lab_privileges.len() >= 15,
        "seeded lab privileges: {lab_privileges:?}"
    );
    for privilege_id in &lab_privileges {
        admin_no_content(
            &app,
            &admin,
            "PUT",
            &format!("/v3/access/roles/{role_id}/privileges/{privilege_id}"),
        )
        .await;
    }

    let mut user_ids = Vec::new();
    for username in ["student-one", "student-two"] {
        let user = admin_create(
            &app,
            &admin,
            "/v3/access/users",
            json!({"username":username,"password":"classroom-password","full_name":username}),
        )
        .await;
        user_ids.push(user["user_id"].as_str().unwrap().to_owned());
    }
    let first = classroom_project(&app, &pool, &admin, "Student one lab").await;
    let second = classroom_project(&app, &pool, &admin, "Student two lab").await;

    // Student one: group membership plus a group ACE on the lab project.
    let group = admin_create(
        &app,
        &admin,
        "/v3/access/groups",
        json!({"name":"Classroom A"}),
    )
    .await;
    let group_id = group["user_group_id"].as_str().unwrap().to_owned();
    admin_no_content(
        &app,
        &admin,
        "PUT",
        &format!("/v3/access/groups/{group_id}/members/{}", user_ids[0]),
    )
    .await;
    admin_create(
        &app,
        &admin,
        "/v3/access/acl",
        json!({"ace_type":"group","group_id":group_id,"role_id":role_id,
        "path":format!("/projects/{}", first.project_id),"propagate":true,"allowed":true}),
    )
    .await;

    // Student two: a user ACE on a resource pool containing the lab project.
    let resource_pool = admin_create(
        &app,
        &admin,
        "/v3/pools",
        json!({"name":"Student two pool"}),
    )
    .await;
    let pool_id = resource_pool["resource_pool_id"]
        .as_str()
        .unwrap()
        .to_owned();
    admin_no_content(
        &app,
        &admin,
        "PUT",
        &format!("/v3/pools/{pool_id}/resources/{}", second.project_id),
    )
    .await;
    admin_create(
        &app,
        &admin,
        "/v3/access/acl",
        json!({"ace_type":"user","user_id":user_ids[1],"role_id":role_id,
        "path":format!("/pools/{pool_id}"),"propagate":true,"allowed":true}),
    )
    .await;

    let student_one = student_login(&app, "student-one", [192, 0, 2, 41]).await;
    let student_two = student_login(&app, "student-two", [192, 0, 2, 42]).await;
    assert_student_isolated(&app, &student_one, "student-one", &first, &second).await;
    assert_student_isolated(&app, &student_two, "student-two", &second, &first).await;

    // Real WebSocket upgrades: own notifications open, the other lab does not.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server_app = app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, server_app).await.unwrap();
    });
    let own_path = format!("/v3/projects/{}/notifications/ws", first.project_id);
    let (status, ticket) = json_route(
        &app,
        "POST",
        "/v3/access/ws-ticket",
        &student_one,
        json!({"path":own_path}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (mut socket, _) = connect_async(format!(
        "ws://{address}{own_path}?ticket={}",
        ticket["ticket"].as_str().unwrap()
    ))
    .await
    .unwrap();
    socket.close(None).await.unwrap();
    let foreign_path = format!("/v3/projects/{}/notifications/ws", second.project_id);
    let rejected = connect_async(format!("ws://{address}{foreign_path}?token={student_one}"))
        .await
        .unwrap_err();
    assert!(
        matches!(rejected, WsError::Http(response) if response.status() == StatusCode::FORBIDDEN)
    );
    let console_path = format!(
        "/v3/compute/projects/{}/vpcs/nodes/{}/console/ws",
        second.project_id, second.node_id
    );
    let rejected = connect_async(format!("ws://{address}{console_path}?token={student_one}"))
        .await
        .unwrap_err();
    assert!(
        matches!(rejected, WsError::Http(response) if response.status() == StatusCode::FORBIDDEN)
    );
    server.abort();

    // Deleting a lab removes its ACEs and pool membership, so a later project
    // with the same ID does not inherit the old grants. The seeded link rows
    // have no runtime endpoint metadata for the delete path to tear down.
    sqlx::query("DELETE FROM topology_objects WHERE kind = 'link'")
        .execute(&pool)
        .await
        .unwrap();
    admin_no_content(
        &app,
        &admin,
        "DELETE",
        &format!("/v3/projects/{}", second.project_id),
    )
    .await;
    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM resources WHERE resource_id = ?")
        .bind(&second.project_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(remaining, 0);
    admin_no_content(
        &app,
        &admin,
        "DELETE",
        &format!("/v3/projects/{}", first.project_id),
    )
    .await;
    let stale: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_acl WHERE path = ?")
        .bind(format!("/projects/{}", first.project_id))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stale, 0);
    let pool_ace: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_acl WHERE path = ?")
        .bind(format!("/pools/{pool_id}"))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(pool_ace, 1, "the pool root ACE is not project-owned");
    admin_create(
        &app,
        &admin,
        "/v3/projects",
        json!({"name":"Reused lab","project_id":first.project_id}),
    )
    .await;
    let (status, projects) =
        json_route(&app, "GET", "/v3/projects", &student_one, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(projects, json!([]));
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
    let (_directory, app, pool) = fixture().await;
    let project_id = Uuid::new_v4().to_string();
    let pool_id = Uuid::new_v4().to_string();
    let alice_id: String =
        sqlx::query_scalar("SELECT user_id FROM auth_users WHERE username = 'alice'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO projects (project_id, name, status, project_json) VALUES (?, 'Pool ACL project', 'closed', ?)")
        .bind(&project_id).bind(json!({"project_id":project_id,"name":"Pool ACL project","status":"closed"}).to_string())
        .execute(&pool).await.unwrap();
    let other_project = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO projects (project_id, name, status, project_json) VALUES (?, 'Other pool project', 'closed', ?)")
        .bind(&other_project).bind(json!({"project_id":other_project,"name":"Other pool project","status":"closed"}).to_string())
        .execute(&pool).await.unwrap();
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
    let alice_token = token(&pool, "alice", 0).await;
    let (status, projects) =
        json_route(&app, "GET", "/v3/projects", &alice_token, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(projects.as_array().unwrap().len(), 1);
    assert_eq!(projects[0]["project_id"], project_id);
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

#[test]
fn equal_depth_acl_denies_are_order_independent() {
    let allow = ("/projects/classroom".to_owned(), true, true);
    let deny = ("/projects/classroom".to_owned(), false, false);
    assert_eq!(
        evaluate_acl("/projects/classroom", &[allow.clone(), deny.clone()]),
        Some(false)
    );
    assert_eq!(
        evaluate_acl("/projects/classroom", &[deny, allow]),
        Some(false)
    );
}

async fn insert_ace(
    pool: &SqlitePool,
    principal: (&str, &str),
    path: &str,
    propagate: bool,
    allowed: bool,
    role_id: &str,
) {
    let (ace_type, principal_id) = principal;
    let (user_id, group_id) = if ace_type == "user" {
        (Some(principal_id), None)
    } else {
        (None, Some(principal_id))
    };
    sqlx::query("INSERT INTO auth_acl (ace_id, ace_type, path, propagate, allowed, user_id, group_id, role_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(Uuid::new_v4().to_string()).bind(ace_type).bind(path).bind(propagate).bind(allowed)
        .bind(user_id).bind(group_id).bind(role_id)
        .execute(pool).await.unwrap();
}

#[tokio::test]
async fn user_aces_replace_group_aces_and_pool_aces_join_depth_ordering() {
    let (_directory, _app, pool) = fixture().await;
    let alice_id: String =
        sqlx::query_scalar("SELECT user_id FROM auth_users WHERE username = 'alice'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let role_id: String = sqlx::query_scalar("SELECT role_id FROM auth_roles WHERE name = 'User'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let group_id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO auth_groups (user_group_id, name) VALUES (?, 'Precedence group')")
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
    let project_id = Uuid::new_v4().to_string();
    let pool_id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO projects (project_id, name, status, project_json) VALUES (?, 'Precedence', 'closed', '{}')")
        .bind(&project_id).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO resource_pools (resource_pool_id, name) VALUES (?, 'Precedence pool')",
    )
    .bind(&pool_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO resources (resource_id, resource_type, name) VALUES (?, 'project', 'Precedence')")
        .bind(&project_id).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO resource_pool_map (resource_id, resource_pool_id) VALUES (?, ?)")
        .bind(&project_id)
        .bind(&pool_id)
        .execute(&pool)
        .await
        .unwrap();
    let principal = Principal {
        user_id: alice_id.clone(),
        is_superadmin: false,
        must_change_password: false,
    };
    let user = ("user", alice_id.as_str());
    let group = ("group", group_id.as_str());
    let project = format!("/projects/{project_id}");
    let nodes = format!("{project}/nodes");
    let pool_root = format!("/pools/{pool_id}");
    let clear = || async {
        sqlx::query("DELETE FROM auth_acl")
            .execute(&pool)
            .await
            .unwrap();
    };

    // A user allow replaces a group deny on the same path.
    insert_ace(&pool, group, &project, true, false, &role_id).await;
    insert_ace(&pool, user, &project, true, true, &role_id).await;
    assert!(has_privilege(&pool, &principal, &project, "Project.Audit").await);
    clear().await;

    // A user deny on an ancestor replaces a deeper group allow.
    insert_ace(&pool, user, "/projects", true, false, &role_id).await;
    insert_ace(&pool, group, &project, true, true, &role_id).await;
    assert!(!has_privilege(&pool, &principal, &project, "Project.Audit").await);
    clear().await;

    // A pool deny acts at project depth, so it beats an inherited allow.
    insert_ace(&pool, group, "/", true, true, &role_id).await;
    insert_ace(&pool, group, &pool_root, false, false, &role_id).await;
    assert!(!has_privilege(&pool, &principal, &project, "Project.Audit").await);
    assert!(has_privilege(&pool, &principal, "/projects/unpooled", "Project.Audit").await);
    clear().await;

    // A pool grant still yields to a deeper direct deny.
    insert_ace(&pool, group, &pool_root, true, true, &role_id).await;
    insert_ace(&pool, group, &nodes, true, false, &role_id).await;
    assert!(has_privilege(&pool, &principal, &project, "Project.Audit").await);
    assert!(!has_privilege(&pool, &principal, &nodes, "Node.Audit").await);
    clear().await;

    // Pool descendants other than the project's resource entry grant nothing,
    // and a non-propagating resource entry covers only the project itself.
    insert_ace(
        &pool,
        group,
        &format!("{pool_root}/resources"),
        true,
        true,
        &role_id,
    )
    .await;
    assert!(!has_privilege(&pool, &principal, &project, "Project.Audit").await);
    clear().await;
    insert_ace(
        &pool,
        group,
        &format!("{pool_root}/resources/{project_id}"),
        false,
        true,
        &role_id,
    )
    .await;
    assert!(has_privilege(&pool, &principal, &project, "Project.Audit").await);
    assert!(!has_privilege(&pool, &principal, &nodes, "Node.Audit").await);
    clear().await;

    // ACE rows are evaluated identically regardless of insertion order.
    for order in [[true, false], [false, true]] {
        for allowed in order {
            insert_ace(&pool, group, &project, true, allowed, &role_id).await;
        }
        assert!(!has_privilege(&pool, &principal, &project, "Project.Audit").await);
        clear().await;
    }
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
