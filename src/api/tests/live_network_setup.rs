use super::*;

#[tokio::test]
#[ignore = "requires the upstream VPCS executable; set NETWORK_SIMULATOR_TEST_VPCS and run with --ignored"]
async fn vpcs_template_creates_nodes_and_point_to_point_links_through_the_controller() {
    let (app, _directory) = test_router().await;
    create_custom_from_runtime(&app, "vpcs", "VPCS").await;
    let (status, templates) = request(&app, Method::GET, "/v3/templates".into(), None).await;
    assert_eq!(status, StatusCode::OK);
    let template = templates
        .as_array()
        .unwrap()
        .iter()
        .find(|template| template["template_type"] == "vpcs")
        .expect("VPCS template is listed when its executable is configured");
    let template_id = template["template_id"].as_str().unwrap();
    let (_, project) = request(
        &app,
        Method::POST,
        "/v3/projects".into(),
        Some(serde_json::json!({ "name": "VPCS lab" })),
    )
    .await;
    let project_id = project["project_id"].as_str().unwrap();

    let mut nodes = Vec::new();
    for (name, address) in [("PC A", "10.0.0.1/24"), ("PC B", "10.0.0.2/24")] {
        let (status, node) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/templates/{template_id}"),
            Some(serde_json::json!({
                "name": name,
                "startup_script": format!("ip {address}")
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(node["node_type"], "vpcs");
        assert_eq!(node["adapters"], 1);
        nodes.push(node);
    }

    let (status, link) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/links"),
        Some(serde_json::json!({
            "nodes": nodes.iter().map(|node| serde_json::json!({
                "node_id": node["node_id"],
                "adapter_number": 0,
                "port_number": 0
            })).collect::<Vec<_>>()
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(link["nodes"].as_array().unwrap().len(), 2);

    for node in &nodes {
        let (status, started) = request(
            &app,
            Method::POST,
            format!(
                "/v3/projects/{project_id}/nodes/{}/start",
                node["node_id"].as_str().unwrap()
            ),
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(started["status"], "started");
    }

    let console_path = format!(
        "/v3/compute/projects/{project_id}/vpcs/nodes/{}/console/ws",
        nodes[0]["node_id"].as_str().unwrap()
    );
    use futures_util::{SinkExt, StreamExt};
    use tokio::{
        net::TcpListener,
        time::{Duration, timeout},
    };
    use tokio_tungstenite::{connect_async, tungstenite::Message};
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("temporary VPCS console listener binds");
    let address = listener.local_addr().unwrap();
    let server_app = app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, server_app)
            .await
            .expect("VPCS console HTTP server runs");
    });
    let (mut console, _) = connect_async(format!("ws://{address}{console_path}"))
        .await
        .expect("VPCS console WebSocket connects");
    let mut startup_output = String::new();
    timeout(Duration::from_secs(5), async {
        loop {
            let Some(Ok(Message::Binary(bytes))) = console.next().await else {
                panic!("VPCS console closed before its prompt");
            };
            startup_output.push_str(&String::from_utf8_lossy(&bytes));
            if startup_output.contains("VPCS>") {
                break;
            }
        }
    })
    .await
    .expect("VPCS startup script reaches the browser console");
    console
        .send(Message::Text("ping 10.0.0.2 -c 1\r".into()))
        .await
        .expect("browser console sends a VPCS command");
    let mut ping_output = String::new();
    timeout(Duration::from_secs(12), async {
        loop {
            let Some(Ok(Message::Binary(bytes))) = console.next().await else {
                panic!("VPCS console closed while ping was running");
            };
            ping_output.push_str(&String::from_utf8_lossy(&bytes));
            if ping_output.contains("bytes from 10.0.0.2") {
                break;
            }
        }
    })
    .await
    .expect("VPCS ping response reaches the browser console");
    assert!(
        ping_output.contains("bytes from 10.0.0.2"),
        "unexpected output: {ping_output}"
    );
    console
        .send(Message::Close(None))
        .await
        .expect("browser console closes cleanly");
    server.abort();

    for node in &nodes {
        let (status, stopped) = request(
            &app,
            Method::POST,
            format!(
                "/v3/projects/{project_id}/nodes/{}/stop",
                node["node_id"].as_str().unwrap()
            ),
            Some(serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(stopped["status"], "stopped");
    }
}

#[tokio::test]
#[ignore = "requires Docker, alpine:latest, upstream VPCS, and capable upstream uBridge"]
async fn alpine_and_vpcs_exchange_icmp_over_a_real_mixed_link_through_the_controller() {
    use std::{env, path::PathBuf};
    use tokio::process::Command;

    use crate::runtime::DockerRuntime;

    let directory = TempDir::new().expect("temporary mixed-runtime API state");
    let store = Store::connect(
        directory
            .path()
            .join("controller.sqlite")
            .to_str()
            .expect("temporary database path is UTF-8"),
    )
    .await
    .expect("test database connects");
    let vpcs_binary = env::var_os("NETWORK_SIMULATOR_TEST_VPCS")
        .expect("NETWORK_SIMULATOR_TEST_VPCS names upstream VPCS");
    let app = router(AppState::with_runtimes(
        store,
        DockerRuntime::new(),
        crate::qemu::QemuRuntime::new(),
        VpcsRuntime::with_paths(vpcs_binary, directory.path().join("vpcs-state")),
        DynamipsRuntime::new(),
        crate::marketplace::MarketplaceCatalog::with_directory(directory.path().join("appliances")),
    ));
    let mut project_id: Option<String> = None;

    let run: Result<(), String> = async {
        let (status, project) = request(
            &app,
            Method::POST,
            "/v3/projects".into(),
            Some(serde_json::json!({ "name": "Live Docker VPCS link" })),
        )
        .await;
        if status != StatusCode::CREATED {
            return Err(format!("project create returned {status}: {project}"));
        }
        let current_project_id = project["project_id"]
            .as_str()
            .ok_or_else(|| "created project has no ID".to_owned())?
            .to_owned();
        project_id = Some(current_project_id.clone());

        let (status, docker_node) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{current_project_id}/nodes"),
            Some(serde_json::json!({
                "name": "Alpine peer",
                "node_type": "docker",
                "image": "alpine:latest",
                "start_command": "/bin/sh -c 'sleep 3600'",
                "adapters": 1
            })),
        )
        .await;
        if status != StatusCode::CREATED {
            return Err(format!(
                "Docker node create returned {status}: {docker_node}"
            ));
        }

        let (status, vpcs_node) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{current_project_id}/nodes"),
            Some(serde_json::json!({
                "name": "VPCS peer",
                "node_type": "vpcs",
                "startup_script": "ip 10.0.0.1/24"
            })),
        )
        .await;
        if status != StatusCode::CREATED {
            return Err(format!("VPCS node create returned {status}: {vpcs_node}"));
        }

        for node in [&vpcs_node, &docker_node] {
            let node_id = node["node_id"]
                .as_str()
                .ok_or_else(|| "created node has no ID".to_owned())?;
            let (status, started) = request(
                &app,
                Method::POST,
                format!("/v3/projects/{current_project_id}/nodes/{node_id}/start"),
                Some(serde_json::json!({})),
            )
            .await;
            if status != StatusCode::OK || started["status"] != "started" {
                return Err(format!("node start returned {status}: {started}"));
            }
        }

        // Create after both runtimes are already running to exercise live
        // endpoint attachment through the controller route.
        let endpoints = serde_json::json!([
            { "node_id": docker_node["node_id"], "adapter_number": 0, "port_number": 0 },
            { "node_id": vpcs_node["node_id"], "adapter_number": 0, "port_number": 0 }
        ]);
        let (status, link) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{current_project_id}/links"),
            Some(serde_json::json!({ "nodes": endpoints })),
        )
        .await;
        if status != StatusCode::CREATED
            || link["nodes"][0]["runtime_link_port"].as_u64().is_none()
            || link["nodes"][1]["runtime_link_port"].as_u64().is_none()
            || link["nodes"][0]["runtime_ubridge_segment"].is_string()
        {
            return Err(format!(
                "live mixed UDP NIO link create returned {status}: {link}"
            ));
        }

        let container_id = docker_node["container_id"]
            .as_str()
            .ok_or_else(|| "Docker node has no container ID".to_owned())?;
        let docker_binary = env::var_os("NETWORK_SIMULATOR_DOCKER")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("docker"));
        for args in [
            vec!["ip", "addr", "add", "10.0.0.2/24", "dev", "eth0"],
            vec!["ip", "link", "set", "eth0", "up"],
        ] {
            let output = Command::new(&docker_binary)
                .args(["exec", container_id])
                .args(args)
                .output()
                .await
                .map_err(|error| format!("could not configure Alpine eth0: {error}"))?;
            if !output.status.success() {
                return Err(format!(
                    "could not configure Alpine eth0: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
        }

        let output = Command::new(docker_binary)
            .args([
                "exec",
                container_id,
                "ping",
                "-c",
                "1",
                "-W",
                "3",
                "10.0.0.1",
            ])
            .output()
            .await
            .map_err(|error| format!("could not run Alpine-to-VPCS ping: {error}"))?;
        if !output.status.success()
            || !String::from_utf8_lossy(&output.stdout).contains("1 packets received")
        {
            return Err(format!(
                "Alpine-to-VPCS ICMP failed: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(())
    }
    .await;

    let cleanup = if let Some(project_id) = project_id {
        let (status, body) = request(
            &app,
            Method::DELETE,
            format!("/v3/projects/{project_id}"),
            None,
        )
        .await;
        (status == StatusCode::NO_CONTENT)
            .then_some(())
            .ok_or_else(|| format!("temporary project cleanup returned {status}: {body}"))
    } else {
        Ok(())
    };
    assert!(run.is_ok(), "live mixed-runtime API test failed: {run:?}");
    assert!(
        cleanup.is_ok(),
        "live mixed-runtime cleanup failed: {cleanup:?}"
    );
}
