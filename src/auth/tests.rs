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

#[tokio::test]
async fn acl_endpoint_catalog_includes_project_nodes_and_links() {
    let (_directory, app, pool) = fixture().await;
    let admin = token(&pool, "admin", 0).await;
    let project_id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO projects (project_id, name, status, project_json) VALUES (?, 'ACL catalog lab', 'closed', ?)")
        .bind(&project_id)
        .bind(json!({"project_id":project_id,"name":"ACL catalog lab","status":"closed"}).to_string())
        .execute(&pool)
        .await
        .unwrap();
    let first = Uuid::new_v4().to_string();
    let second = Uuid::new_v4().to_string();
    let link_id = Uuid::new_v4().to_string();
    for (node_id, name) in [(&first, "Router A"), (&second, "Router B")] {
        let node =
            json!({"node_id":node_id,"project_id":project_id,"name":name,"node_type":"vpcs"});
        sqlx::query("INSERT INTO topology_objects (project_id, kind, object_id, name, object_json) VALUES (?, 'node', ?, ?, ?)")
            .bind(&project_id).bind(node_id).bind(name).bind(node.to_string())
            .execute(&pool).await.unwrap();
    }
    let link = json!({"link_id":link_id,"nodes":[{"node_id":first},{"node_id":second}]});
    sqlx::query("INSERT INTO topology_objects (project_id, kind, object_id, object_json) VALUES (?, 'link', ?, ?)")
        .bind(&project_id).bind(&link_id).bind(link.to_string())
        .execute(&pool).await.unwrap();

    let (status, endpoints) =
        json_route(&app, "GET", "/v3/access/acl/endpoints", &admin, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    let endpoint = |path: String| {
        endpoints
            .as_array()
            .unwrap()
            .iter()
            .find(|value| value["endpoint"] == path)
            .cloned()
            .unwrap_or_else(|| panic!("missing ACL endpoint {path}"))
    };
    assert_eq!(
        endpoint(format!("/projects/{project_id}/nodes"))["endpoint_type"],
        "node"
    );
    assert_eq!(
        endpoint(format!("/projects/{project_id}/nodes/{first}"))["name"],
        "Node \"Router A\" in project \"ACL catalog lab\""
    );
    assert_eq!(
        endpoint(format!("/projects/{project_id}/links"))["endpoint_type"],
        "link"
    );
    assert_eq!(
        endpoint(format!("/projects/{project_id}/links/{link_id}"))["name"],
        "Link from \"Router A\" to \"Router B\" in project \"ACL catalog lab\""
    );
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
    for rule in ROUTE_RULES.iter().chain(ROUTE_RULES_EXTRA.iter()) {
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

#[path = "tests_acl.rs"]
mod tests_acl;
#[path = "tests_authentication.rs"]
mod tests_authentication;
#[path = "tests_classroom.rs"]
mod tests_classroom;
#[path = "tests_tokens.rs"]
mod tests_tokens;
