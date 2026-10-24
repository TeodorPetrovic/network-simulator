use super::*;

#[tokio::test]
#[ignore = "requires Docker, upstream VPCS/uBridge, and uBridge CAP_NET_ADMIN/CAP_NET_RAW"]
async fn builtin_switch_live_connects_three_hosts_and_isolates_access_vlans() {
    use std::{env, path::Path, path::PathBuf};

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        process::Command,
        time::{Duration, timeout},
    };

    async fn require_status(
        app: &axum::Router,
        method: Method,
        uri: String,
        body: Option<Value>,
        expected: StatusCode,
    ) -> Result<Value, String> {
        let (status, response) = request(app, method, uri.clone(), body).await;
        if status != expected {
            return Err(format!("{uri} returned {status}: {response}"));
        }
        Ok(response)
    }

    async fn docker_command(
        binary: &Path,
        container: &str,
        arguments: &[&str],
    ) -> Result<std::process::Output, String> {
        Command::new(binary)
            .args(["exec", container])
            .args(arguments)
            .output()
            .await
            .map_err(|error| format!("Docker command failed to run: {error}"))
    }

    async fn vpcs_ping(console: &mut tokio::net::TcpStream, address: &str) -> Result<bool, String> {
        for _ in 0..2 {
            console
                .write_all(format!("ping {address} -c 1\r").as_bytes())
                .await
                .map_err(|error| error.to_string())?;
            let mut output = Vec::new();
            timeout(Duration::from_secs(8), async {
                loop {
                    let mut chunk = [0_u8; 2048];
                    let length = console.read(&mut chunk).await?;
                    output.extend_from_slice(&chunk[..length]);
                    let text = String::from_utf8_lossy(&output);
                    if text.contains("bytes from") || text.contains("VPCS>") {
                        break Ok::<(), std::io::Error>(());
                    }
                }
            })
            .await
            .map_err(|_| format!("VPCS ping to {address} timed out"))?
            .map_err(|error| error.to_string())?;
            if String::from_utf8_lossy(&output).contains("bytes from") {
                return Ok(true);
            }
        }
        Ok(false)
    }

    let directory = TempDir::new().expect("temporary builtin live test state");
    let store = Store::connect(
        directory
            .path()
            .join("controller.sqlite")
            .to_str()
            .expect("UTF-8 test database path"),
    )
    .await
    .expect("test database connects");
    let vpcs_binary = env::var_os("NETWORK_SIMULATOR_TEST_VPCS")
        .expect("NETWORK_SIMULATOR_TEST_VPCS names upstream VPCS");
    let ubridge_binary = env::var_os("NETWORK_SIMULATOR_TEST_UBRIDGE")
        .expect("NETWORK_SIMULATOR_TEST_UBRIDGE names capable upstream uBridge");
    let vpcs = VpcsRuntime::with_paths(vpcs_binary, directory.path().join("vpcs-state"));
    let ubridge = crate::ubridge::UbridgeRuntime::with_paths(
        PathBuf::from(ubridge_binary),
        directory.path().join("ubridge.sock"),
    );
    let app = router(AppState::with_runtimes_and_ubridge(
        store,
        DockerRuntime::new(),
        QemuRuntime::new(),
        vpcs.clone(),
        DynamipsRuntime::new(),
        ubridge.clone(),
        crate::marketplace::MarketplaceCatalog::with_directory(directory.path().join("appliances")),
    ));
    let docker_binary = env::var_os("NETWORK_SIMULATOR_DOCKER")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("docker"));
    let mut project_id: Option<String> = None;
    let mut switch_node_id: Option<String> = None;
    let mut vpcs_node_id: Option<String> = None;
    let mut link_endpoints = Vec::<Value>::new();
    let mut docker_hosts = Vec::<(String, String, String, String)>::new();

    let run: Result<(), String> = async {
        let project = require_status(
            &app,
            Method::POST,
            "/v3/projects".into(),
            Some(serde_json::json!({ "name": "Builtin switch live test" })),
            StatusCode::CREATED,
        )
        .await?;
        let current_project_id = project["project_id"]
            .as_str()
            .ok_or_else(|| "project has no ID".to_owned())?
            .to_owned();
        project_id = Some(current_project_id.clone());

        let switch = require_status(
            &app,
            Method::POST,
            format!("/v3/projects/{current_project_id}/nodes"),
            Some(serde_json::json!({
                "name":"SW1",
                "node_type":"ethernet_switch",
                "ports_mapping":[
                    {"type":"access","vlan":10},
                    {"type":"access","vlan":10},
                    {"type":"access","vlan":10},
                    {"type":"access","vlan":20}
                ]
            })),
            StatusCode::CREATED,
        )
        .await?;
        let switch_id = switch["node_id"]
            .as_str()
            .ok_or_else(|| "switch has no ID".to_owned())?
            .to_owned();
        switch_node_id = Some(switch_id.clone());

        let mut hosts = Vec::<(String, String, String)>::new();
        for (name, address) in [
            ("Docker A", "10.0.10.1/24"),
            ("Docker B", "10.0.10.3/24"),
            ("Docker VLAN20", "10.0.10.4/24"),
        ] {
            let node = require_status(
                &app,
                Method::POST,
                format!("/v3/projects/{current_project_id}/nodes"),
                Some(serde_json::json!({
                    "name":name,
                    "node_type":"docker",
                    "image":"alpine:latest",
                    "start_command":"/bin/sh -c 'sleep 3600'",
                    "adapters":1
                })),
                StatusCode::CREATED,
            )
            .await?;
            let node_id = node["node_id"]
                .as_str()
                .ok_or_else(|| format!("{name} has no ID"))?
                .to_owned();
            let container_id = node["container_id"]
                .as_str()
                .ok_or_else(|| format!("{name} has no container ID"))?
                .to_owned();
            hosts.push((node_id.clone(), address.to_owned(), name.to_owned()));
            docker_hosts.push((container_id, node_id, address.to_owned(), name.to_owned()));
        }
        let vpcs_node = require_status(
            &app,
            Method::POST,
            format!("/v3/projects/{current_project_id}/nodes"),
            Some(serde_json::json!({
                "name":"VPCS C",
                "node_type":"vpcs",
                "startup_script":"ip 10.0.10.2/24"
            })),
            StatusCode::CREATED,
        )
        .await?;
        let vpcs_id = vpcs_node["node_id"]
            .as_str()
            .ok_or_else(|| "VPCS node has no ID".to_owned())?
            .to_owned();
        vpcs_node_id = Some(vpcs_id.clone());
        hosts.insert(
            2,
            (
                vpcs_id.clone(),
                "10.0.10.2/24".to_owned(),
                "VPCS C".to_owned(),
            ),
        );

        for (port_number, (node_id, _, name)) in hosts.iter().enumerate() {
            let link = require_status(
                &app,
                Method::POST,
                format!("/v3/projects/{current_project_id}/links"),
                Some(serde_json::json!({ "nodes": [
                    {"node_id":switch_id,"adapter_number":0,"port_number":port_number},
                    {"node_id":node_id,"adapter_number":0,"port_number":0}
                ]})),
                StatusCode::CREATED,
            )
            .await
            .map_err(|error| format!("{name} switch link: {error}"))?;
            link_endpoints.extend(link["nodes"].as_array().into_iter().flatten().cloned());
        }

        for (node_id, _, name) in &hosts {
            let started = require_status(
                &app,
                Method::POST,
                format!("/v3/projects/{current_project_id}/nodes/{node_id}/start"),
                Some(serde_json::json!({})),
                StatusCode::OK,
            )
            .await
            .map_err(|error| format!("start {name}: {error}"))?;
            if started["status"] != "started" {
                return Err(format!("{name} did not start: {started}"));
            }
        }

        for (container, _, address, name) in &docker_hosts {
            for arguments in [
                vec!["ip", "link", "set", "eth0", "up"],
                vec!["ip", "addr", "add", address, "dev", "eth0"],
            ] {
                let output = docker_command(&docker_binary, container, &arguments).await?;
                if !output.status.success() {
                    return Err(format!(
                        "configure {name}: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    ));
                }
            }
        }

        let mut console = vpcs
            .connect_console(&vpcs_id)
            .await
            .map_err(|error| error.message().to_owned())?;
        let mut prompt = Vec::new();
        timeout(Duration::from_secs(10), async {
            loop {
                let mut chunk = [0_u8; 2048];
                let length = console.read(&mut chunk).await?;
                prompt.extend_from_slice(&chunk[..length]);
                if String::from_utf8_lossy(&prompt).contains("VPCS>") {
                    break Ok::<(), std::io::Error>(());
                }
            }
        })
        .await
        .map_err(|_| "VPCS console prompt timed out")?
        .map_err(|error| error.to_string())?;

        for (container, _, _, name) in &docker_hosts[..2] {
            let targets = if name == "Docker A" {
                ["10.0.10.2", "10.0.10.3"]
            } else {
                ["10.0.10.1", "10.0.10.2"]
            };
            for target in targets {
                let output = Command::new(&docker_binary)
                    .args(["exec", container, "ping", "-c", "1", "-W", "2", target])
                    .output()
                    .await
                    .map_err(|error| format!("Docker ping could not run: {error}"))?;
                if !output.status.success()
                    || !String::from_utf8_lossy(&output.stdout).contains("1 packets received")
                {
                    return Err(format!(
                        "{name} could not ping {target}: {}{}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    ));
                }
            }
        }
        for target in ["10.0.10.1", "10.0.10.3"] {
            if !vpcs_ping(&mut console, target).await? {
                return Err(format!("VPCS could not ping {target} on the switch"));
            }
        }
        let isolated = Command::new(&docker_binary)
            .args([
                "exec",
                &docker_hosts[2].0,
                "ping",
                "-c",
                "1",
                "-W",
                "2",
                "10.0.10.1",
            ])
            .output()
            .await
            .map_err(|error| format!("VLAN isolation ping could not run: {error}"))?;
        if isolated.status.success() {
            return Err("VLAN20 access port reached the VLAN10 Docker host".to_owned());
        }
        Ok(())
    }
    .await;

    let cleanup = if let Some(project_id) = project_id.as_deref() {
        let (status, body) = request(
            &app,
            Method::DELETE,
            format!("/v3/projects/{project_id}"),
            None,
        )
        .await;
        (status == StatusCode::NO_CONTENT)
            .then_some(())
            .ok_or_else(|| format!("builtin live test cleanup returned {status}: {body}"))
    } else {
        Ok(())
    };
    for endpoint in &link_endpoints {
        let endpoint_id = endpoint["runtime_endpoint_id"].as_str().unwrap_or_default();
        assert!(
            !ubridge.has_link_endpoint(endpoint_id).await,
            "uBridge endpoint remained after cleanup: {endpoint_id}"
        );
        if let Some(node_id) = endpoint["node_id"].as_str()
            && docker_hosts
                .iter()
                .any(|(_, docker_node, _, _)| docker_node == node_id)
        {
            let host = crate::ubridge::host_veth_name(endpoint_id).expect("Docker veth name");
            let output = Command::new("ip")
                .args(["link", "show", "dev", &host])
                .output()
                .await
                .expect("inspect cleaned Docker veth");
            assert!(
                !output.status.success(),
                "Docker host veth remained: {host}"
            );
        }
    }
    if let Some(node_id) = vpcs_node_id.as_deref() {
        assert_eq!(
            vpcs.status(node_id).await.expect("inspect cleaned VPCS"),
            "stopped"
        );
    }
    assert!(run.is_ok(), "builtin switch live test failed: {run:?}");
    assert!(
        cleanup.is_ok(),
        "builtin switch cleanup failed: {cleanup:?}"
    );
    let _ = switch_node_id;
}
