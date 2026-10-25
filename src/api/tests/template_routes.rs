use super::*;

#[tokio::test]
async fn templates_create_nodes_with_real_runtime_lifecycle_routes() {
    let (app, directory) = test_router().await;
    create_custom_from_runtime(&app, "docker", "Alpine custom").await;
    let (status, templates) = request(&app, Method::GET, "/v3/templates".into(), None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(templates.as_array().unwrap().iter().any(|template| {
        template["template_type"] == "vpcs" && template["name"] == "VPCS"
    }));
    let template = templates
        .as_array()
        .unwrap()
        .iter()
        .find(|template| template["template_type"] == "docker")
        .expect("local Docker image template is listed");
    let template = template["template_id"].as_str().expect("template UUID");

    let (_, project) = request(
        &app,
        Method::POST,
        "/v3/projects".into(),
        Some(serde_json::json!({ "name": "Runtime lab" })),
    )
    .await;
    let project_id = project["project_id"].as_str().unwrap();
    let (status, node) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/templates/{template}"),
        Some(serde_json::json!({
            "name": "Alpine node",
            "x": 24,
            "y": -18,
            "start_command": "/bin/sh -c 'sleep 3600'"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(node["image"], "alpine:latest");
    assert_eq!(node["x"], 24);
    assert_eq!(node["start_command"], "/bin/sh -c 'sleep 3600'");

    let node_id = node["node_id"].as_str().unwrap();
    let (status, configured) = request(
        &app,
        Method::PUT,
        format!("/v3/projects/{project_id}/nodes/{node_id}/config"),
        Some(serde_json::json!({
            "memory": 256,
            "cpus": 1.5,
            "adapters": 2,
            "environment": "LAB_ROLE=router",
            "extra_hosts": "gateway:192.0.2.1",
            "extra_volumes": ["/etc/lab"]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(configured["memory"], 256);
    assert_eq!(configured["cpus"], 1.5);
    assert_eq!(configured["adapters"], 2);
    assert_eq!(configured["environment"], "LAB_ROLE=router");
    assert_eq!(configured["extra_hosts"], "gateway:192.0.2.1");
    assert_eq!(configured["extra_volumes"], serde_json::json!(["/etc/lab"]));
    let docker_arguments = std::fs::read_to_string(directory.path().join("docker-arguments.log"))
        .expect("Docker create arguments were recorded");
    let creates = docker_arguments
        .lines()
        .filter(|line| line.starts_with("create "))
        .collect::<Vec<_>>();
    assert!(
        creates.len() >= 2,
        "the edit replaces the stopped container"
    );
    assert!(creates.iter().all(|line| line.contains("--network none")));
    assert!(creates.iter().all(|line| line.contains("--privileged")));
    assert!(creates.iter().any(|line| line.contains("--memory 256m")));
    assert!(creates.iter().any(|line| line.contains("--cpus 1.5")));
    assert!(
        creates
            .iter()
            .any(|line| line.contains("--env LAB_ROLE=router"))
    );

    let (status, node) = request(
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
    assert_eq!(node["status"], "started");
    let (status, error) = request(
        &app,
        Method::PUT,
        format!("/v3/projects/{project_id}/nodes/{node_id}/config"),
        Some(serde_json::json!({ "memory": 512 })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("Stop this device")
    );

    let (status, node) = request(
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
    assert_eq!(node["status"], "stopped");
}

#[tokio::test]
async fn saved_runtime_templates_create_nodes_with_their_settings() {
    let (app, directory) = test_router_with_vpcs(true).await;
    let (_, project) = request(
        &app,
        Method::POST,
        "/v3/projects".into(),
        Some(serde_json::json!({ "name": "Saved template lab" })),
    )
    .await;
    let project_id = project["project_id"].as_str().unwrap();
    let qemu_image = directory.path().join("qemu-images/router.qcow2");

    let (status, vpcs_template) = request(
        &app,
        Method::POST,
        "/v3/templates".into(),
        Some(serde_json::json!({
            "name": "Saved VPCS",
            "template_type": "vpcs",
            "adapters": 1,
            "startup_script": "ip 192.0.2.10/24",
            "base_script_file": "configs/base.vpc",
            "console_type": "telnet",
            "console_auto_start": true
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, docker_template) = request(
        &app,
        Method::POST,
        "/v3/templates".into(),
        Some(serde_json::json!({
            "name": "Saved Docker",
            "template_type": "docker",
            "image": "alpine:latest",
            "start_command": "sleep 321",
            "adapters": 3,
            "memory": 256,
            "cpus": 1.5,
            "environment": "ROLE=router",
            "extra_hosts": "gateway:192.0.2.1",
            "extra_volumes": ["/etc/lab"],
            "console_type": "telnet",
            "console_resolution": "1280x720",
            "console_http_port": 8080,
            "console_http_path": "/console",
            "aux_type": "none",
            "custom_adapters": []
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, qemu_template) = request(
        &app,
        Method::POST,
        "/v3/templates".into(),
        Some(serde_json::json!({
            "name": "Saved QEMU",
            "template_type": "qemu",
            "hda_disk_image": qemu_image,
            "adapters": 2,
            "memory": 96,
            "cpus": 2,
            "platform": "x86_64",
            "adapter_type": "e1000",
            "hda_disk_interface": "ide",
            "boot_priority": "c",
            "linked_clone": false
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, vpcs_node) = request(
        &app,
        Method::POST,
        format!(
            "/v3/projects/{project_id}/templates/{}",
            vpcs_template["template_id"].as_str().unwrap()
        ),
        Some(serde_json::json!({ "name": "Map VPCS", "x": 12, "y": 34 })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(vpcs_node["startup_script"], "ip 192.0.2.10/24");
    assert_eq!(vpcs_node["base_script_file"], "configs/base.vpc");
    assert_eq!(vpcs_node["console_auto_start"], true);
    assert_eq!(vpcs_node["x"], 12);

    let (status, docker_node) = request(
        &app,
        Method::POST,
        format!(
            "/v3/projects/{project_id}/templates/{}",
            docker_template["template_id"].as_str().unwrap()
        ),
        Some(serde_json::json!({ "name": "Map Docker" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(docker_node["image"], "alpine:latest");
    assert_eq!(docker_node["start_command"], "sleep 321");
    assert_eq!(docker_node["adapters"], 3);
    assert_eq!(docker_node["memory"], 256);
    assert_eq!(docker_node["cpus"], 1.5);
    assert_eq!(docker_node["environment"], "ROLE=router");
    assert_eq!(docker_node["extra_hosts"], "gateway:192.0.2.1");
    assert_eq!(
        docker_node["extra_volumes"],
        serde_json::json!(["/etc/lab"])
    );
    assert_eq!(docker_node["console_resolution"], "1280x720");

    let (status, qemu_node) = request(
        &app,
        Method::POST,
        format!(
            "/v3/projects/{project_id}/templates/{}",
            qemu_template["template_id"].as_str().unwrap()
        ),
        Some(serde_json::json!({ "name": "Map QEMU" })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "QEMU template add: {qemu_node}"
    );
    assert_eq!(
        qemu_node["hda_disk_image"].as_str(),
        Some(qemu_image.to_str().unwrap())
    );
    assert_eq!(qemu_node["adapters"], 2);
    assert_eq!(qemu_node["memory"], 96);
    assert_eq!(qemu_node["cpus"], 2);
    assert_eq!(qemu_node["maxcpus"], 2);
    assert_eq!(qemu_node["linked_clone"], false);
}

#[tokio::test]
async fn rejects_unsupported_node_types() {
    let (app, _directory) = test_router().await;
    let (_, project) = request(
        &app,
        Method::POST,
        "/v3/projects".into(),
        Some(serde_json::json!({ "name": "Runtime compatibility" })),
    )
    .await;
    let project_id = project["project_id"].as_str().unwrap();
    let (status, error) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes"),
        Some(serde_json::json!({ "name": "Fake device", "node_type": "not-a-runtime" })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("Unsupported node type")
    );
}

#[tokio::test]
async fn builtin_templates_create_persistent_nodes_with_gns3_ports() {
    let (app, _directory) = test_router().await;
    let (_, project) = request(
        &app,
        Method::POST,
        "/v3/projects".into(),
        Some(serde_json::json!({ "name": "Builtin nodes" })),
    )
    .await;
    let project_id = project["project_id"].as_str().unwrap();

    let (status, templates) = request(&app, Method::GET, "/v3/templates".into(), None).await;
    assert_eq!(status, StatusCode::OK);
    let switch_template = templates
        .as_array()
        .unwrap()
        .iter()
        .find(|template| template["template_type"] == "ethernet_switch")
        .unwrap();
    assert_eq!(
        switch_template["ports_mapping"].as_array().unwrap().len(),
        8
    );
    let template_id = switch_template["template_id"].as_str().unwrap();

    let (status, switch) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/templates/{template_id}"),
        Some(serde_json::json!({
            "name": "SW1",
            "ports_mapping": [
                {"type":"access", "vlan":10},
                {"type":"access", "vlan":20}
            ]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(switch["node_type"], "ethernet_switch");
    assert_eq!(switch["status"], "started");
    assert_eq!(switch["ports_mapping"][0]["name"], "Ethernet0");
    assert_eq!(switch["ports_mapping"][1]["port_number"], 1);
    assert_eq!(switch["ports_mapping"][1]["vlan"], 20);

    let (status, nodes) = request(
        &app,
        Method::GET,
        format!("/v3/projects/{project_id}/nodes"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(nodes.as_array().unwrap().len(), 1);
    assert_eq!(nodes[0]["node_id"], switch["node_id"]);
}
