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
        assert_eq!(reset.status(), StatusCode::NOT_IMPLEMENTED);
        assert_eq!(
            json_body(reset).await["message"],
            "Resetting runtime consoles is not supported"
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
        let duplicate_project_node = sqlx::query(
            "SELECT object_json FROM topology_objects WHERE project_id=? AND kind='node'",
        )
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
    async fn project_stats_lock_snapshots_files_export_import_and_duplicate_work_over_http() {
        let (app, _dir, _store) = app().await;
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
            StatusCode::NOT_IMPLEMENTED
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
