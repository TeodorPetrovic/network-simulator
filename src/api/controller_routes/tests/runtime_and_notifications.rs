use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::Row;
use tempfile::TempDir;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};
use tower::ServiceExt;
use uuid::Uuid;

use super::super::{AppState, router};
use crate::storage::Store;

async fn app() -> (axum::Router, TempDir, Store) {
    let dir = TempDir::new().expect("test directory");
    let store = Store::connect(dir.path().join("api.sqlite").to_str().unwrap())
        .await
        .unwrap();
    let state = AppState::new(store.clone()).with_bind_address("127.0.0.1:3182".parse().unwrap());
    (router(state), dir, store)
}

async fn mock_dynamips(commands: Vec<(&'static str, &'static str)>) -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        for (expected, response) in commands {
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut reader = BufReader::new(read);
            let mut command = String::new();
            reader.read_line(&mut command).await.unwrap();
            assert_eq!(command.trim(), expected);
            write.write_all(response.as_bytes()).await.unwrap();
        }
    });
    port
}

async fn send(
    app: &axum::Router,
    method: Method,
    uri: &str,
    body: Body,
    content_type: Option<&str>,
) -> axum::response::Response {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(value) = content_type {
        builder = builder.header("content-type", value);
    }
    app.clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap()
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn assert_node_has_no_runtime_state(node: &Value) {
    if let Some(fields) = node.as_object() {
        for key in fields.keys() {
            assert!(
                !key.to_ascii_lowercase().starts_with("runtime_"),
                "{key} was retained"
            );
        }
    }
    for key in [
        "container_id",
        "container_name",
        "container_pid",
        "image_digest",
        "dynamips_id",
        "pid",
        "pid_file",
        "qemu_pid",
        "qmp",
        "qmp_socket",
        "qmp_path",
        "serial",
        "serial_socket",
        "serial_path",
        "overlay",
        "overlay_path",
        "log",
        "log_file",
        "log_path",
        "working_dir",
        "working_directory",
        "hypervisor_port",
        "console",
        "aux",
        "console_host",
        "console_port",
        "aux_port",
        "runtime_overlay",
        "runtime_qmp_socket",
        "runtime_serial_socket",
        "runtime_pid_file",
        "runtime_log_file",
        "runtime_working_dir",
        "runtime_hypervisor_port",
        "runtime_hdb_overlay",
    ] {
        assert!(node.get(key).is_none(), "{key} was retained");
    }
}

async fn websocket_status(app: &axum::Router, uri: &str) -> StatusCode {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        time::{Duration, timeout},
    };

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server_app = app.clone();
    let server = tokio::spawn(async move { axum::serve(listener, server_app).await.unwrap() });
    let mut client = TcpStream::connect(address).await.unwrap();
    client
            .write_all(
                format!(
                    "GET {uri} HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    let mut response = [0_u8; 512];
    let length = timeout(Duration::from_secs(2), client.read(&mut response))
        .await
        .unwrap()
        .unwrap();
    server.abort();
    let first_line = String::from_utf8_lossy(&response[..length]);
    StatusCode::from_u16(
        first_line
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn dynamips_idlepc_routes_use_the_hypervisor_protocol() {
    let dir = TempDir::new().expect("test directory");
    let store = Store::connect(dir.path().join("api.sqlite").to_str().unwrap())
        .await
        .unwrap();
    let image_dir = dir.path().join("ios");
    let state_dir = dir.path().join("dynamips-state");
    std::fs::create_dir_all(&image_dir).unwrap();
    std::fs::create_dir_all(&state_dir).unwrap();
    let image = image_dir.join("router.image");
    std::fs::write(&image, [0x7f, b'E', b'L', b'F', 1, 2, 1, 0]).unwrap();
    let mut state = AppState::new(store.clone());
    state.dynamips =
        crate::dynamips::DynamipsRuntime::with_paths("dynamips", &image_dir, &state_dir);
    let app = router(state);
    let project_id = uuid::Uuid::new_v4();
    let project = send(
        &app,
        Method::POST,
        "/v3/projects",
        Body::from(format!(
            r#"{{"project_id":"{project_id}","name":"IOS lab"}}"#
        )),
        Some("application/json"),
    )
    .await;
    assert_eq!(project.status(), StatusCode::CREATED);

    let proposals_node = uuid::Uuid::new_v4();
    let auto_node = uuid::Uuid::new_v4();
    let proposals_port = mock_dynamips(vec![
        ("vm get_status \"R1\"", "100 2\r\n100-OK\r\n"),
        ("vm set_idle_pc_online \"R1\" 0 0x0", "100-OK\r\n"),
        (
            "vm get_idle_pc_prop \"R1\" 0",
            "100 0x1234abcd 90%\r\n100-OK\r\n",
        ),
        ("vm set_idle_pc_online \"R1\" 0 0x0", "100-OK\r\n"),
    ])
    .await;
    let auto_port = mock_dynamips(vec![
        ("vm get_status \"R2\"", "100 2\r\n100-OK\r\n"),
        ("vm set_idle_pc_online \"R2\" 0 0x0", "100-OK\r\n"),
        (
            "vm get_idle_pc_prop \"R2\" 0",
            "100 0x1234abcd 90%\r\n100-OK\r\n",
        ),
        ("vm set_idle_pc_online \"R2\" 0 0x1234abcd", "100-OK\r\n"),
        ("vm cpu_usage \"R2\" 0", "100 100\r\n100-OK\r\n"),
        ("vm cpu_usage \"R2\" 0", "100 101\r\n100-OK\r\n"),
    ])
    .await;
    for (node_id, name, port) in [
        (proposals_node, "R1", proposals_port),
        (auto_node, "R2", auto_port),
    ] {
        let working_dir = state_dir
            .join(project_id.to_string())
            .join(node_id.to_string());
        std::fs::create_dir_all(&working_dir).unwrap();
        std::fs::write(
            working_dir.join(".dynamips-runtime"),
            format!("{project_id}\n{node_id}\n{port}\n0\n0\n"),
        )
        .unwrap();
        let node = json!({
            "node_id":node_id,
            "project_id":project_id,
            "name":name,
            "node_type":"dynamips",
            "platform":"c7200",
            "slot0":"C7200-IO-FE",
            "image":image.display().to_string(),
            "ram":512,
            "runtime_working_dir":working_dir.display().to_string(),
            "runtime_log_file":working_dir.join("dynamips_stdout.log").display().to_string(),
            "runtime_hypervisor_port":port,
            "status":"started",
            "x":0,
            "y":0,
            "locked":false
        });
        sqlx::query("INSERT INTO topology_objects(project_id,kind,object_id,name,object_json) VALUES(?,?,?,?,?)")
                .bind(project_id.to_string())
                .bind("node")
                .bind(node_id.to_string())
                .bind(name)
                .bind(node.to_string())
                .execute(store.pool())
                .await
                .unwrap();
    }

    let proposals = send(
        &app,
        Method::GET,
        &format!("/v3/projects/{project_id}/nodes/{proposals_node}/dynamips/idlepc_proposals"),
        Body::empty(),
        None,
    )
    .await;
    assert_eq!(proposals.status(), StatusCode::OK);
    assert_eq!(json_body(proposals).await, json!(["0x1234abcd 90%"]));

    let auto = send(
        &app,
        Method::GET,
        &format!("/v3/projects/{project_id}/nodes/{auto_node}/dynamips/auto_idlepc"),
        Body::empty(),
        None,
    )
    .await;
    assert_eq!(auto.status(), StatusCode::OK);
    assert_eq!(json_body(auto).await, json!({"idlepc":"0x1234abcd"}));
    let stored_node: String = sqlx::query_scalar(
            "SELECT object_json FROM topology_objects WHERE project_id = ? AND kind = 'node' AND object_id = ?",
        )
        .bind(project_id.to_string())
        .bind(auto_node.to_string())
        .fetch_one(store.pool())
        .await
        .unwrap();
    let stored_node: Value = serde_json::from_str(&stored_node).unwrap();
    assert_eq!(stored_node["idlepc"], "0x1234abcd");
}

#[tokio::test]
async fn project_archive_loading_reads_a_local_gns3_file() {
    let (app, dir, _store) = app().await;
    let path = dir.path().join("local.gns3");
    tokio::fs::write(
        &path,
        r#"{"name":"Loaded lab","nodes":[],"links":[],"drawings":[]}"#,
    )
    .await
    .unwrap();
    let response = send(
        &app,
        Method::POST,
        "/v3/projects/load",
        Body::from(json!({"path":path}).to_string()),
        Some("application/json"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(json_body(response).await["name"], "Loaded lab");
}

#[tokio::test]
async fn controller_and_project_notification_websockets_upgrade_over_http() {
    let (app, _dir, _store) = app().await;
    let response = send(
        &app,
        Method::POST,
        "/v3/projects",
        Body::from(r#"{"name":"WS lab"}"#),
        Some("application/json"),
    )
    .await;
    let project = json_body(response).await;
    let project_id = project["project_id"].as_str().unwrap();
    assert_eq!(
        websocket_status(&app, "/v3/notifications/ws").await,
        StatusCode::SWITCHING_PROTOCOLS
    );
    assert_eq!(
        websocket_status(&app, &format!("/v3/projects/{project_id}/notifications/ws")).await,
        StatusCode::SWITCHING_PROTOCOLS
    );
}
