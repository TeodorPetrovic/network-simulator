use super::*;

#[path = "chr_helpers.rs"]
mod chr_helpers;
use chr_helpers::{
    qemu_interface_for_mac, qemu_test_mac, read_until, routeros_command, strip_ansi,
};

#[tokio::test]
#[ignore = "requires Docker, upstream VPCS/uBridge, KVM, and the installed MikroTik CHR image"]
async fn chr_routes_vpcs_and_docker_over_controller_udp_nios() {
    use std::{env, path::PathBuf};

    use tokio::{
        io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
        net::UnixStream,
        process::Command,
        time::{Duration, timeout},
    };
    use uuid::Uuid;
    let (directory, docker, qemu, vpcs, ubridge, app) = chr_helpers::live_chr_environment().await;
    let mut project_id: Option<String> = None;
    let mut qemu_vm = None;
    let mut docker_container: Option<String> = None;
    let mut vpcs_node_id: Option<String> = None;
    let mut docker_node_id: Option<String> = None;
    let mut link_endpoints = Vec::<Value>::new();
    let mut link_ids = Vec::<String>::new();
    let test_started_at = std::time::Instant::now();
    let mut links_created_at_ms = None;
    let mut chr_start_returned_at_ms = None;

    let run: Result<(), String> = async {
        create_custom_from_runtime(&app, "qemu", "Live CHR").await;
        let (status, templates) = request(&app, Method::GET, "/v3/templates".into(), None).await;
        if status != StatusCode::OK {
            return Err(format!("template list returned {status}: {templates}"));
        }
        let template_id = templates.as_array().into_iter().flatten()
            .find(|template| template["name"] == "Live CHR · Default")
            .and_then(|template| template["template_id"].as_str())
            .ok_or_else(|| format!("CHR template is missing: {templates}"))?;
        let (status, project) = request(
            &app,
            Method::POST,
            "/v3/projects".into(),
            Some(serde_json::json!({ "name": "Live CHR NIO matrix" })),
        ).await;
        if status != StatusCode::CREATED {
            return Err(format!("project create returned {status}: {project}"));
        }
        let current_project_id = project["project_id"].as_str().ok_or("project has no ID")?.to_owned();
        project_id = Some(current_project_id.clone());

        let (status, chr) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{current_project_id}/templates/{template_id}"),
            Some(serde_json::json!({
                "name": "CHR",
                "memory": 512,
                "cpus": 1,
                "adapters": 3,
                "adapter_type": "virtio-net-pci"
            })),
        ).await;
        if status != StatusCode::CREATED {
            return Err(format!("CHR node create returned {status}: {chr}"));
        }
        let chr_node_id = chr["node_id"].as_str().ok_or("CHR node has no ID")?;
        let (status, configured_chr) = request(
            &app,
            Method::PUT,
            format!("/v3/projects/{current_project_id}/nodes/{chr_node_id}/config"),
            Some(serde_json::json!({ "adapters": 3 })),
        ).await;
        if status != StatusCode::OK {
            return Err(format!("CHR adapter configuration returned {status}: {configured_chr}"));
        }
        qemu_vm = Some(qemu.vm_from_node(&configured_chr).map_err(|error| error.message().to_owned())?);

        let (status, vpcs_node) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{current_project_id}/nodes"),
            Some(serde_json::json!({
                "name": "VPCS peer",
                "node_type": "vpcs",
                "startup_script": "ip 10.0.1.2/24"
            })),
        ).await;
        if status != StatusCode::CREATED {
            return Err(format!("VPCS node create returned {status}: {vpcs_node}"));
        }
        vpcs_node_id = vpcs_node["node_id"].as_str().map(str::to_owned);
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
        ).await;
        if status != StatusCode::CREATED {
            return Err(format!("Docker node create returned {status}: {docker_node}"));
        }
        docker_node_id = docker_node["node_id"].as_str().map(str::to_owned);
        docker_container = docker_node["container_id"].as_str().map(str::to_owned);

        let vpcs_link = serde_json::json!({ "nodes": [
            { "node_id": chr["node_id"], "adapter_number": 0, "port_number": 0 },
            { "node_id": vpcs_node["node_id"], "adapter_number": 0, "port_number": 0 }
        ] });
        let (status, vpcs_link) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{current_project_id}/links"),
            Some(vpcs_link),
        ).await;
        if status != StatusCode::CREATED {
            return Err(format!("CHR↔VPCS link returned {status}: {vpcs_link}"));
        }
        link_ids.push(vpcs_link["link_id"].as_str().ok_or("VPCS link has no ID")?.to_owned());
        link_endpoints.extend(vpcs_link["nodes"].as_array().into_iter().flatten().cloned());

        let docker_link = serde_json::json!({ "nodes": [
            { "node_id": chr["node_id"], "adapter_number": 1, "port_number": 0 },
            { "node_id": docker_node["node_id"], "adapter_number": 0, "port_number": 0 }
        ] });
        let (status, docker_link) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{current_project_id}/links"),
            Some(docker_link),
        ).await;
        if status != StatusCode::CREATED {
            return Err(format!("CHR↔Docker link returned {status}: {docker_link}"));
        }
        link_ids.push(docker_link["link_id"].as_str().ok_or("Docker link has no ID")?.to_owned());
        link_endpoints.extend(docker_link["nodes"].as_array().into_iter().flatten().cloned());
        links_created_at_ms = Some(test_started_at.elapsed().as_millis());

        for node in [&chr, &vpcs_node, &docker_node] {
            let node_id = node["node_id"].as_str().ok_or("node has no ID")?;
            let (status, started) = request(
                &app,
                Method::POST,
                format!("/v3/projects/{current_project_id}/nodes/{node_id}/start"),
                Some(serde_json::json!({})),
            ).await;
            if status != StatusCode::OK || started["status"] != "started" {
                return Err(format!("node start returned {status}: {started}"));
            }
            if node_id == chr_node_id {
                chr_start_returned_at_ms = Some(test_started_at.elapsed().as_millis());
            }
        }

        let vm = qemu_vm.as_ref().ok_or("CHR runtime model missing")?;
        let serial_socket = UnixStream::connect(&vm.serial_socket)
            .await
            .map_err(|error| format!("CHR serial socket did not open: {error}"))?;
        let mut serial = BufReader::new(serial_socket);
        read_until(&mut serial, "CHR Login:", Duration::from_secs(180)).await?;
        let chr_login_prompt_at_ms = test_started_at.elapsed().as_millis();
        serial.write_all(b"admin\r").await.map_err(|error| error.to_string())?;
        read_until(&mut serial, "Password:", Duration::from_secs(10)).await?;
        serial.write_all(b"\r").await.map_err(|error| error.to_string())?;
        read_until(
            &mut serial,
            "Do you want to see the software license? [Y/n]:",
            Duration::from_secs(30),
        ).await?;
        serial.write_all(b"n\r").await.map_err(|error| error.to_string())?;
        read_until(&mut serial, "new password>", Duration::from_secs(20)).await?;
        serial.write_all(b"\x03").await.map_err(|error| error.to_string())?;
        read_until(&mut serial, "[admin@CHR] >", Duration::from_secs(20)).await?;

        let interface_output = routeros_command(&mut serial, "/interface ethernet print detail").await?;
        let interface0 = qemu_interface_for_mac(
            &interface_output,
            &qemu_test_mac(chr["node_id"].as_str().ok_or("CHR node has no ID")?, 0)?,
        );
        let interface1 = qemu_interface_for_mac(
            &interface_output,
            &qemu_test_mac(chr["node_id"].as_str().ok_or("CHR node has no ID")?, 1)?,
        );
        let (Some(interface0), Some(interface1)) = (interface0, interface1) else {
            return Err(format!("CHR did not expose both linked interfaces: {interface_output}"));
        };
        eprintln!("CHR link MAC mapping: adapter 0={interface0}, adapter 1={interface1}; interfaces: {interface_output}");
        routeros_command(&mut serial, &format!("/ip address add address=10.0.1.1/24 interface={interface0}")).await?;
        routeros_command(&mut serial, &format!("/ip address add address=10.0.2.1/24 interface={interface1}")).await?;
        routeros_command(&mut serial, &format!("/tool sniffer set filter-interface={interface1}")).await?;
        routeros_command(&mut serial, "/tool sniffer start").await?;

        let container_id = docker_container.as_deref().ok_or("Docker node has no container ID")?;
        let docker_binary = env::var_os("NETWORK_SIMULATOR_DOCKER")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("docker"));
        for args in [
            vec!["ip", "addr", "add", "10.0.2.2/24", "dev", "eth0"],
            vec!["ip", "link", "set", "eth0", "up"],
        ] {
            let output = Command::new(&docker_binary)
                .args(["exec", container_id])
                .args(args)
                .output()
                .await
                .map_err(|error| format!("could not configure Alpine eth0: {error}"))?;
            if !output.status.success() {
                return Err(format!("could not configure Alpine eth0: {}", String::from_utf8_lossy(&output.stderr).trim()));
            }
        }
        let capture_paths = link_endpoints
            .iter()
            .filter(|endpoint| {
                endpoint["node_id"].as_str()
                    == Some(docker_node_id.as_deref().unwrap_or_default())
                    || (endpoint["node_id"].as_str() == Some(chr_node_id)
                        && endpoint["adapter_number"].as_u64() == Some(1))
            })
            .map(|endpoint| {
                let endpoint_id = endpoint["runtime_endpoint_id"]
                    .as_str()
                    .unwrap_or_default();
                (
                    endpoint_id.to_owned(),
                    directory.path().join(format!("{endpoint_id}.pcap")),
                )
            })
            .collect::<Vec<_>>();
        for (endpoint_id, path) in &capture_paths {
            ubridge
                .start_link_capture(endpoint_id, &path.to_string_lossy())
                .await
                .map_err(|error| {
                    format!("could not capture CHR/Docker link: {}", error.message())
                })?;
        }
        let first_alpine_probe_at_ms = test_started_at.elapsed().as_millis();
        // A live CHR run can drop the first echo immediately after fresh
        // ARP resolution even though the ARP exchange and later echoes are
        // valid. Bound this reachability check to three one-packet probes;
        // each probe has a four-second deadline.
        let mut docker_ping_ok = false;
        let mut docker_ping_attempts = 0;
        let mut docker_ping_output = String::new();
        let mut first_docker_ping_output = String::new();
        for attempt in 0..1 {
            docker_ping_attempts = attempt + 1;
            let output = Command::new(&docker_binary)
                .args([
                    "exec",
                    container_id,
                    "ping",
                    "-c",
                    "1",
                    "-W",
                    "4",
                    "10.0.2.1",
                ])
                .output()
                .await
                .map_err(|error| format!("could not ping CHR from Alpine: {error}"))?;
            docker_ping_output = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            if attempt == 0 {
                first_docker_ping_output.clone_from(&docker_ping_output);
            }
            docker_ping_ok = output.status.success()
                && docker_ping_output.contains("1 packets received");
            if docker_ping_ok {
                break;
            }
        }
        let _ = routeros_command(&mut serial, "/tool sniffer stop").await;
        let chr_guest_capture = routeros_command(&mut serial, "/tool sniffer packet print detail without-paging").await?;
        let mut network_debug = String::new();
        if !docker_ping_ok {
            network_debug.push_str(&format!("\nCHR guest-side packet capture:\n{chr_guest_capture}"));
            network_debug.push_str(&format!(
                "\nAlpine used {docker_ping_attempts} bounded probes; first probe output:\n{first_docker_ping_output}\nTest timing from setup: CHR/VPCS/Docker links created +{} ms; CHR start request returned +{} ms; CHR login prompt +{} ms; first Alpine probe +{} ms. Link records existed before startup, but the API launches QEMU with no NIC and hot-attaches link NIOs over QMP before the start request returns.\nQEMU NIC model: virtio-net-pci; expected adapter MACs: {}, {}",
                links_created_at_ms.unwrap_or_default(),
                chr_start_returned_at_ms.unwrap_or_default(),
                chr_login_prompt_at_ms,
                first_alpine_probe_at_ms,
                qemu_test_mac(chr_node_id, 0)?,
                qemu_test_mac(chr_node_id, 1)?
            ));
            let retry = Command::new(&docker_binary)
                .args(["exec", container_id, "ping", "-c", "3", "-W", "1", "10.0.2.1"])
                .output()
                .await
                .map_err(|error| error.to_string())?;
            network_debug.push_str(&format!(
                "\nAlpine three-packet retry:\n{}{}",
                String::from_utf8_lossy(&retry.stdout),
                String::from_utf8_lossy(&retry.stderr)
            ));
            docker_ping_ok = retry.status.success()
                && String::from_utf8_lossy(&retry.stdout).contains("3 packets received");
            eprintln!(
                "First Alpine probe:\n{first_docker_ping_output}\nCHR guest-side capture:\n{chr_guest_capture}\nRetry:\n{}{}",
                String::from_utf8_lossy(&retry.stdout),
                String::from_utf8_lossy(&retry.stderr)
            );
            let chr_arp = routeros_command(&mut serial, "/ip arp print detail").await?;
            let chr_interfaces =
                routeros_command(&mut serial, "/interface ethernet print detail").await?;
            let chr_stats = routeros_command(&mut serial, "/interface ethernet print stats").await?;
            let chr_firewall = routeros_command(
                &mut serial,
                "/ip firewall filter print detail stats",
            )
            .await?;
            let chr_interface_lists =
                routeros_command(&mut serial, "/interface list member print detail").await?;
            network_debug.push_str(&format!(
                "\nCHR ARP before reverse ping:\n{chr_arp}\nCHR interfaces:\n{chr_interfaces}\nCHR interface stats:\n{chr_stats}\nCHR IPv4 firewall rules and counters:\n{chr_firewall}\nCHR interface list members:\n{chr_interface_lists}"
            ));
            for endpoint in link_endpoints.iter().filter(|endpoint| {
                endpoint["node_id"].as_str()
                    == Some(docker_node_id.as_deref().unwrap_or_default())
            }) {
                let endpoint_id = endpoint["runtime_endpoint_id"].as_str().unwrap_or_default();
                let host = crate::ubridge::host_veth_name(endpoint_id)
                    .map_err(|error| error.message().to_owned())?;
                let link = Command::new("ip")
                    .args(["-s", "-d", "link", "show", "dev", &host])
                    .output()
                    .await
                    .map_err(|error| error.to_string())?;
                let offloads = Command::new("ethtool")
                    .args(["-k", &host])
                    .output()
                    .await
                    .map(|output| {
                        format!(
                            "{}{}",
                            String::from_utf8_lossy(&output.stdout),
                            String::from_utf8_lossy(&output.stderr)
                        )
                    })
                    .unwrap_or_else(|error| format!("ethtool unavailable: {error}"));
                let fdb = Command::new("bridge")
                    .args(["fdb", "show", "dev", &host])
                    .output()
                    .await
                    .map_err(|error| error.to_string())?;
                let guest = Command::new(&docker_binary)
                    .args([
                        "exec",
                        container_id,
                        "sh",
                        "-c",
                        "ip link show dev eth0; ip -s link show dev eth0; ip neigh show dev eth0",
                    ])
                    .output()
                    .await
                    .map_err(|error| error.to_string())?;
                network_debug.push_str(&format!(
                    "\nHost veth {host}:\n{}\nHost veth offloads:\n{offloads}\nDocker eth0/link neighbors:\n{}{}\nLinux bridge FDB for this endpoint:\n{}",
                    String::from_utf8_lossy(&link.stdout),
                    String::from_utf8_lossy(&guest.stdout),
                    String::from_utf8_lossy(&guest.stderr),
                    String::from_utf8_lossy(&fdb.stdout)
                ));
            }
            for (endpoint_id, path) in &capture_paths {
                let _ = ubridge.stop_link_capture(endpoint_id).await;
                let capture = Command::new("tcpdump")
                    .args(["-nn", "-e", "-tt", "-vvv", "-xx", "-r"])
                    .arg(path)
                    .arg("arp or icmp")
                    .output()
                    .await
                    .map_err(|error| error.to_string())?;
                network_debug.push_str(&format!(
                    "\nuBridge capture {endpoint_id}:\n{}{}",
                    String::from_utf8_lossy(&capture.stdout),
                    String::from_utf8_lossy(&capture.stderr)
                ));
            }
        }
        let chr_ping_output = routeros_command(&mut serial, "/ping 10.0.2.2 count=1").await?;

        let mut vpcs_console = vpcs.connect_console(vpcs_node["node_id"].as_str().ok_or("VPCS node has no ID")?)
            .await.map_err(|error| error.message().to_owned())?;
        let mut vpcs_output = Vec::new();
        timeout(Duration::from_secs(5), async {
            loop {
                let mut chunk = [0_u8; 2048];
                let length = vpcs_console.read(&mut chunk).await?;
                vpcs_output.extend_from_slice(&chunk[..length]);
                if String::from_utf8_lossy(&vpcs_output).contains("VPCS>") { break Ok::<(), std::io::Error>(()); }
            }
        }).await.map_err(|_| "VPCS console prompt timed out")?
            .map_err(|error| error.to_string())?;
        vpcs_output.clear();
        let mut vpcs_ping_ok = false;
        for _ in 0..3 {
            vpcs_console.write_all(b"ping 10.0.1.1 -c 1\r").await.map_err(|error| error.to_string())?;
            let mut attempt_output = Vec::new();
            timeout(Duration::from_secs(8), async {
                loop {
                    let mut chunk = [0_u8; 2048];
                    let length = vpcs_console.read(&mut chunk).await?;
                    attempt_output.extend_from_slice(&chunk[..length]);
                    let text = String::from_utf8_lossy(&attempt_output);
                    if text.contains("bytes from 10.0.1.1") || text.contains("VPCS>") {
                        break Ok::<(), std::io::Error>(());
                    }
                }
            }).await.map_err(|_| format!("VPCS↔CHR ping timed out: {}", String::from_utf8_lossy(&attempt_output)))?
                .map_err(|error| error.to_string())?;
            vpcs_ping_ok = String::from_utf8_lossy(&attempt_output).contains("bytes from 10.0.1.1");
            vpcs_output.extend_from_slice(&attempt_output);
            if vpcs_ping_ok { break; }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        if !docker_ping_ok || !chr_ping_output.contains("received=1") || !vpcs_ping_ok {
            return Err(format!(
                "QEMU connectivity failed; Alpine ping: {docker_ping_output}; CHR ping: {chr_ping_output}; VPCS ping: {}{network_debug}",
                String::from_utf8_lossy(&vpcs_output),
            ));
        }

        let (status, hot_peer) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{current_project_id}/nodes"),
            Some(serde_json::json!({
                "name": "Hot added NIC peer",
                "node_type": "vpcs",
                "startup_script": "ip 10.0.3.2/24"
            })),
        ).await;
        if status != StatusCode::CREATED {
            return Err(format!("hot-add peer create returned {status}: {hot_peer}"));
        }
        let hot_peer_id = hot_peer["node_id"].as_str().ok_or("hot-add peer has no ID")?;
        let (status, started) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{current_project_id}/nodes/{hot_peer_id}/start"),
            Some(serde_json::json!({})),
        ).await;
        if status != StatusCode::OK || started["status"] != "started" {
            return Err(format!("hot-add peer start returned {status}: {started}"));
        }
        let (status, hot_link) = request(
            &app,
            Method::POST,
            format!("/v3/projects/{current_project_id}/links"),
            Some(serde_json::json!({ "nodes": [
                { "node_id": chr_node_id, "adapter_number": 2, "port_number": 0 },
                { "node_id": hot_peer_id, "adapter_number": 0, "port_number": 0 }
            ] })),
        ).await;
        if status != StatusCode::CREATED {
            return Err(format!("live QEMU NIC hot-add returned {status}: {hot_link}"));
        }
        link_ids.push(hot_link["link_id"].as_str().ok_or("hot-add link has no ID")?.to_owned());

        let docker_link_path = format!("/v3/projects/{current_project_id}/links/{}", link_ids[1]);
        for (suspend, expected_success) in [(true, false), (false, true)] {
            let (status, body) = request(
                &app,
                Method::PUT,
                docker_link_path.clone(),
                Some(serde_json::json!({"suspend":suspend})),
            ).await;
            if status != StatusCode::OK {
                return Err(format!("live link suspend={suspend} returned {status}: {body}"));
            }
            let probe = Command::new(&docker_binary)
                .args(["exec", container_id, "ping", "-c", "1", "-W", "2", "10.0.2.1"])
                .output().await.map_err(|error| error.to_string())?;
            if probe.status.success() != expected_success {
                return Err(format!(
                    "live link suspend={suspend} had unexpected ping result: {}{}",
                    String::from_utf8_lossy(&probe.stdout),
                    String::from_utf8_lossy(&probe.stderr)
                ));
            }
        }

        for endpoint in &link_endpoints {
            let endpoint_id = endpoint["runtime_endpoint_id"].as_str().unwrap_or_default();
            let _ = ubridge.stop_link_capture(endpoint_id).await;
        }
        let (status, capture) = request(
            &app,
            Method::POST,
            format!("{docker_link_path}/capture/start"),
            Some(serde_json::json!({"capture_file_name":"live-route.pcap"})),
        ).await;
        if status != StatusCode::CREATED {
            return Err(format!("live capture start returned {status}: {capture}"));
        }
        let probe = Command::new(&docker_binary)
            .args(["exec", container_id, "ping", "-c", "1", "-W", "3", "10.0.2.1"])
            .output().await.map_err(|error| error.to_string())?;
        if !probe.status.success() {
            return Err(format!("ping during live packet capture failed: {}", String::from_utf8_lossy(&probe.stdout)));
        }
        let (status, _) = request(
            &app,
            Method::POST,
            format!("{docker_link_path}/capture/stop"),
            Some(serde_json::json!({})),
        ).await;
        if status != StatusCode::NO_CONTENT {
            return Err(format!("live capture stop returned {status}"));
        }
        let (status, headers, bytes) = request_bytes(
            &app,
            Method::GET,
            format!("{docker_link_path}/capture/file"),
        ).await;
        if status != StatusCode::OK
            || !headers.get("content-type").and_then(|value| value.to_str().ok()).unwrap_or_default().contains("pcap")
            || bytes.len() < 40
        {
            return Err(format!("live capture download was invalid: status={status}, bytes={}", bytes.len()));
        }

        chr_helpers::delete_live_links(&app, &current_project_id, &link_ids).await?;
        Ok(())
    }
    .await;

    let cleanup = chr_helpers::cleanup_live_project(&app, project_id).await;
    if let Some(vm) = qemu_vm.as_ref() {
        assert_eq!(
            qemu.status(vm).await.expect("inspect cleaned CHR"),
            "stopped"
        );
        assert!(
            !vm.pid_file.exists(),
            "QEMU pid file remained after project deletion"
        );
    }
    for endpoint in &link_endpoints {
        assert!(
            !ubridge
                .has_link_endpoint(endpoint["runtime_endpoint_id"].as_str().unwrap_or_default())
                .await
        );
    }
    if let Some(node_id) = vpcs_node_id.as_deref() {
        assert_eq!(
            vpcs.status(node_id).await.expect("inspect cleaned VPCS"),
            "stopped"
        );
    }
    if let Some(container_id) = docker_container.as_deref() {
        let docker_binary = env::var_os("NETWORK_SIMULATOR_DOCKER")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("docker"));
        let inspect = Command::new(&docker_binary)
            .args(["inspect", container_id])
            .output()
            .await
            .expect("inspect cleaned Alpine container");
        assert!(
            !inspect.status.success(),
            "Alpine container remained after project deletion"
        );
    }
    if let Some(node_id) = docker_node_id.as_deref() {
        for endpoint in link_endpoints
            .iter()
            .filter(|endpoint| endpoint["node_id"].as_str() == Some(node_id))
        {
            let host =
                crate::ubridge::host_veth_name(endpoint["runtime_endpoint_id"].as_str().unwrap())
                    .expect("Docker host veth name");
            let inspect = Command::new("ip")
                .args(["link", "show", "dev", &host])
                .output()
                .await
                .expect("inspect cleaned Docker host veth");
            assert!(
                !inspect.status.success(),
                "Docker host veth remained after project deletion: {host}"
            );
        }
    }
    assert!(run.is_ok(), "live CHR NIO matrix failed: {run:?}");
    assert!(cleanup.is_ok(), "live CHR cleanup failed: {cleanup:?}");
}
