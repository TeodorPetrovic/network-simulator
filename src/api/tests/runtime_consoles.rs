use super::*;

#[tokio::test]
async fn docker_console_attaches_to_the_running_container_tty_over_websocket() {
    use futures_util::{SinkExt, StreamExt};
    use tokio::{
        net::TcpListener,
        time::{Duration, timeout},
    };
    use tokio_tungstenite::{connect_async, tungstenite::Message};

    let (app, _directory) = test_router().await;
    let (_, project) = request(
        &app,
        Method::POST,
        "/v3/projects".into(),
        Some(serde_json::json!({ "name": "Docker console lab" })),
    )
    .await;
    let project_id = project["project_id"].as_str().unwrap();
    let (status, node) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes"),
        Some(serde_json::json!({
            "name": "Interactive node",
            "node_type": "docker",
            "image": "alpine:latest"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let node_id = node["node_id"].as_str().unwrap();
    let console_path =
        format!("/v3/compute/projects/{project_id}/docker/nodes/{node_id}/console/ws");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("temporary HTTP listener binds");
    let address = listener.local_addr().unwrap();
    let server_app = app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, server_app).await.expect("test HTTP server runs");
    });
    let (mut stopped_socket, _) = connect_async(format!("ws://{address}{console_path}"))
        .await.expect("stopped-node websocket upgrade succeeds to return an error frame");
    let stopped_message = timeout(Duration::from_secs(2), stopped_socket.next())
        .await.expect("stopped-node error arrives")
        .expect("stopped-node socket sends a message")
        .expect("stopped-node websocket frame is valid");
    assert_eq!(stopped_message.into_text().unwrap(), "node is not started");
    stopped_socket.close(None).await.expect("stopped-node websocket closes");

    let (status, _) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes/{node_id}/start"),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (mut socket, _) = connect_async(format!("ws://{address}{console_path}"))
        .await
        .expect("Docker console websocket connects");
    let initial = timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("Docker attach produces initial terminal output")
        .expect("Docker console remains open")
        .expect("Docker console output is valid")
        .into_data();
    assert!(String::from_utf8_lossy(&initial).contains("DOCKER-ATTACHED"));

    socket
        .send(Message::Text("show interfaces\r\n".into()))
        .await
        .expect("terminal input reaches the attached process");
    let echoed = timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("attached process echoes terminal input")
        .expect("Docker console remains open")
        .expect("Docker console input echo is valid")
        .into_data();
    assert!(String::from_utf8_lossy(&echoed).contains("show interfaces"));
    socket
        .close(None)
        .await
        .expect("Docker console closes cleanly");
    server.abort();
}

#[tokio::test]
async fn qemu_templates_create_private_disks_and_exercise_qmp_process_lifecycle() {
    let (app, directory) = test_router().await;
    create_custom_from_runtime(&app, "qemu", "Test Router").await;
    let (status, templates) = request(&app, Method::GET, "/v3/templates".into(), None).await;
    assert_eq!(status, StatusCode::OK);
    let template = templates
        .as_array()
        .unwrap()
        .iter()
        .find(|template| template["template_type"] == "qemu")
        .unwrap_or_else(|| {
            panic!("custom QEMU appliance did not create a palette template: {templates:#}")
        });
    let template_id = template["template_id"].as_str().unwrap();
    assert_eq!(template["name"], "Test Router · Default");

    let (_, project) = request(
        &app,
        Method::POST,
        "/v3/projects".into(),
        Some(serde_json::json!({ "name": "QEMU lifecycle lab" })),
    )
    .await;
    let project_id = project["project_id"].as_str().unwrap();
    let mut qemu_nodes = Vec::new();
    for name in ["Router A"] {
        let (status, node) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/templates/{template_id}"),
            Some(serde_json::json!({ "name": name })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(node["node_type"], "qemu");
        assert!(std::path::Path::new(node["runtime_overlay"].as_str().unwrap()).is_file());
        assert_eq!(node["status"], "stopped");
        qemu_nodes.push(node);
    }
    let node_a = qemu_nodes[0]["node_id"].as_str().unwrap();
    let (status, configured) = request(
        &app,
        Method::PUT,
        format!("/v3/projects/{project_id}/nodes/{node_a}/config"),
        Some(serde_json::json!({
            "memory": 1024,
            "cpus": 2,
            "adapters": 2,
            "adapter_type": "virtio-net-pci",
            "hda_disk_interface": "virtio",
            "boot_priority": "dc"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(configured["memory"], 1024);
    assert_eq!(configured["cpus"], 2);
    assert_eq!(configured["adapter_type"], "virtio-net-pci");
    assert_eq!(configured["hda_disk_interface"], "virtio");
    assert_eq!(configured["boot_priority"], "dc");

    let console_path = format!(
        "/v3/compute/projects/{project_id}/qemu/nodes/{}/console/ws",
        qemu_nodes[0]["node_id"].as_str().unwrap()
    );
    assert_eq!(
        websocket_status(&app, console_path.clone()).await,
        StatusCode::SWITCHING_PROTOCOLS
    );

    let (status, node) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes/{node_a}/start"),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(node["status"], "started");
    let (status, error) = request(
        &app,
        Method::PUT,
        format!("/v3/projects/{project_id}/nodes/{node_a}/config"),
        Some(serde_json::json!({ "memory": 2048 })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("Stop this device")
    );
    assert_eq!(
        websocket_status(&app, console_path).await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    let (status, node) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes/{node_a}/suspend"),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(node["status"], "suspended");
    let (status, node) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes/{node_a}/start"),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(node["status"], "started");
    let (status, _) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes/{node_a}/reload"),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let args = std::fs::read_to_string(directory.path().join("qemu-arguments.log"))
        .expect("QEMU launch arguments were recorded");
    assert!(args.contains("socket,id=nsboot0,udp=127.0.0.1:"));
    assert!(args.contains(",localaddr=127.0.0.1:"));
    assert!(args.contains("-device virtio-net-pci,id=nsdev_boot1,netdev=nsboot1"));
    assert!(args.contains("-chardev socket,id=serial0,path="));
    assert!(args.contains("server=on,wait=off -serial chardev:serial0"));
    assert!(args.contains("-m 1024 -smp cpus=2,maxcpus=2,sockets=1 -boot order=dc"));
    assert!(args.contains("if=virtio,index=0,media=disk,id=drive0,format=qcow2"));
    for node in &qemu_nodes {
        let node_id = node["node_id"].as_str().unwrap();
        let (status, node) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes/{node_id}/stop"),
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(node["status"], "stopped");
    }
    for node in &qemu_nodes {
        assert!(std::path::Path::new(node["runtime_overlay"].as_str().unwrap()).is_file());
    }
    let (status, _) = request(
        &app,
        Method::DELETE,
        format!("/v3/projects/{project_id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    for node in qemu_nodes {
        assert!(!std::path::Path::new(node["runtime_overlay"].as_str().unwrap()).exists());
    }
}

#[tokio::test]
async fn qemu_image_upload_is_installed_in_the_shared_runtime_library() {
    let (app, directory) = test_router().await;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v3/qemu/images/custom-router.qcow2")
                .header("content-type", "application/octet-stream")
                .body(Body::from("test disk bytes"))
                .expect("QEMU upload request builds"),
        )
        .await
        .expect("QEMU image upload response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("QEMU upload response body")
        .to_bytes();
    let uploaded: Value = serde_json::from_slice(&body).expect("QEMU upload response JSON");
    assert_eq!(uploaded["name"], "custom-router.qcow2");
    assert_eq!(uploaded["template_type"], "qemu");
    assert_eq!(uploaded["bytes"], 15);
    let image_path = std::path::PathBuf::from(uploaded["image"].as_str().unwrap());
    assert!(image_path.starts_with(directory.path().join("qemu-images")));
    assert_eq!(std::fs::read(&image_path).unwrap(), b"test disk bytes");

    let (status, templates) = request(&app, Method::GET, "/v3/templates".into(), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        templates
            .as_array()
            .unwrap()
            .iter()
            .all(|template| template["template_type"] != "qemu"),
        "an uploaded disk image must not create a palette template: {templates:#}"
    );
    let (_, manager) = request(
        &app,
        Method::GET,
        "/v3/marketplace/gns3/manager".into(),
        None,
    )
    .await;
    let source_template_id = manager["custom_image_options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|template| template["hda_disk_image"] == uploaded["image"])
        .unwrap()["template_id"]
        .as_str()
        .unwrap();
    let (status, _) = request(
        &app,
        Method::POST,
        "/v3/marketplace/gns3/custom".into(),
        Some(serde_json::json!({
            "name": "Uploaded custom disk",
            "source_template_id": source_template_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, templates) = request(&app, Method::GET, "/v3/templates".into(), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        templates
            .as_array()
            .unwrap()
            .iter()
            .any(|template| template["hda_disk_image"] == uploaded["image"])
    );
}

#[test]
fn parses_docker_console_resize_control_frames() {
    assert_eq!(
        parse_docker_resize_control(r#"{"type":"resize","rows":40,"columns":120}"#),
        Some((40, 120))
    );
    assert_eq!(
        parse_docker_resize_control(r#"{"type":"resize","rows":0,"columns":120}"#),
        None
    );
    assert_eq!(
        parse_docker_resize_control(r#"{"type":"input","rows":40,"columns":120}"#),
        None
    );
    assert_eq!(parse_docker_resize_control("show version"), None);
}

#[test]
fn capture_paths_are_canonicalized_inside_the_project_directory() {
    let directory = TempDir::new().expect("temporary capture root");
    let project_dir = directory.path().join("project");
    std::fs::create_dir_all(&project_dir).expect("create project capture directory");
    let capture = project_dir.join("traffic.pcap");
    std::fs::write(&capture, b"pcap").expect("write capture");
    let outside = directory.path().join("outside.pcap");
    std::fs::write(&outside, b"private").expect("write outside file");
    assert_eq!(
        canonical_capture_file_path_in(
            &project_dir,
            "traffic.pcap",
            Some(capture.to_str().unwrap()),
            true,
        )
        .unwrap(),
        capture.canonicalize().unwrap()
    );
    assert!(
        canonical_capture_file_path_in(
            &project_dir,
            "traffic.pcap",
            Some(outside.to_str().unwrap()),
            true,
        )
        .is_err()
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, project_dir.join("linked.pcap"))
            .expect("create capture symlink");
        assert!(
            canonical_capture_file_path_in(
                &project_dir,
                "linked.pcap",
                Some(project_dir.join("linked.pcap").to_str().unwrap()),
                true,
            )
            .is_err()
        );
    }
    assert!(safe_capture_file_name("x\nhypervisor stop\n").is_err());
}

#[test]
fn frequency_drop_is_normalized_and_filters_select_one_relay_endpoint() {
    let filters = validate_link_filters(&serde_json::json!({ "frequency_drop": [3] }))
        .expect("frequency_drop is supported");
    assert_eq!(filters["frequency_drop"], serde_json::json!([3]));
    assert!(validate_link_filters(&serde_json::json!({ "bpf": ["ip\ntcp"] })).is_err());

    let link = serde_json::json!({
        "filters": { "packet_loss": [10] },
        "nodes": [
            { "runtime_endpoint_id": "first" },
            { "runtime_endpoint_id": "second" }
        ]
    });
    assert_eq!(
        link_filters_for_endpoint(&link, &link["nodes"][0]),
        serde_json::json!({ "packet_loss": [10] })
    );
    assert_eq!(
        link_filters_for_endpoint(&link, &link["nodes"][1]),
        serde_json::json!({})
    );
}

#[tokio::test]
async fn failed_relay_attach_runs_runtime_nio_cleanup() {
    let attach_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let cleanup_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let cleanup = cleanup_count.clone();
    let result = attach_runtime_nio_with_rollback(
        true,
        || {
            let attach_count = attach_count.clone();
            async move {
                attach_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        },
        || async { Err(ApiError::conflict("uBridge attach failed")) },
        || async move {
            cleanup.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        },
    )
    .await;

    assert!(result.is_err());
    assert_eq!(attach_count.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(cleanup_count.load(std::sync::atomic::Ordering::SeqCst), 1);

    let cleanup = cleanup_count.clone();
    let result = attach_runtime_nio_with_rollback(
        false,
        || async { Ok(()) },
        || async { Err(ApiError::conflict("uBridge update failed")) },
        || async move {
            cleanup.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        },
    )
    .await;
    assert!(result.is_err());
    assert_eq!(cleanup_count.load(std::sync::atomic::Ordering::SeqCst), 1);
}
