use super::*;

#[tokio::test]
#[ignore = "requires Docker, upstream VPCS/uBridge, and uBridge CAP_NET_ADMIN/CAP_NET_RAW"]
async fn builtin_hub_live_forwards_docker_vpcs_traffic_and_cleans_links() {
    use std::{env, path::PathBuf};

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        process::Command,
        time::{Duration, timeout},
    };

    let directory = TempDir::new().expect("temporary builtin hub live test state");
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
    let mut project_id = None;
    let mut docker_node_id = None;
    let mut docker_container_id = None;
    let mut vpcs_node_id = None;
    let mut endpoints = Vec::new();

    let run: Result<(), String> = async {
        let (status, project) = request(
            &app,
            Method::POST,
            "/v3/projects".into(),
            Some(serde_json::json!({"name":"Builtin hub live test"})),
        )
        .await;
        if status != StatusCode::CREATED {
            return Err(format!("create project returned {status}: {project}"));
        }
        let pid = project["project_id"]
            .as_str()
            .ok_or_else(|| "project has no ID".to_owned())?
            .to_owned();
        project_id = Some(pid.clone());

        let (status, hub) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{pid}/nodes"),
            Some(serde_json::json!({"name":"HUB1","node_type":"ethernet_hub"})),
        )
        .await;
        if status != StatusCode::CREATED {
            return Err(format!("create hub returned {status}: {hub}"));
        }
        let hub_id = hub["node_id"]
            .as_str()
            .ok_or_else(|| "hub has no ID".to_owned())?
            .to_owned();

        let (status, docker_node) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{pid}/nodes"),
            Some(serde_json::json!({
                "name":"Docker hub host","node_type":"docker","image":"alpine:latest",
                "start_command":"/bin/sh -c 'sleep 3600'","adapters":1
            })),
        )
        .await;
        if status != StatusCode::CREATED {
            return Err(format!(
                "create Docker node returned {status}: {docker_node}"
            ));
        }
        let docker_id = docker_node["node_id"]
            .as_str()
            .ok_or_else(|| "Docker node has no ID".to_owned())?
            .to_owned();
        let container_id = docker_node["container_id"]
            .as_str()
            .ok_or_else(|| "Docker node has no container ID".to_owned())?
            .to_owned();
        docker_node_id = Some(docker_id.clone());
        docker_container_id = Some(container_id.clone());

        let (status, vpcs_node) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{pid}/nodes"),
            Some(serde_json::json!({
                "name":"VPCS hub host","node_type":"vpcs","startup_script":"ip 10.20.0.2/24"
            })),
        )
        .await;
        if status != StatusCode::CREATED {
            return Err(format!("create VPCS node returned {status}: {vpcs_node}"));
        }
        let vpcs_id = vpcs_node["node_id"]
            .as_str()
            .ok_or_else(|| "VPCS node has no ID".to_owned())?
            .to_owned();
        vpcs_node_id = Some(vpcs_id.clone());

        for (port, host_id) in [(0_u64, docker_id.as_str()), (1, vpcs_id.as_str())] {
            let (status, link) = request(
                &app,
                Method::POST,
                format!("/v3/projects/{pid}/links"),
                Some(serde_json::json!({"nodes":[
                    {"node_id":hub_id,"adapter_number":0,"port_number":port},
                    {"node_id":host_id,"adapter_number":0,"port_number":0}
                ]})),
            )
            .await;
            if status != StatusCode::CREATED {
                return Err(format!("create hub link returned {status}: {link}"));
            }
            endpoints.extend(link["nodes"].as_array().into_iter().flatten().cloned());
        }

        for (node_id, name) in [(&docker_id, "Docker hub host"), (&vpcs_id, "VPCS hub host")] {
            let (status, body) = request(
                &app,
                Method::POST,
                format!("/v3/projects/{pid}/nodes/{node_id}/start"),
                Some(serde_json::json!({})),
            )
            .await;
            if status != StatusCode::OK || body["status"] != "started" {
                return Err(format!("start {name} returned {status}: {body}"));
            }
        }

        for arguments in [
            vec!["exec", &container_id, "ip", "link", "set", "eth0", "up"],
            vec![
                "exec",
                &container_id,
                "ip",
                "addr",
                "add",
                "10.20.0.1/24",
                "dev",
                "eth0",
            ],
        ] {
            let output = Command::new(&docker_binary)
                .args(arguments)
                .output()
                .await
                .map_err(|error| format!("Docker command failed: {error}"))?;
            if !output.status.success() {
                return Err(format!(
                    "configure Docker hub host failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
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
        for attempt in 0..2 {
            console
                .write_all(b"ping 10.20.0.1 -c 1\r")
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
            .map_err(|_| "VPCS hub ping timed out")?
            .map_err(|error| error.to_string())?;
            if String::from_utf8_lossy(&output).contains("bytes from") {
                break;
            }
            if attempt == 1 {
                return Err(format!(
                    "VPCS could not ping Docker through hub: {}",
                    String::from_utf8_lossy(&output)
                ));
            }
        }

        let output = Command::new(&docker_binary)
            .args([
                "exec",
                &container_id,
                "ping",
                "-c",
                "1",
                "-W",
                "2",
                "10.20.0.2",
            ])
            .output()
            .await
            .map_err(|error| format!("Docker ping could not run: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "Docker could not ping VPCS through hub: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Ok(())
    }
    .await;

    let cleanup = if let Some(pid) = project_id.as_deref() {
        let (status, body) =
            request(&app, Method::DELETE, format!("/v3/projects/{pid}"), None).await;
        (status == StatusCode::NO_CONTENT)
            .then_some(())
            .ok_or_else(|| format!("hub live test cleanup returned {status}: {body}"))
    } else {
        Ok(())
    };
    for endpoint in &endpoints {
        let endpoint_id = endpoint["runtime_endpoint_id"].as_str().unwrap_or_default();
        assert!(
            !ubridge.has_link_endpoint(endpoint_id).await,
            "uBridge endpoint remained after hub cleanup: {endpoint_id}"
        );
    }
    if let Some(node_id) = vpcs_node_id.as_deref() {
        assert_eq!(
            vpcs.status(node_id).await.expect("inspect VPCS cleanup"),
            "stopped"
        );
    }
    if let Some(node_id) = docker_node_id.as_deref() {
        if let Some(endpoint) = endpoints
            .iter()
            .find(|endpoint| endpoint["node_id"] == node_id)
        {
            let host = crate::ubridge::host_veth_name(
                endpoint["runtime_endpoint_id"].as_str().unwrap_or_default(),
            )
            .expect("Docker host veth name");
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
    if let Some(container_id) = docker_container_id.as_deref() {
        let output = Command::new(&docker_binary)
            .args(["inspect", container_id])
            .output()
            .await
            .expect("inspect removed Docker hub host");
        assert!(
            !output.status.success(),
            "Docker container remained: {container_id}"
        );
    }
    assert!(run.is_ok(), "builtin hub live test failed: {run:?}");
    assert!(cleanup.is_ok(), "builtin hub cleanup failed: {cleanup:?}");
}
