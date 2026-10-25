use super::*;

#[tokio::test]
async fn builtin_http_lifecycle_matches_always_running_nodes() {
    let (app, _directory) = test_router().await;
    let (_, project) = request(
        &app,
        Method::POST,
        "/v3/projects".into(),
        Some(serde_json::json!({ "name": "Always on builtins" })),
    )
    .await;
    let project_id = project["project_id"].as_str().unwrap();
    let (status, switch) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes"),
        Some(serde_json::json!({ "name": "SW1", "node_type": "ethernet_switch" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let switch_id = switch["node_id"].as_str().unwrap();
    for action in ["start", "stop", "suspend", "reload"] {
        let (status, _) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{project_id}/nodes/{switch_id}/{action}"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "switch {action}");
    }

    let (status, cloud) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes"),
        Some(serde_json::json!({
            "name": "Cloud1",
            "node_type": "cloud",
            "ports_mapping": [{"type":"ethernet","interface":"virbr0"}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let cloud_id = cloud["node_id"].as_str().unwrap();
    let (status, started) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes/{cloud_id}/start"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(started["status"], "started");
    let (status, _) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes/{cloud_id}/stop"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn builtin_node_http_validation_rejects_bad_vlan_and_cloud_nio_values() {
    let (app, _directory) = test_router().await;
    let (_, project) = request(
        &app,
        Method::POST,
        "/v3/projects".into(),
        Some(serde_json::json!({ "name": "Builtin validation" })),
    )
    .await;
    let project_id = project["project_id"].as_str().unwrap();
    let (status, error) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes"),
        Some(serde_json::json!({
            "name":"Bad switch",
            "node_type":"ethernet_switch",
            "ports_mapping":[{"type":"access","vlan":4095}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(error["message"].as_str().unwrap().contains("VLAN"));

    let (status, error) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes"),
        Some(serde_json::json!({
            "name":"Bad cloud",
            "node_type":"cloud",
            "ports_mapping":[{"type":"udp","lport":4001,"rhost":"127.0.0.1;false","rport":4002}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(error["message"].as_str().unwrap().contains("remote host"));

    let (status, error) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes"),
        Some(serde_json::json!({
            "name":"Loopback cloud",
            "node_type":"cloud",
            "ports_mapping":[{"type":"ethernet","interface":"lo"}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("allow_host_interfaces")
    );

    let (status, allowlisted_cloud) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes"),
        Some(serde_json::json!({
            "name":"Explicit loopback cloud",
            "node_type":"cloud",
            "ports_mapping":[{"type":"ethernet","interface":"lo"}],
            "allow_host_interfaces":["lo"]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        allowlisted_cloud["allow_host_interfaces"],
        serde_json::json!(["lo"])
    );

    let (status, error) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes"),
        Some(serde_json::json!({
            "name":"Missing host interface",
            "node_type":"cloud",
            "ports_mapping":[{"type":"ethernet","interface":"nosuch0"}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("does not exist")
    );

    let (status, error) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes"),
        Some(serde_json::json!({
            "name":"Unapproved NAT interface",
            "node_type":"nat",
            "ports_mapping":[{"type":"ethernet","interface":"lo"}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("configured default")
    );

    let (status, allowlisted_nat) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes"),
        Some(serde_json::json!({
            "name":"Explicit interface NAT",
            "node_type":"nat",
            "ports_mapping":[{"type":"ethernet","interface":"lo"}],
            "allow_host_interfaces":["lo"]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        allowlisted_nat["allow_host_interfaces"],
        serde_json::json!(["lo"])
    );
}

#[tokio::test]
async fn builtin_configuration_uses_flat_fields_and_preserves_always_running_status() {
    let (app, _directory) = test_router().await;
    let (_, project) = request(
        &app,
        Method::POST,
        "/v3/projects".into(),
        Some(serde_json::json!({ "name": "Builtin configuration" })),
    )
    .await;
    let project_id = project["project_id"].as_str().unwrap();

    let (_, switch) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes"),
        Some(serde_json::json!({
            "name":"SW1","node_type":"ethernet_switch",
            "ports_mapping":[
                {"type":"access","vlan":1},
                {"type":"access","vlan":1}
            ]
        })),
    )
    .await;
    let switch_id = switch["node_id"].as_str().unwrap();
    let (status, configured_switch) = request(
        &app,
        Method::PUT,
        format!("/v3/projects/{project_id}/nodes/{switch_id}/config"),
        Some(serde_json::json!({
            "name":"SW1-updated",
            "ports_mapping":[
                {"type":"access","vlan":10},
                {"type":"dot1q","vlan":10}
            ],
            "console_type":"telnet"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(configured_switch["name"], "SW1-updated");
    assert_eq!(configured_switch["status"], "started");
    assert_eq!(configured_switch["ports_count"], 2);
    assert_eq!(configured_switch["ports_mapping"][1]["type"], "dot1q");

    let (_, hub) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes"),
        Some(serde_json::json!({"name":"Hub1","node_type":"ethernet_hub"})),
    )
    .await;
    let hub_id = hub["node_id"].as_str().unwrap();
    let (status, configured_hub) = request(
        &app,
        Method::PUT,
        format!("/v3/projects/{project_id}/nodes/{hub_id}/config"),
        Some(serde_json::json!({
            "properties": {
                "ports_mapping": [
                    {"name":"old","port_number":9},
                    {"name":"old2","port_number":9}
                ]
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(configured_hub["status"], "started");
    assert_eq!(configured_hub["ports_mapping"].as_array().unwrap().len(), 2);
    assert!(configured_hub["properties"]["ports_mapping"].is_null());

    let (_, cloud) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes"),
        Some(serde_json::json!({
            "name":"Cloud1","node_type":"cloud",
            "ports_mapping":[{"type":"udp","lport":4100,"rhost":"127.0.0.1","rport":4101}]
        })),
    )
    .await;
    let cloud_id = cloud["node_id"].as_str().unwrap();
    let (status, configured_cloud) = request(
        &app,
        Method::PUT,
        format!("/v3/projects/{project_id}/nodes/{cloud_id}/config"),
        Some(serde_json::json!({
            "remote_console_host":"192.0.2.5",
            "remote_console_port":2323,
            "remote_console_type":"telnet",
            "remote_console_http_path":"/console"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(configured_cloud["status"], "started");
    assert_eq!(configured_cloud["remote_console_host"], "192.0.2.5");
    assert_eq!(configured_cloud["remote_console_port"], 2323);

    let (_, nat) = request(
        &app,
        Method::POST,
        format!("/v3/projects/{project_id}/nodes"),
        Some(serde_json::json!({"name":"NAT1","node_type":"nat"})),
    )
    .await;
    let nat_id = nat["node_id"].as_str().unwrap();
    let (status, configured_nat) = request(
        &app,
        Method::PUT,
        format!("/v3/projects/{project_id}/nodes/{nat_id}/config"),
        Some(serde_json::json!({"name":"NAT1-updated"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(configured_nat["name"], "NAT1-updated");
    assert_eq!(configured_nat["status"], "started");
    assert_eq!(configured_nat["ports_mapping"][0]["interface"], "virbr0");
}

#[test]
fn validates_only_runtime_settings_supported_by_each_runtime() {
    let qemu = validate_node_configuration_patch(
        "qemu",
        serde_json::json!({
            "memory": 2048,
            "cpus": 4,
            "adapters": 3,
            "adapter_type": "virtio-net-pci",
            "hda_disk_interface": "scsi",
            "boot_priority": "dc"
        }),
    )
    .unwrap_or_else(|_| panic!("QEMU's implemented settings should be accepted"));
    assert_eq!(qemu["memory"], 2048);
    assert!(
        validate_node_configuration_patch(
            "qemu",
            serde_json::json!({ "adapter_type": "unsupported-nic" })
        )
        .is_err()
    );

    let docker = validate_node_configuration_patch(
        "docker",
        serde_json::json!({
            "memory": 0,
            "cpus": 0.5,
            "adapters": 4,
            "environment": "LAB_MODE=router",
            "extra_hosts": "gateway:192.0.2.1",
            "extra_volumes": ["/etc/lab"],
            "console_type": "none"
        }),
    )
    .unwrap_or_else(|_| panic!("Docker's implemented settings should be accepted"));
    assert_eq!(docker["cpus"], 0.5);
    assert_eq!(docker["console_type"], "none");
    assert!(
        validate_node_configuration_patch(
            "docker",
            serde_json::json!({ "runtime_overlay": "/tmp/image.qcow2" })
        )
        .is_err()
    );

    let vpcs = validate_node_configuration_patch(
        "vpcs",
        serde_json::json!({
            "startup_script": "ip 192.0.2.2/24",
            "console_type": "ssh"
        }),
    )
    .unwrap_or_else(|_| {
        panic!("VPCS startup scripts and GNS3 console types should be configurable")
    });
    assert_eq!(vpcs["startup_script"], "ip 192.0.2.2/24");
    assert_eq!(vpcs["console_type"], "ssh");
    for console_type in ["telnet", "ssh", "none"] {
        assert!(
            validate_node_configuration_patch(
                "vpcs",
                serde_json::json!({"console_type": console_type})
            )
            .is_ok()
        );
    }
    assert!(
        validate_node_configuration_patch("vpcs", serde_json::json!({"console_type": "vnc"}))
            .is_err()
    );
    let dynamips = validate_node_configuration_patch(
        "dynamips",
        serde_json::json!({ "startup_config": "hostname R1\n" }),
    )
    .unwrap_or_else(|_| panic!("Dynamips startup config should be configurable"));
    assert_eq!(dynamips["startup_config"], "hostname R1\n");
    assert!(
        validate_node_configuration_patch("dynamips", serde_json::json!({ "ram": 512 })).is_err()
    );
}

#[tokio::test]
async fn vpcs_suspend_matches_gns3_method_not_allowed_behavior() {
    let directory = TempDir::new().expect("temporary VPCS suspend test directory");
    let store = Store::connect(
        directory
            .path()
            .join("vpcs-suspend.sqlite")
            .to_str()
            .expect("UTF-8 database path"),
    )
    .await
    .expect("test database connects");
    let project_id = uuid::Uuid::new_v4();
    let node_id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO projects(project_id,name,status,project_json) VALUES(?,?,?,?)")
        .bind(project_id.to_string())
        .bind("VPCS suspend")
        .bind("opened")
        .bind(serde_json::json!({"name":"VPCS suspend"}).to_string())
        .execute(store.pool())
        .await
        .unwrap();
    let node = serde_json::json!({
        "node_id": node_id,
        "project_id": project_id,
        "node_type": "vpcs",
        "name": "PC1",
        "status": "started"
    });
    sqlx::query("INSERT INTO topology_objects(project_id,kind,object_id,name,object_json) VALUES(?,?,?,?,?)")
        .bind(project_id.to_string())
        .bind("node")
        .bind(node_id.to_string())
        .bind("PC1")
        .bind(node.to_string())
        .execute(store.pool())
        .await
        .unwrap();

    let state = AppState::new(store);
    let error = match suspend_node(State(state), Path((project_id, node_id))).await {
        Ok(_) => panic!("VPCS suspend should be rejected"),
        Err(error) => error,
    };
    assert_eq!(error.status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn forwarded_proxy_headers_keep_bearer_authentication_enforced() {
    let directory = TempDir::new().expect("temporary proxy-auth test directory");
    let store = Store::connect_unseeded(
        directory
            .path()
            .join("proxy-auth.sqlite")
            .to_str()
            .expect("UTF-8 database path"),
    )
    .await
    .expect("test database connects without auth bootstrap");
    let initial_password_path = directory.path().join("initial-password.txt");
    auth::initialize(store.pool(), true, &initial_password_path)
        .await
        .expect("auth bootstrap completes");
    let password =
        std::fs::read_to_string(initial_password_path).expect("initial password file is readable");
    let app = router(AppState::new(store));

    let login = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v3/access/users/authenticate")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "username":"admin",
                        "password":password.trim()
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::OK);
    let login_body: Value =
        serde_json::from_slice(&login.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let token = login_body["access_token"].as_str().unwrap();

    let without_token = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v3/access/users/me")
                .header("x-forwarded-for", "198.51.100.27")
                .header("x-forwarded-proto", "https")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(without_token.status(), StatusCode::UNAUTHORIZED);

    let with_token = app
        .oneshot(
            Request::builder()
                .uri("/v3/access/users/me")
                .header("x-forwarded-for", "198.51.100.27")
                .header("x-forwarded-proto", "https")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(with_token.status(), StatusCode::OK);
}

#[test]
fn sparse_dynamips_template_options_use_platform_defaults() {
    let mut template = serde_json::json!({
        "dynamips_id": null,
        "platform": "c3725",
        "slot0": "GT96100-FE",
        "slot1": null,
        "wic0": null,
        "mmap": null,
        "sparsemem": null,
        "clock_divisor": null,
        "private_config": null,
        "console_type": null,
        "mac_addr": "",
        "system_id": "FTX0945W0MY",
        "iomem": null
    })
    .as_object()
    .expect("template JSON is an object")
    .clone();

    let options = dynamips_options_from_template(&mut template, 7)
        .unwrap_or_else(|_| panic!("allocated runtime ID should make the template settings valid"));

    assert_eq!(options.dynamips_id, 7);
    assert_eq!(template["dynamips_id"], 7);
    assert_eq!(options.slots[0].as_deref(), Some("GT96100-FE"));
    assert_eq!(options.wics[0], None);
    assert_eq!(options.mac_addr, None);
    assert_eq!(options.iomem, None);
    assert!(options.mmap);
    assert!(options.sparsemem);
    assert_eq!(options.clock_divisor, 8);
    assert!(options.private_config.is_empty());
    assert_eq!(options.console_type, "telnet");
}
