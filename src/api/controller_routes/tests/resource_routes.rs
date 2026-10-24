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
    async fn settings_computes_symbols_and_node_file_actions_are_http_accessible() {
        let (app, _dir, store) = app().await;
        let response = send(
            &app,
            Method::PUT,
            "/v3/settings",
            Body::from(r#"{"Server":{"console_start_port_range":5000}}"#),
            Some("application/json"),
        )
        .await;
        assert_eq!(
            json_body(response).await["Server"]["console_start_port_range"],
            5000
        );
        assert_eq!(
            json_body(send(&app, Method::GET, "/v3/settings", Body::empty(), None).await).await["Server"]
                ["console_start_port_range"],
            5000
        );
        let computes =
            json_body(send(&app, Method::GET, "/v3/computes", Body::empty(), None).await).await;
        assert_eq!(computes[0]["compute_id"], "local");
        assert_eq!(computes[0]["host"], "127.0.0.1");
        assert_eq!(computes[0]["port"], 3182);
        assert_eq!(
            json_body(send(&app, Method::GET, "/v3/computes/local", Body::empty(), None).await)
                .await["port"],
            3182
        );
        assert!(
            json_body(
                send(
                    &app,
                    Method::GET,
                    "/v3/symbols/default_symbols",
                    Body::empty(),
                    None
                )
                .await
            )
            .await
            .is_object()
        );
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&320u32.to_be_bytes());
        png.extend_from_slice(&200u32.to_be_bytes());
        assert_eq!(
            send(
                &app,
                Method::POST,
                "/v3/symbols/custom.png/raw",
                Body::from(png.clone()),
                Some("application/octet-stream")
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            json_body(
                send(
                    &app,
                    Method::GET,
                    "/v3/symbols/custom.png/dimensions",
                    Body::empty(),
                    None
                )
                .await
            )
            .await,
            json!({"width":320,"height":200})
        );
        let raw_symbol = send(
            &app,
            Method::GET,
            "/v3/symbols/custom.png/raw",
            Body::empty(),
            None,
        )
        .await;
        assert_eq!(
            raw_symbol.headers()["content-disposition"],
            "attachment; filename=\"custom.png\""
        );
        assert!(
            raw_symbol.headers()["content-security-policy"]
                .to_str()
                .unwrap()
                .contains("sandbox")
        );
        assert_eq!(raw_symbol.headers()["x-content-type-options"], "nosniff");
        assert_eq!(
            raw_symbol.into_body().collect().await.unwrap().to_bytes()[..],
            png[..]
        );
        assert_eq!(
            send(
                &app,
                Method::DELETE,
                "/v3/symbols/custom.png",
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        assert!(
            json_body(send(&app, Method::GET, "/v3/symbols", Body::empty(), None).await)
                .await
                .as_array()
                .unwrap()
                .is_empty()
        );
        let active_svg =
            br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#;
        assert_eq!(
            send(
                &app,
                Method::POST,
                "/v3/symbols/active.svg/raw",
                Body::from(active_svg.as_slice()),
                Some("image/svg+xml"),
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        let svg_response = send(
            &app,
            Method::GET,
            "/v3/symbols/active.svg/raw",
            Body::empty(),
            None,
        )
        .await;
        assert_eq!(svg_response.headers()["content-type"], "image/svg+xml");
        assert!(
            svg_response.headers()["content-disposition"]
                .to_str()
                .unwrap()
                .starts_with("attachment;")
        );
        assert!(
            svg_response.headers()["content-security-policy"]
                .to_str()
                .unwrap()
                .contains("sandbox")
        );
        assert_eq!(svg_response.headers()["x-content-type-options"], "nosniff");
        assert_eq!(
            svg_response.into_body().collect().await.unwrap().to_bytes()[..],
            active_svg[..]
        );
        assert_eq!(
            send(
                &app,
                Method::DELETE,
                "/v3/symbols/active.svg",
                Body::empty(),
                None,
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            send(&app, Method::GET, "/v3/statistics", Body::empty(), None)
                .await
                .status(),
            StatusCode::OK
        );
        let notifications = send(&app, Method::GET, "/v3/notifications", Body::empty(), None).await;
        assert_eq!(notifications.status(), StatusCode::OK);
        assert_eq!(
            notifications.headers()["content-type"],
            "application/x-ndjson"
        );
        let images = send(&app, Method::GET, "/v3/images", Body::empty(), None).await;
        assert_eq!(images.status(), StatusCode::OK);
        assert!(json_body(images).await.is_array());
        assert_eq!(
            send(
                &app,
                Method::GET,
                "/v3/images?image_type=bogus",
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            send(
                &app,
                Method::GET,
                "/v3/images/no-such-image",
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send(
                &app,
                Method::DELETE,
                "/v3/images/no-such-image",
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            send(
                &app,
                Method::POST,
                "/v3/images/upload/empty.qcow2",
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        let project_id = uuid::Uuid::new_v4();
        let node_id = uuid::Uuid::new_v4();
        let create = send(
            &app,
            Method::POST,
            "/v3/projects",
            Body::from(format!(
                r#"{{"project_id":"{project_id}","name":"node files"}}"#
            )),
            Some("application/json"),
        )
        .await;
        assert_eq!(create.status(), StatusCode::CREATED);
        let node = json!({"node_id":node_id,"project_id":project_id,"name":"host","node_type":"vpcs","status":"stopped","x":0,"y":0,"locked":false});
        sqlx::query("INSERT INTO topology_objects(project_id,kind,object_id,name,object_json) VALUES(?,?,?,?,?)")
            .bind(project_id.to_string()).bind("node").bind(node_id.to_string()).bind("host").bind(node.to_string()).execute(store.pool()).await.unwrap();
        assert_eq!(
            json_body(
                send(
                    &app,
                    Method::GET,
                    &format!("/v3/projects/{project_id}/nodes/{node_id}/files"),
                    Body::empty(),
                    None
                )
                .await
            )
            .await
            .as_array()
            .unwrap()
            .len(),
            0
        );
        assert_eq!(
            send(
                &app,
                Method::POST,
                &format!("/v3/projects/{project_id}/nodes/{node_id}/files/config/startup.cfg"),
                Body::from("hostname lab"),
                None
            )
            .await
            .status(),
            StatusCode::CREATED
        );
        assert_eq!(
            send(
                &app,
                Method::GET,
                &format!("/v3/projects/{project_id}/nodes/{node_id}/files/config/startup.cfg"),
                Body::empty(),
                None
            )
            .await
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()[..],
            b"hostname lab"[..]
        );
        assert_eq!(
            send(
                &app,
                Method::POST,
                &format!("/v3/projects/{project_id}/nodes/{node_id}/isolate"),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            json_body(
                send(
                    &app,
                    Method::GET,
                    &format!("/v3/projects/{project_id}/nodes/{node_id}"),
                    Body::empty(),
                    None
                )
                .await
            )
            .await["isolated"],
            true
        );
        assert_eq!(
            send(
                &app,
                Method::POST,
                &format!("/v3/projects/{project_id}/nodes/{node_id}/unisolate"),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            send(
                &app,
                Method::POST,
                &format!("/v3/projects/{project_id}/nodes/{node_id}/console/reset"),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::NOT_IMPLEMENTED
        );
        assert!(
            json_body(
                send(
                    &app,
                    Method::GET,
                    &format!("/v3/projects/{project_id}/nodes/{node_id}/links"),
                    Body::empty(),
                    None
                )
                .await
            )
            .await
            .as_array()
            .unwrap()
            .is_empty()
        );
        assert_eq!(
            send(
                &app,
                Method::GET,
                &format!("/v3/projects/{project_id}/nodes/{node_id}/dynamips/auto_idlepc"),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            send(
                &app,
                Method::GET,
                &format!("/v3/projects/{project_id}/nodes/{node_id}/dynamips/idlepc_proposals"),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
        for method in [Method::POST, Method::PUT, Method::DELETE] {
            assert_eq!(
                send(
                    &app,
                    method,
                    &format!(
                        "/v3/projects/{project_id}/nodes/{node_id}/qemu/disk_image/extra.qcow2"
                    ),
                    Body::from(r#"{"size":"1G"}"#),
                    Some("application/json")
                )
                .await
                .status(),
                StatusCode::UNPROCESSABLE_ENTITY
            );
        }
        let duplicate = send(
            &app,
            Method::POST,
            &format!("/v3/projects/{project_id}/nodes/{node_id}/duplicate"),
            Body::from(r#"{"x":50,"y":50,"z":1}"#),
            Some("application/json"),
        )
        .await;
        assert_eq!(duplicate.status(), StatusCode::CREATED);
        assert_eq!(
            send(
                &app,
                Method::DELETE,
                &format!("/v3/projects/{project_id}/nodes/{node_id}/files/config/startup.cfg"),
                Body::empty(),
                None
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
    }

    #[tokio::test]
