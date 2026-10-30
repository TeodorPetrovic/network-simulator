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
async fn project_stats_lock_snapshots_files_export_import_and_duplicate_work_over_http() {
    let (app, _dir, store) = app().await;
    let response = send(
        &app,
        Method::POST,
        "/v3/projects",
        Body::from(r#"{"name":"API parity lab"}"#),
        Some("application/json"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let project = json_body(response).await;
    let id = project["project_id"].as_str().unwrap();
    assert_eq!(
        json_body(
            send(
                &app,
                Method::GET,
                &format!("/v3/projects/{id}/stats"),
                Body::empty(),
                None
            )
            .await
        )
        .await["nodes"],
        0
    );
    assert_eq!(
        json_body(
            send(
                &app,
                Method::GET,
                &format!("/v3/projects/{id}/locked"),
                Body::empty(),
                None
            )
            .await
        )
        .await,
        false
    );
    assert_eq!(
        send(
            &app,
            Method::POST,
            &format!("/v3/projects/{id}/lock"),
            Body::empty(),
            None
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    let project_stream = send(
        &app,
        Method::GET,
        &format!("/v3/projects/{id}/notifications"),
        Body::empty(),
        None,
    )
    .await;
    assert_eq!(project_stream.status(), StatusCode::OK);
    assert_eq!(
        project_stream.headers()["content-type"],
        "application/x-ndjson"
    );
    assert_eq!(
        json_body(
            send(
                &app,
                Method::GET,
                &format!("/v3/projects/{id}/locked"),
                Body::empty(),
                None
            )
            .await
        )
        .await,
        true
    );
    assert_eq!(
        send(
            &app,
            Method::POST,
            &format!("/v3/projects/{id}/unlock"),
            Body::empty(),
            None
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    sqlx::query("INSERT INTO project_files(project_id,file_path,contents) VALUES(?,?,?)")
        .bind(id).bind("snapshot-state.txt").bind(b"saved project state".as_slice())
        .execute(store.pool()).await.unwrap();
    let state_node = json_body(send(&app, Method::POST, &format!("/v3/projects/{id}/nodes"),
        Body::from(r#"{"name":"state node","node_type":"vpcs"}"#), Some("application/json")).await).await;
    let state_node_id = state_node["node_id"].as_str().unwrap();
    sqlx::query("INSERT INTO node_files(project_id,node_id,file_path,contents) VALUES(?,?,?,?)")
        .bind(id).bind(state_node_id).bind("startup.vpc").bind(b"saved node config".as_slice())
        .execute(store.pool()).await.unwrap();
    let snapshot = json_body(
        send(
            &app,
            Method::POST,
            &format!("/v3/projects/{id}/snapshots"),
            Body::from(r#"{"name":"initial","description":"test"}"#),
            Some("application/json"),
        )
        .await,
    )
    .await;
    assert!(snapshot["created"].as_str().is_some_and(|value| value.contains('T')));
    let snapshot_id = snapshot["snapshot_id"].as_str().unwrap();
    assert_eq!(
        json_body(
            send(
                &app,
                Method::GET,
                &format!("/v3/projects/{id}/snapshots"),
                Body::empty(),
                None
            )
            .await
        )
        .await
        .as_array()
        .unwrap()
        .len(),
        1
    );
    assert_eq!(
        send(
            &app,
            Method::POST,
            &format!("/v3/projects/{id}/snapshots/{snapshot_id}/restore"),
            Body::empty(),
            None
        )
        .await
        .status(),
        StatusCode::OK
    );
    let restored_project_file: Vec<u8> = sqlx::query_scalar("SELECT contents FROM project_files WHERE project_id=? AND file_path='snapshot-state.txt'")
        .bind(id).fetch_one(store.pool()).await.unwrap();
    let restored_node_file: Vec<u8> = sqlx::query_scalar("SELECT contents FROM node_files WHERE project_id=? AND node_id=? AND file_path='startup.vpc'")
        .bind(id).bind(state_node_id).fetch_one(store.pool()).await.unwrap();
    assert_eq!(restored_project_file, b"saved project state");
    assert_eq!(restored_node_file, b"saved node config");
    assert_eq!(
        send(
            &app,
            Method::DELETE,
            &format!("/v3/projects/{id}/snapshots/{snapshot_id}"),
            Body::empty(),
            None
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(
            &app,
            Method::POST,
            &format!("/v3/projects/{id}/nodes/start"),
            Body::empty(),
            None
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(
            &app,
            Method::POST,
            &format!("/v3/projects/{id}/nodes/stop"),
            Body::empty(),
            None
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(
            &app,
            Method::POST,
            &format!("/v3/projects/{id}/nodes/suspend"),
            Body::empty(),
            None
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(
            &app,
            Method::POST,
            &format!("/v3/projects/{id}/nodes/reload"),
            Body::empty(),
            None
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        send(
            &app,
            Method::POST,
            &format!("/v3/projects/{id}/nodes/console/reset"),
            Body::empty(),
            None
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    let file_response = send(
        &app,
        Method::POST,
        &format!("/v3/projects/{id}/files/readme.txt"),
        Body::from("lab"),
        None,
    )
    .await;
    assert_eq!(file_response.status(), StatusCode::NO_CONTENT);
    let file = send(
        &app,
        Method::GET,
        &format!("/v3/projects/{id}/files/readme.txt"),
        Body::empty(),
        None,
    )
    .await
    .into_body()
    .collect()
    .await
    .unwrap()
    .to_bytes();
    assert_eq!(&file[..], b"lab");
    assert_eq!(
        send(
            &app,
            Method::GET,
            &format!("/v3/projects/{id}/gns3file"),
            Body::empty(),
            None
        )
        .await
        .status(),
        StatusCode::OK
    );
    let archive = send(
        &app,
        Method::GET,
        &format!("/v3/projects/{id}/export"),
        Body::empty(),
        None,
    )
    .await
    .into_body()
    .collect()
    .await
    .unwrap()
    .to_bytes();
    assert_eq!(&archive[..2], b"PK");
    let new_id = uuid::Uuid::new_v4();
    let response = send(
        &app,
        Method::POST,
        &format!("/v3/projects/{new_id}/import?name=Imported"),
        Body::from(archive),
        Some("application/gns3project"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(json_body(response).await["name"], "Imported");
    let imported_file = send(
        &app,
        Method::GET,
        &format!("/v3/projects/{new_id}/files/readme.txt"),
        Body::empty(),
        None,
    )
    .await
    .into_body()
    .collect()
    .await
    .unwrap()
    .to_bytes();
    assert_eq!(&imported_file[..], b"lab");
    let response = send(
        &app,
        Method::POST,
        &format!("/v3/projects/{id}/duplicate"),
        Body::from(r#"{"name":"Cloned lab"}"#),
        Some("application/json"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(json_body(response).await["name"], "Cloned lab");
}
