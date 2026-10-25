use super::*;

#[tokio::test]
async fn concurrent_node_and_topology_updates_are_serialized() {
    let (app, _directory) = test_router().await;
    let (_, project) = request(&app, Method::POST, "/v3/projects".into(),
        Some(serde_json::json!({"name":"Concurrent topology updates"}))).await;
    let project_id = project["project_id"].as_str().unwrap().to_owned();
    let mut node_ids = Vec::new();
    for index in 0..30 {
        let (status, node) = request(&app, Method::POST, format!("/v3/projects/{project_id}/nodes"),
            Some(serde_json::json!({"name":format!("N{index}"),"node_type":"ethernet_switch"}))).await;
        assert_eq!(status, StatusCode::CREATED);
        node_ids.push(node["node_id"].as_str().unwrap().to_owned());
    }

    let mut drawing_ids = Vec::new();
    for _ in 0..3 {
        let (status, drawing) = request(&app, Method::POST, format!("/v3/projects/{project_id}/drawings"),
            Some(serde_json::json!({"x":0,"y":0,"z":0,"rotation":0,"svg":"<svg></svg>"}))).await;
        assert_eq!(status, StatusCode::CREATED);
        drawing_ids.push(drawing["drawing_id"].as_str().unwrap().to_owned());
    }
    let mut writes: Vec<futures_util::future::BoxFuture<'static, (StatusCode, Value)>> = Vec::new();
    for (index, node_id) in node_ids.iter().enumerate() {
        let app = app.clone();
        let project_id = project_id.clone();
        let node_id = node_id.clone();
        writes.push(Box::pin(async move { request(&app, Method::PUT,
            format!("/v3/projects/{project_id}/nodes/{node_id}"),
            Some(serde_json::json!({"x":index as i64 * 17,"y":index as i64 * -11}))).await }));
    }
    for (index, drawing_id) in drawing_ids.into_iter().enumerate() {
        let app = app.clone();
        let project_id = project_id.clone();
        writes.push(Box::pin(async move { request(&app, Method::PUT,
            format!("/v3/projects/{project_id}/drawings/{drawing_id}"),
            Some(serde_json::json!({"x":index as i64 * 13,"y":index as i64 * 19}))).await }));
    }
    let results = futures_util::future::join_all(writes).await;
    assert!(results.iter().all(|(status, _)| *status == StatusCode::OK), "statuses: {:?}", results.iter().map(|(s,_)|s).collect::<Vec<_>>());
    let (status, nodes) = request(&app, Method::GET, format!("/v3/projects/{project_id}/nodes"), None).await;
    assert_eq!(status, StatusCode::OK);
    for (index, node_id) in node_ids.iter().enumerate() {
        let node = nodes.as_array().unwrap().iter().find(|node| node["node_id"] == *node_id).unwrap();
        assert_eq!(node["x"], index as i64 * 17);
        assert_eq!(node["y"], index as i64 * -11);
    }
}
use serde_json::json;

#[tokio::test]
async fn list_name_filters_match_exactly() {
    let (app, _directory) = test_router().await;
    for name in ["Exact", "Exact copy"] {
        let (status, _) = request(&app, Method::POST, "/v3/projects".into(), Some(json!({"name":name}))).await;
        assert_eq!(status, StatusCode::CREATED);
    }
    let (status, projects) = request(&app, Method::GET, "/v3/projects?name=Exact".into(), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(projects.as_array().unwrap().len(), 1);
    assert_eq!(projects[0]["name"], "Exact");
}

#[tokio::test]
async fn project_operation_locks_serialize_one_project_without_blocking_another() {
    let directory = TempDir::new().expect("temporary lock-test directory");
    let store = Store::connect(
        directory
            .path()
            .join("lock-test.sqlite")
            .to_str()
            .expect("UTF-8 database path"),
    )
    .await
    .expect("test database connects");
    let state = AppState::new(store);
    let first = state.lock_project_operations("project-a").await;
    let _other_project = state.lock_project_operations("project-b").await;
    assert!(
        timeout(
            std::time::Duration::from_millis(20),
            state.lock_project_operations("project-a")
        )
        .await
        .is_err()
    );
    drop(first);
    let _released = timeout(
        std::time::Duration::from_secs(1),
        state.lock_project_operations("project-a"),
    )
    .await
    .expect("same-project lock becomes available after release");
}

#[tokio::test]
async fn vpcs_base_script_file_is_loaded_from_project_files_and_inline_script_wins() {
    let directory = TempDir::new().expect("temporary test directory");
    let store = Store::connect(
        directory
            .path()
            .join("vpcs-config.sqlite")
            .to_str()
            .expect("UTF-8 database path"),
    )
    .await
    .expect("test database connects");
    let project_id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO projects(project_id, name, status, project_json) VALUES(?, ?, ?, ?)")
        .bind(&project_id)
        .bind("VPCS script lab")
        .bind("opened")
        .bind("{}")
        .execute(store.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO project_files(project_id, file_path, contents) VALUES(?, ?, ?)")
        .bind(&project_id)
        .bind("configs/base.vpc")
        .bind(b"ip 192.0.2.1/24\n".as_slice())
        .execute(store.pool())
        .await
        .unwrap();
    let node = serde_json::json!({
        "startup_script": "",
        "base_script_file": "configs/base.vpc"
    });
    assert!(matches!(
        vpcs_startup_script(store.pool(), &project_id, &node).await,
        Ok(Some(script)) if script == "ip 192.0.2.1/24\n"
    ));
    let inline = serde_json::json!({
        "startup_script": "ip 198.51.100.2/24",
        "base_script_file": "configs/base.vpc"
    });
    assert!(matches!(
        vpcs_startup_script(store.pool(), &project_id, &inline).await,
        Ok(Some(script)) if script == "ip 198.51.100.2/24"
    ));
}

#[tokio::test]
async fn static_spa_serves_direct_project_routes_and_keeps_v3_errors_as_404() {
    let (app, _directory) = test_router().await;
    let response = app
        .clone()
        .oneshot(
            Request::get("/projects/p1")
                .body(Body::empty())
                .expect("project route request builds"),
        )
        .await
        .expect("project route returns a response");
    assert_eq!(response.status(), StatusCode::OK);
    let shell = response
        .into_body()
        .collect()
        .await
        .expect("SPA shell reads")
        .to_bytes();
    let shell = String::from_utf8(shell.to_vec()).expect("SPA shell is UTF-8");
    assert!(shell.contains("Network Studio"));
    let asset = shell
        .split("href=\"/assets/")
        .nth(1)
        .and_then(|path| path.split('\"').next())
        .expect("shell references its generated stylesheet");
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/assets/{asset}"))
                .body(Body::empty())
                .expect("asset request builds"),
        )
        .await
        .expect("stylesheet returns a response");
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .oneshot(
            Request::get("/v3/not-a-real-route")
                .body(Body::empty())
                .expect("unknown API request builds"),
        )
        .await
        .expect("unknown API route returns a response");
    // Unclassified /v3 paths are denied before routing (deny-by-default auth
    // classification), so an unknown API path yields a JSON 403, never the SPA shell.
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("application/json"))
    );
}

#[tokio::test]
async fn v3_topology_flow_persists_snapshot_and_cascades_node_links() {
    let (app, _directory) = test_router().await;
    let (status, project) = request(
        &app,
        Method::POST,
        "/v3/projects".into(),
        Some(serde_json::json!({ "name": "Core lab" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(project["filename"], "Core lab.gns3");
    let project_id = project["project_id"].as_str().expect("project UUID");

    let create_node = |name: &str| serde_json::json!({ "name": name, "node_type": "docker", "image": "alpine:latest" });
    let (status, node_a) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes"),
        Some(create_node("Edge A")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(node_a["node_type"], "docker");
    assert!(node_a["container_id"].is_string());
    let (status, node_b) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes"),
        Some(create_node("Edge B")),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let endpoints = serde_json::json!([
        { "node_id": node_a["node_id"], "adapter_number": 0, "port_number": "auto", "label": { "text": "A" } },
        { "node_id": node_b["node_id"], "adapter_number": 0, "port_number": "auto", "label": { "text": "B" } }
    ]);
    let (status, error) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/links"),
        Some(serde_json::json!({ "nodes": [endpoints[0], endpoints[1], endpoints[0]] })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(error["message"].as_str().unwrap().contains("exactly two"));

    let (status, link) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/links"),
        Some(serde_json::json!({
            "nodes": endpoints,
            "suspend": true,
            "filters": { "delay": [10, 5, "normal"], "packet_loss": [10] },
            "link_style": { "width": 2 },
            "capturing": true,
            "capture_file_name": "client-controlled.pcap",
            "capture_file_path": "/etc/passwd"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(link["nodes"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        link["nodes"][0]["runtime_peer_link_port"],
        link["nodes"][1]["runtime_link_port"]
    );
    assert_eq!(
        link["nodes"][1]["runtime_peer_link_port"],
        link["nodes"][0]["runtime_link_port"]
    );
    assert_eq!(link["nodes"][0]["label"]["text"], "A");
    assert_eq!(link["nodes"][1]["label"]["text"], "B");
    assert_eq!(link["suspend"], true);
    assert_eq!(link["capturing"], false);
    assert!(link["capture_file_name"].is_null());
    assert!(link["capture_file_path"].is_null());
    assert_eq!(
        link["filters"]["delay"],
        serde_json::json!([10, 5, "normal"])
    );
    assert_eq!(link["link_style"]["width"], 2);
    let (status, _) = request(
        &app,
        Method::PUT,
        format!(
            "/v3/projects/{project_id}/links/{}",
            link["link_id"].as_str().unwrap()
        ),
        Some(serde_json::json!({ "capture_file_path": "/etc/passwd" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let occupied_endpoints = serde_json::json!([
        {"node_id":node_a["node_id"],"adapter_number":0,"port_number":0},
        {"node_id":node_b["node_id"],"adapter_number":0,"port_number":0}
    ]);
    let (status, conflict) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/links"),
        Some(serde_json::json!({ "nodes": occupied_endpoints })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(conflict["message"].is_string());
    assert_eq!(conflict["code"], "port_in_use");

    let (status, snapshot) = request(
        &app,
        Method::GET,
        format!("/v3/projects/{project_id}/topology"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(snapshot["nodes"].as_array().map(Vec::len), Some(2));
    assert_eq!(snapshot["links"].as_array().map(Vec::len), Some(1));
    assert!(snapshot["event_cursor"].as_i64().unwrap_or_default() >= 4);

    let (status, _) = request(
        &app,
        Method::DELETE,
        format!(
            "/v3/projects/{project_id}/nodes/{}",
            node_a["node_id"].as_str().unwrap()
        ),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (_, snapshot) = request(
        &app,
        Method::GET,
        format!("/v3/projects/{project_id}/topology"),
        None,
    )
    .await;
    assert_eq!(snapshot["nodes"].as_array().map(Vec::len), Some(1));
    assert_eq!(snapshot["links"].as_array().map(Vec::len), Some(0));
}

#[tokio::test]
async fn project_lifecycle_batches_skip_inapplicable_nodes_and_continue_after_failures() {
    let (app, directory) = test_router().await;
    let (_, project) = request(
        &app,
        Method::POST,
        "/v3/projects".into(),
        Some(serde_json::json!({ "name": "Batch lifecycle lab" })),
    )
    .await;
    let project_id = project["project_id"].as_str().unwrap();
    let (_, templates) = request(&app, Method::GET, "/v3/templates".into(), None).await;
    for (node_type, name) in [
        ("ethernet_switch", "A switch"),
        ("ethernet_hub", "B hub"),
        ("cloud", "C cloud"),
        ("nat", "D NAT"),
    ] {
        let template = templates
            .as_array()
            .unwrap()
            .iter()
            .find(|template| template["template_type"] == node_type)
            .unwrap();
        let (status, _) = request(
            &app,
            Method::POST,
            format!(
                "/v3/projects/{project_id}/templates/{}",
                template["template_id"].as_str().unwrap()
            ),
            Some(serde_json::json!({ "name": name, "x": 0, "y": 0 })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "create {node_type}");
    }

    let (status, _docker_node) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes"),
        Some(serde_json::json!({
            "name": "Z Docker",
            "node_type": "docker",
            "image": "alpine:latest",
            "adapters": 1
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    for (action, expected_runtime_state) in [
        ("start", "running"),
        ("suspend", "paused"),
        ("start", "running"),
        ("reload", "running"),
        ("stop", "exited"),
    ] {
        let (status, _) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes/{action}"),
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "project {action} all");
        let runtime_state = std::fs::read_to_string(directory.path().join("container-state"))
            .expect("Docker test runtime state is written");
        assert_eq!(
            runtime_state.trim(),
            expected_runtime_state,
            "Docker after {action} all"
        );
    }

    let (status, missing_qemu) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes"),
        Some(serde_json::json!({
            "name": "Y missing QEMU",
            "node_type": "qemu",
            "hda_disk_image": directory.path().join("qemu-images/router.qcow2"),
            "adapters": 0,
            "memory": 64,
            "cpus": 1
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    std::fs::remove_file(missing_qemu["runtime_overlay"].as_str().unwrap()).unwrap();

    let (status, error) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes/start"),
        Some(serde_json::json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(error["message"].as_str().unwrap().contains("overlay"));
    let runtime_state = std::fs::read_to_string(directory.path().join("container-state"))
        .expect("Docker test runtime state is written after the batch failure");
    assert_eq!(runtime_state.trim(), "running");
    assert!(!super::super::controller_routes::bulk_node_action_applies(
        &serde_json::json!({ "node_type": "vpcs", "status": "started" }),
        crate::runtime::NodeAction::Suspend
    ));
}
