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
async fn controller_statistics_reports_live_node_link_and_capture_counts() {
    let (app, _directory, store) = app().await;
    let project = json_body(
        send(
            &app,
            Method::POST,
            "/v3/projects",
            Body::from(r#"{"name":"Statistics lab"}"#),
            Some("application/json"),
        )
        .await,
    )
    .await;
    let project_id = project["project_id"].as_str().unwrap();
    let started_id = Uuid::new_v4().to_string();
    let stopped_id = Uuid::new_v4().to_string();
    let link_id = Uuid::new_v4().to_string();
    for (id, node_type, status) in [
        (&started_id, "qemu", "started"),
        (&stopped_id, "docker", "stopped"),
    ] {
        let object = json!({
            "node_id":id,
            "project_id":project_id,
            "node_type":node_type,
            "status":status,
            "console_host":"192.0.2.10"
        });
        sqlx::query("INSERT INTO topology_objects(project_id,kind,object_id,name,object_json) VALUES(?, 'node', ?, ?, ?)")
                .bind(project_id)
                .bind(id)
                .bind(node_type)
                .bind(object.to_string())
                .execute(store.pool())
                .await
                .unwrap();
    }
    let link = json!({
        "link_id":link_id,
        "project_id":project_id,
        "nodes":[],
        "capturing":true,
        "wireshark":true
    });
    sqlx::query("INSERT INTO topology_objects(project_id,kind,object_id,object_json) VALUES(?, 'link', ?, ?)")
            .bind(project_id)
            .bind(&link_id)
            .bind(link.to_string())
            .execute(store.pool())
            .await
            .unwrap();

    let stats =
        json_body(send(&app, Method::GET, "/v3/statistics", Body::empty(), None).await).await;
    assert!(stats["uptime_seconds"].as_u64().is_some());
    assert_eq!(stats["nodes"]["total"], 2);
    assert_eq!(stats["nodes"]["by_type"]["qemu"], 1);
    assert_eq!(stats["nodes"]["by_type"]["docker"], 1);
    assert_eq!(stats["nodes"]["by_status"]["started"], 1);
    assert_eq!(stats["nodes"]["by_status"]["stopped"], 1);
    assert_eq!(stats["links"]["total"], 1);
    assert_eq!(stats["links"]["capturing"], 1);
    assert_eq!(stats["webwireshark"]["containers"], 1);

    let reset = send(
        &app,
        Method::POST,
        &format!("/v3/projects/{project_id}/nodes/console/reset"),
        Body::empty(),
        None,
    )
    .await;
    assert_eq!(reset.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_body(reset).await["message"],
        "Console reset is unavailable because this runtime has no console listener reset hook"
    );
    for id in [&started_id, &stopped_id] {
        let row = sqlx::query("SELECT object_json FROM topology_objects WHERE project_id=? AND kind='node' AND object_id=?")
                .bind(project_id)
                .bind(id)
                .fetch_one(store.pool())
                .await
                .unwrap();
        let node: Value =
            serde_json::from_str(row.get::<String, _>("object_json").as_str()).unwrap();
        assert_eq!(node["console_host"], "192.0.2.10");
    }
}

#[tokio::test]
async fn imported_and_duplicated_runtime_nodes_cannot_remove_source_files() {
    let (app, directory, store) = app().await;
    let runtime_dir = directory.path().join("runtime-source");
    tokio::fs::create_dir_all(&runtime_dir).await.unwrap();
    let runtime_paths = [
        "overlay.qcow2",
        "qmp.sock",
        "serial.sock",
        "vm.pid",
        "vm.log",
        "disk.qcow2",
        "legacy.pid",
    ]
    .into_iter()
    .map(|name| (name, runtime_dir.join(name)))
    .collect::<Vec<_>>();
    for (name, path) in &runtime_paths {
        tokio::fs::write(path, format!("source runtime: {name}"))
            .await
            .unwrap();
    }
    let source_node_id = Uuid::new_v4();
    let source_project_id = Uuid::new_v4();
    let source_project = json!({
        "project_id":source_project_id,
        "name":"Source lab",
        "status":"opened",
        "nodes":[{
            "node_id":source_node_id,
            "node_type":"qemu",
            "status":"started",
            "runtime_overlay":runtime_paths[0].1,
            "runtime_working_dir":runtime_dir.display().to_string(),
            "container_id":"0123456789abcdef",
            "console":5001
        }]
    });
    sqlx::query("INSERT INTO projects(project_id,name,status,project_json) VALUES(?,?,?,?)")
        .bind(source_project_id.to_string())
        .bind("Source lab")
        .bind("opened")
        .bind(source_project.to_string())
        .execute(store.pool())
        .await
        .unwrap();
    let source_node = json!({
        "node_id":source_node_id,
        "project_id":source_project_id,
        "node_type":"qemu",
        "name":"Source router",
        "status":"stopped",
        "hda_disk_image":"/images/router.qcow2",
        "runtime_overlay":runtime_paths[0].1,
        "runtime_qmp_socket":runtime_paths[1].1,
        "runtime_serial_socket":runtime_paths[2].1,
        "runtime_pid_file":runtime_paths[3].1,
        "runtime_log_file":runtime_paths[4].1,
        "runtime_hdb_overlay":runtime_paths[5].1,
        "runtime_working_dir":runtime_dir.display().to_string(),
        "runtime_hypervisor_port":7200,
        "container_id":"0123456789abcdef",
        "image_digest":"sha256:source",
        "dynamips_id":1,
        "qemu_pid":1234,
        "pid_file":runtime_paths[6].1,
        "console":5001
    });
    sqlx::query("INSERT INTO topology_objects(project_id,kind,object_id,name,object_json) VALUES(?, 'node', ?, ?, ?)")
            .bind(source_project_id.to_string())
            .bind(source_node_id.to_string())
            .bind("Source router")
            .bind(source_node.to_string())
            .execute(store.pool())
            .await
            .unwrap();
    sqlx::query("INSERT INTO node_files(project_id,node_id,file_path,contents) VALUES(?,?,?,?)")
        .bind(source_project_id.to_string()).bind(source_node_id.to_string())
        .bind("nvram.bin").bind(b"persistent disk state".as_slice())
        .execute(store.pool()).await.unwrap();

    let duplicate_project = json_body(
        send(
            &app,
            Method::POST,
            &format!("/v3/projects/{source_project_id}/duplicate"),
            Body::from("{}"),
            Some("application/json"),
        )
        .await,
    )
    .await;
    let duplicate_project_id = duplicate_project["project_id"].as_str().unwrap();
    assert_node_has_no_runtime_state(&duplicate_project["nodes"][0]);
    let duplicate_project_node =
        sqlx::query("SELECT object_json FROM topology_objects WHERE project_id=? AND kind='node'")
            .bind(duplicate_project_id)
            .fetch_one(store.pool())
            .await
            .unwrap();
    let duplicate_project_node: Value = serde_json::from_str(
        duplicate_project_node
            .get::<String, _>("object_json")
            .as_str(),
    )
    .unwrap();
    assert_node_has_no_runtime_state(&duplicate_project_node);
    let project_copy_file: Vec<u8> = sqlx::query_scalar("SELECT contents FROM node_files WHERE project_id=? AND file_path='nvram.bin'")
        .bind(duplicate_project_id).fetch_one(store.pool()).await.unwrap();
    assert_eq!(project_copy_file, b"persistent disk state");
    assert_eq!(
        send(
            &app,
            Method::DELETE,
            &format!("/v3/projects/{duplicate_project_id}"),
            Body::empty(),
            None,
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );

    let duplicate_node = json_body(
        send(
            &app,
            Method::POST,
            &format!("/v3/projects/{source_project_id}/nodes/{source_node_id}/duplicate"),
            Body::from("{}"),
            Some("application/json"),
        )
        .await,
    )
    .await;
    assert_node_has_no_runtime_state(&duplicate_node);
    let duplicate_node_id = duplicate_node["node_id"].as_str().unwrap();
    let node_copy_file: Vec<u8> = sqlx::query_scalar("SELECT contents FROM node_files WHERE project_id=? AND node_id=? AND file_path='nvram.bin'")
        .bind(source_project_id.to_string()).bind(duplicate_node_id).fetch_one(store.pool()).await.unwrap();
    assert_eq!(node_copy_file, b"persistent disk state");
    assert_eq!(
        send(
            &app,
            Method::DELETE,
            &format!("/v3/projects/{source_project_id}/nodes/{duplicate_node_id}"),
            Body::empty(),
            None,
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    for (name, path) in &runtime_paths {
        assert_eq!(
            tokio::fs::read(path).await.unwrap(),
            format!("source runtime: {name}").as_bytes(),
            "duplicate deletion changed {name}"
        );
    }
    let source_node: Value = sqlx::query_scalar::<_, String>(
            "SELECT object_json FROM topology_objects WHERE project_id=? AND kind='node' AND object_id=?",
        )
        .bind(source_project_id.to_string())
        .bind(source_node_id.to_string())
        .fetch_one(store.pool())
        .await
        .map(|json| serde_json::from_str(&json).unwrap())
        .unwrap();
    assert_eq!(
        source_node["runtime_overlay"],
        runtime_paths[0].1.to_string_lossy().as_ref()
    );
    assert_eq!(source_node["container_id"], "0123456789abcdef");
    assert_eq!(source_node["dynamips_id"], 1);

    let import_node_id = Uuid::new_v4();
    let unsafe_project_path = directory.path().join("unsafe-project.gns3");
    let unsafe_project = json!({
        "name":"Imported unsafe topology",
        "nodes":[{
            "node_id":import_node_id,
            // Keep this fixture independent of a QEMU disk image: this
            // test exercises imported runtime fields, not provisioning.
            "node_type":"unavailable_qemu_fixture",
            "name":"Imported router",
            "status":"started",
            "hda_disk_image":"/images/router.qcow2",
            "runtime_overlay":runtime_paths[0].1,
            "runtime_qmp_socket":runtime_paths[1].1,
            "runtime_serial_socket":runtime_paths[2].1,
            "runtime_pid_file":runtime_paths[3].1,
            "runtime_log_file":runtime_paths[4].1,
            "runtime_hdb_overlay":runtime_paths[5].1,
            "runtime_working_dir":runtime_dir.display().to_string(),
            "runtime_hypervisor_port":7200,
            "container_id":"0123456789abcdef",
            "image_digest":"sha256:source",
            "dynamips_id":1,
            "qemu_pid":1234,
            "pid_file":runtime_paths[6].1
        }],
        "links":[],
        "drawings":[]
    });
    tokio::fs::write(&unsafe_project_path, unsafe_project.to_string())
        .await
        .unwrap();
    let loaded = json_body(
        send(
            &app,
            Method::POST,
            "/v3/projects/load",
            Body::from(json!({"path":unsafe_project_path}).to_string()),
            Some("application/json"),
        )
        .await,
    )
    .await;
    assert_eq!(
        loaded["nodes"][0]["status"], "stopped",
        "loaded project: {loaded}"
    );
    assert_node_has_no_runtime_state(&loaded["nodes"][0]);
    let loaded_project_id = loaded["project_id"].as_str().unwrap();
    let loaded_node: Value = sqlx::query_scalar::<_, String>(
        "SELECT object_json FROM topology_objects WHERE project_id=? AND kind='node'",
    )
    .bind(loaded_project_id)
    .fetch_one(store.pool())
    .await
    .map(|json| serde_json::from_str(&json).unwrap())
    .unwrap();
    assert_node_has_no_runtime_state(&loaded_node);
    assert_eq!(
        send(
            &app,
            Method::DELETE,
            &format!("/v3/projects/{loaded_project_id}"),
            Body::empty(),
            None,
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    for (_, path) in &runtime_paths {
        assert!(path.exists(), "import deletion removed {}", path.display());
    }
}

#[tokio::test]
async fn load_project_rejects_non_regular_and_oversized_files() {
    let (app, directory, _store) = app().await;
    let non_regular = send(
        &app,
        Method::POST,
        "/v3/projects/load",
        Body::from(json!({"path":directory.path().to_string_lossy()}).to_string()),
        Some("application/json"),
    )
    .await;
    assert_eq!(non_regular.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        json_body(non_regular).await["message"]
            .as_str()
            .unwrap()
            .contains("regular file")
    );

    let oversized_path = directory.path().join("oversized.gns3");
    let oversized = std::fs::File::create(&oversized_path).unwrap();
    oversized.set_len(64 * 1024 * 1024 + 1).unwrap();
    let too_large = send(
        &app,
        Method::POST,
        "/v3/projects/load",
        Body::from(json!({"path":oversized_path.to_string_lossy()}).to_string()),
        Some("application/json"),
    )
    .await;
    assert_eq!(too_large.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn malformed_zip_import_returns_http_400() {
    let (app, _directory, _store) = app().await;
    let mut malformed = vec![0_u8; 30];
    malformed[..4].copy_from_slice(b"PK\x03\x04");
    malformed[26..28].copy_from_slice(&u16::MAX.to_le_bytes());
    let response = send(
        &app,
        Method::POST,
        &format!("/v3/projects/{}/import", Uuid::new_v4()),
        Body::from(malformed),
        Some("application/gns3project"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
