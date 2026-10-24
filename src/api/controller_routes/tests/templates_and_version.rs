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
        let state =
            AppState::new(store.clone()).with_bind_address("127.0.0.1:3182".parse().unwrap());
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
    async fn node_deletion_cleans_file_rows_and_database_trigger_covers_direct_deletes() {
        let (app, _directory, store) = app().await;
        let project = json_body(
            send(
                &app,
                Method::POST,
                "/v3/projects",
                Body::from(r#"{"name":"Node file cleanup"}"#),
                Some("application/json"),
            )
            .await,
        )
        .await;
        let project_id = project["project_id"].as_str().unwrap();
        let node_id = Uuid::new_v4();
        let node = json!({"node_id":node_id,"project_id":project_id,"node_type":"ethernet_switch","name":"Switch","status":"stopped"});
        sqlx::query("INSERT INTO topology_objects(project_id,kind,object_id,name,object_json) VALUES(?, 'node', ?, 'Switch', ?)")
            .bind(project_id)
            .bind(node_id.to_string())
            .bind(node.to_string())
            .execute(store.pool())
            .await
            .unwrap();
        assert_eq!(
            send(
                &app,
                Method::POST,
                &format!("/v3/projects/{project_id}/nodes/{node_id}/files/config.cfg"),
                Body::from("switch config"),
                None,
            )
            .await
            .status(),
            StatusCode::CREATED
        );
        assert_eq!(
            send(
                &app,
                Method::DELETE,
                &format!("/v3/projects/{project_id}/nodes/{node_id}"),
                Body::empty(),
                None,
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM node_files WHERE project_id=? AND node_id=?")
                .bind(project_id)
                .bind(node_id.to_string())
                .fetch_one(store.pool())
                .await
                .unwrap();
        assert_eq!(count, 0);

        let direct_delete_id = Uuid::new_v4();
        let direct_node = json!({"node_id":direct_delete_id,"project_id":project_id,"node_type":"ethernet_switch","name":"Direct switch"});
        sqlx::query("INSERT INTO topology_objects(project_id,kind,object_id,name,object_json) VALUES(?, 'node', ?, 'Direct switch', ?)")
            .bind(project_id)
            .bind(direct_delete_id.to_string())
            .bind(direct_node.to_string())
            .execute(store.pool())
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO node_files(project_id,node_id,file_path,contents) VALUES(?,?,?,?)",
        )
        .bind(project_id)
        .bind(direct_delete_id.to_string())
        .bind("startup.cfg")
        .bind(b"config".as_slice())
        .execute(store.pool())
        .await
        .unwrap();
        sqlx::query(
            "DELETE FROM topology_objects WHERE project_id=? AND kind='node' AND object_id=?",
        )
        .bind(project_id)
        .bind(direct_delete_id.to_string())
        .execute(store.pool())
        .await
        .unwrap();
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM node_files WHERE project_id=? AND node_id=?")
                .bind(project_id)
                .bind(direct_delete_id.to_string())
                .fetch_one(store.pool())
                .await
                .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn template_crud_and_duplicate_are_http_accessible() {
        let (app, _dir, _store) = app().await;
        let response = send(
            &app,
            Method::POST,
            "/v3/templates",
            Body::from(
                json!({"name":"Test VPCS","template_type":"vpcs","startup_script":""}).to_string(),
            ),
            Some("application/json"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let template = json_body(response).await;
        let id = template["template_id"].as_str().unwrap();
        assert_eq!(
            send(
                &app,
                Method::GET,
                &format!("/v3/templates/{id}"),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::OK
        );
        let response = send(
            &app,
            Method::PUT,
            &format!("/v3/templates/{id}"),
            Body::from(r#"{"name":"Updated VPCS"}"#),
            Some("application/json"),
        )
        .await;
        assert_eq!(json_body(response).await["name"], "Updated VPCS");
        let response = send(
            &app,
            Method::POST,
            &format!("/v3/templates/{id}/duplicate"),
            Body::empty(),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let duplicate = json_body(response).await;
        assert_eq!(duplicate["name"], "Updated VPCS copy");
        let response = send(&app, Method::GET, "/v3/templates", Body::empty(), None).await;
        assert!(
            json_body(response)
                .await
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["template_id"] == id)
        );
        assert_eq!(
            send(
                &app,
                Method::DELETE,
                &format!(
                    "/v3/templates/{}",
                    duplicate["template_id"].as_str().unwrap()
                ),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
    }

    #[tokio::test]
    async fn controller_version_matches_the_upstream_shape_and_check_route() {
        let (app, _dir, _store) = app().await;
        let response = send(&app, Method::GET, "/v3/version", Body::empty(), None).await;
        assert_eq!(
            json_body(response).await,
            json!({"controller_host":null,"version":"3.1.0.dev7","local":true})
        );
        let response = send(
            &app,
            Method::POST,
            "/v3/version",
            Body::from(r#"{"version":"3.1.0.dev7"}"#),
            Some("application/json"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(json_body(response).await, json!({"version":"3.1.0.dev7"}));
        assert_eq!(
            send(
                &app,
                Method::POST,
                "/v3/version",
                Body::from(r#"{"version":"different"}"#),
                Some("application/json"),
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
    }

    #[tokio::test]
