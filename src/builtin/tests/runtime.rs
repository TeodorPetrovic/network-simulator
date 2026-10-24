use super::*;

    #[tokio::test]
    #[ignore = "requires upstream uBridge with CAP_NET_ADMIN and CAP_NET_RAW"]
    async fn cloud_udp_nio_exchanges_frames_with_the_link_engine() {
        use std::{env, path::PathBuf};

        use crate::{links::LinkEngine, ubridge::UbridgeRuntime};
        use tokio::time::timeout;

        let binary = env::var_os("NETWORK_SIMULATOR_TEST_UBRIDGE")
            .expect("NETWORK_SIMULATOR_TEST_UBRIDGE names a capable upstream uBridge");
        let directory = tempfile::tempdir().expect("temporary uBridge state");
        let ubridge = UbridgeRuntime::with_paths(
            PathBuf::from(binary),
            directory.path().join("ubridge.sock"),
        );
        let engine = LinkEngine::new(ubridge.clone());
        let external_peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind Cloud UDP peer");
        let external_port = external_peer.local_addr().unwrap().port();
        let cloud_local_port = ubridge.reserve_udp_port().await.unwrap();
        let cloud_id = Uuid::new_v4().to_string();
        let peer_id = Uuid::new_v4().to_string();
        let cloud = BuiltinRuntime::normalize_node(json!({
            "node_id": cloud_id,
            "name": "Cloud UDP live test",
            "node_type": "cloud",
            "ports_mapping": [{
                "name": "External UDP",
                "type": "udp",
                "lport": cloud_local_port,
                "rhost": "127.0.0.1",
                "rport": external_port
            }]
        }))
        .expect("normalize Cloud UDP mapping");
        let mut link = json!({
            "link_id": Uuid::new_v4().to_string(),
            "nodes": [
                {"node_id": cloud_id, "node_type": "cloud", "adapter_number": 0, "port_number": 0},
                {"node_id": peer_id, "node_type": "qemu", "adapter_number": 0, "port_number": 0}
            ]
        });
        engine
            .reserve_pair(&mut link)
            .await
            .expect("allocate link NIO ports");
        let endpoints = link["nodes"].as_array().unwrap();
        let cloud_endpoint = endpoints[0].clone();
        let peer_endpoint = endpoints[1].clone();
        let runtime_port = peer_endpoint["runtime_udp_port"].as_u64().unwrap() as u16;
        let adapter_port = peer_endpoint["runtime_adapter_port"].as_u64().unwrap() as u16;
        let peer_runtime = UdpSocket::bind((Ipv4Addr::LOCALHOST, runtime_port))
            .await
            .expect("bind peer runtime NIO");
        let run: Result<(), String> = async {
            let cloud_nio = BuiltinRuntime::adapter_nio(&cloud, &cloud_endpoint)
                .map_err(|error| error.message().to_owned())?;
            engine
                .attach_endpoint(&cloud_endpoint, cloud_nio, &json!({}), false, None)
                .await
                .map_err(|error| error.message().to_owned())?;
            engine
                .attach_endpoint(
                    &peer_endpoint,
                    AdapterNio::Udp {
                        adapter_port,
                        runtime_port,
                    },
                    &json!({}),
                    false,
                    None,
                )
                .await
                .map_err(|error| error.message().to_owned())?;

            let from_cloud = vec![
                0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02, 0, 0, 0, 0, 1, 0x88, 0xb5, 1, 2, 3,
            ];
            external_peer
                .send_to(&from_cloud, (Ipv4Addr::LOCALHOST, cloud_local_port))
                .await
                .map_err(|error| error.to_string())?;
            let mut received = [0u8; 128];
            let (length, _) = timeout(
                Duration::from_secs(3),
                peer_runtime.recv_from(&mut received),
            )
            .await
            .map_err(|_| "Cloud UDP frame did not reach the link peer".to_owned())?
            .map_err(|error| error.to_string())?;
            if received[..length] != from_cloud {
                return Err("Cloud UDP link changed the incoming Ethernet frame".to_owned());
            }

            let from_peer = vec![
                0x02, 0, 0, 0, 0, 1, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x88, 0xb5, 4, 5, 6,
            ];
            peer_runtime
                .send_to(&from_peer, (Ipv4Addr::LOCALHOST, adapter_port))
                .await
                .map_err(|error| error.to_string())?;
            let (length, _) = timeout(
                Duration::from_secs(3),
                external_peer.recv_from(&mut received),
            )
            .await
            .map_err(|_| "Link peer frame did not reach the Cloud UDP peer".to_owned())?
            .map_err(|error| error.to_string())?;
            if received[..length] != from_peer {
                return Err("Cloud UDP link changed the outgoing Ethernet frame".to_owned());
            }
            Ok(())
        }
        .await;

        let _ = engine.detach_endpoint(&cloud_endpoint).await;
        let _ = engine.detach_endpoint(&peer_endpoint).await;
        assert!(
            !ubridge
                .has_link_endpoint(cloud_endpoint["runtime_endpoint_id"].as_str().unwrap())
                .await
        );
        assert!(
            !ubridge
                .has_link_endpoint(peer_endpoint["runtime_endpoint_id"].as_str().unwrap())
                .await
        );
        assert!(run.is_ok(), "Cloud UDP live test failed: {run:?}");
    }

    #[tokio::test]
    #[ignore = "requires Docker with alpine:latest, capable uBridge, and active libvirt default NAT network 'default' bridged as virbr0 with 192.168.122.1/24"]
    async fn nat_live_routes_docker_traffic_through_libvirt() {
        use std::{env, path::PathBuf};

        use crate::{
            links::{LinkEngine, NioAdapter},
            runtime::{DockerRuntime, NodeAction},
            ubridge::UbridgeRuntime,
        };
        use tokio::{process::Command, time::timeout};

        let ubridge_binary = env::var_os("NETWORK_SIMULATOR_TEST_UBRIDGE")
            .expect("NETWORK_SIMULATOR_TEST_UBRIDGE names a capable upstream uBridge");
        let directory = tempfile::tempdir().expect("temporary builtin NAT test state");
        let ubridge = UbridgeRuntime::with_paths(
            PathBuf::from(ubridge_binary),
            directory.path().join("ubridge.sock"),
        );
        let docker = DockerRuntime::new();
        let engine = LinkEngine::new(ubridge.clone());
        let docker_binary = env::var_os("NETWORK_SIMULATOR_DOCKER")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("docker"));
        let project_id = Uuid::new_v4().to_string();
        let nat_id = Uuid::new_v4().to_string();
        let docker_id = Uuid::new_v4().to_string();
        let mut nat = BuiltinRuntime::normalize_node(json!({
            "node_id": nat_id,
            "name": "NAT live test",
            "node_type": "nat",
            "ports_mapping": [{"type":"ethernet", "interface":"virbr0"}]
        }))
        .expect("normalize libvirt NAT node");
        nat["project_id"] = json!(project_id);
        let mut link = json!({
            "link_id": Uuid::new_v4().to_string(),
            "nodes": [
                {"node_id": nat_id, "node_type":"nat", "adapter_number":0, "port_number":0},
                {"node_id": docker_id, "node_type":"docker", "adapter_number":0, "port_number":0}
            ]
        });
        engine
            .reserve_pair(&mut link)
            .await
            .expect("allocate NAT link NIO ports");
        let endpoints = link["nodes"].as_array().unwrap();
        let nat_endpoint = endpoints[0].clone();
        let docker_endpoint = endpoints[1].clone();
        let nat_tap = host_ethernet_tap_name(&nat_endpoint).expect("derive NAT bridge TAP");
        let docker_host = crate::ubridge::host_veth_name(
            docker_endpoint["runtime_endpoint_id"].as_str().unwrap(),
        )
        .expect("derive Docker host veth");
        let docker_guest = crate::ubridge::guest_veth_name(
            docker_endpoint["runtime_endpoint_id"].as_str().unwrap(),
        )
        .expect("derive Docker guest veth");
        let docker_node = Uuid::parse_str(&docker_id).unwrap();
        let mac = format!(
            "02:{:02x}:{:02x}:{:02x}:{:02x}:00",
            docker_node.as_bytes()[0],
            docker_node.as_bytes()[1],
            docker_node.as_bytes()[2],
            docker_node.as_bytes()[3]
        );
        let mut container_id = None;

        let run: Result<(), String> = async {
            let network = Command::new("virsh")
                .args(["net-info", "default"])
                .output()
                .await
                .map_err(|error| format!("Could not inspect libvirt default network: {error}"))?;
            let network_info = String::from_utf8_lossy(&network.stdout);
            let network_field = |name: &str| {
                network_info.lines().any(|line| {
                    line.split_once(':')
                        .is_some_and(|(key, value)| key.trim() == name && value.trim() == "yes")
                })
            };
            if !network.status.success()
                || !network_field("Active")
                || !network_info.lines().any(|line| {
                    line.split_once(':').is_some_and(|(key, value)| {
                        key.trim() == "Bridge" && value.trim() == "virbr0"
                    })
                })
            {
                return Err(format!(
                    "libvirt default network 'default' must be active and use virbr0; virsh net-info returned: {}{}",
                    network_info,
                    String::from_utf8_lossy(&network.stderr)
                ));
            }
            let bridge_address = Command::new("ip")
                .args(["-o", "-4", "addr", "show", "dev", "virbr0"])
                .output()
                .await
                .map_err(|error| format!("Could not inspect virbr0 address: {error}"))?;
            let bridge_address_text = String::from_utf8_lossy(&bridge_address.stdout);
            if !bridge_address.status.success()
                || !bridge_address_text.contains("192.168.122.1/24")
            {
                return Err(format!(
                    "libvirt default network 'default' must configure 192.168.122.1/24 on virbr0; ip addr returned: {}{}",
                    bridge_address_text,
                    String::from_utf8_lossy(&bridge_address.stderr)
                ));
            }
            BuiltinRuntime::validate_host_interfaces(&nat)
                .map_err(|error| format!("configured NAT interface failed validation: {error}"))?;
            let (created, _) = docker
                .create_container(
                    &project_id,
                    &docker_id,
                    "alpine:latest",
                    "Built-in NAT live test",
                    Some("/bin/sh -c 'sleep 3600'"),
                    None,
                    None,
                )
                .await
                .map_err(|error| error.message().to_owned())?;
            container_id = Some(created.clone());
            docker
                .lifecycle(&created, NodeAction::Start)
                .await
                .map_err(|error| error.message().to_owned())?;
            let pid = docker
                .container_pid(&created)
                .await
                .map_err(|error| error.message().to_owned())?;
            ubridge
                .create_veth_pair(&docker_host, &docker_guest)
                .await
                .map_err(|error| error.message().to_owned())?;
            ubridge
                .set_docker_mac_address(&docker_guest, &mac)
                .await
                .map_err(|error| error.message().to_owned())?;
            ubridge
                .set_interface_up(&docker_guest)
                .await
                .map_err(|error| error.message().to_owned())?;
            ubridge
                .set_interface_up(&docker_host)
                .await
                .map_err(|error| error.message().to_owned())?;
            ubridge
                .move_veth_to_container(&docker_guest, pid, "eth0")
                .await
                .map_err(|error| error.message().to_owned())?;

            let nat_nio = BuiltinRuntime::adapter_nio(&nat, &nat_endpoint)
                .map_err(|error| error.message().to_owned())?;
            let attach_started = Instant::now();
            engine
                .attach_endpoint(&nat_endpoint, nat_nio, &json!({}), false, None)
                .await
                .map_err(|error| error.message().to_owned())?;
            eprintln!("NAT TAP attachment reached forwarding after {:?}", attach_started.elapsed());
            let bridge_ports = Command::new("bridge")
                .args(["link", "show", "master", "virbr0"])
                .output()
                .await
                .map_err(|error| format!("Could not inspect virbr0 bridge ports: {error}"))?;
            if !String::from_utf8_lossy(&bridge_ports.stdout).contains(&nat_tap) {
                return Err(format!(
                    "Cloud/NAT TAP {nat_tap} is not enslaved to virbr0: {}",
                    String::from_utf8_lossy(&bridge_ports.stdout)
                ));
            }
            // Attachment must return only after the libvirt bridge can
            // forward the guest's first ARP packet.
            let bridge_state_path = format!("/sys/class/net/{nat_tap}/brport/state");
            let bridge_state = std::fs::read_to_string(&bridge_state_path)
                .map_err(|error| format!("Could not read bridge state for {nat_tap}: {error}"))?;
            if bridge_state.trim() != "3" {
                return Err(format!("NAT TAP {nat_tap} was reported ready in bridge state {}", bridge_state.trim()));
            }
            let docker_nio = docker
                .adapter_nio(&docker_endpoint)
                .map_err(|error| error.message().to_owned())?;
            engine
                .attach_endpoint(&docker_endpoint, docker_nio, &json!({}), false, None)
                .await
                .map_err(|error| error.message().to_owned())?;
            let capture_paths = [
                (
                    nat_endpoint["runtime_endpoint_id"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    directory.path().join("nat-endpoint.pcap"),
                ),
                (
                    docker_endpoint["runtime_endpoint_id"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    directory.path().join("docker-endpoint.pcap"),
                ),
            ];
            for (endpoint_id, path) in &capture_paths {
                engine
                    .start_capture(
                        &json!({"runtime_endpoint_id": endpoint_id}),
                        &path.to_string_lossy(),
                    )
                    .await
                    .map_err(|error| error.message().to_owned())?;
            }

            for arguments in [
                vec!["ip", "link", "set", "eth0", "up"],
                vec!["ip", "addr", "add", "192.168.122.250/24", "dev", "eth0"],
                vec!["ip", "route", "replace", "default", "via", "192.168.122.1"],
            ] {
                let output = Command::new(&docker_binary)
                    .args(["exec", &created])
                    .args(arguments)
                    .output()
                    .await
                    .map_err(|error| {
                        format!("Could not configure Alpine NAT interface: {error}")
                    })?;
                if !output.status.success() {
                    return Err(format!(
                        "Could not configure Alpine NAT interface: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    ));
                }
            }

            for destination in ["192.168.122.1", "1.1.1.1"] {
                let output = timeout(
                    Duration::from_secs(8),
                    Command::new(&docker_binary)
                        .args(["exec", &created, "ping", "-c", "1", "-W", "5", destination])
                        .output(),
                )
                .await
                .map_err(|_| format!("Alpine ping to {destination} timed out"))?
                .map_err(|error| format!("Alpine ping to {destination} could not run: {error}"))?;
                if !output.status.success() {
                    let neighbors = Command::new(&docker_binary)
                        .args(["exec", &created, "ip", "neigh", "show", "dev", "eth0"])
                        .output()
                        .await
                        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                        .unwrap_or_default();
                    let tap_state = Command::new("ip")
                        .args(["-d", "link", "show", "dev", &nat_tap])
                        .output()
                        .await
                        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                        .unwrap_or_default();
                    let tap_stats = Command::new("ip")
                        .args(["-s", "link", "show", "dev", &nat_tap])
                        .output()
                        .await
                        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                        .unwrap_or_default();
                    let bridge_stats = Command::new("ip")
                        .args(["-s", "link", "show", "dev", "virbr0"])
                        .output()
                        .await
                        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                        .unwrap_or_default();
                    let host_neighbors = Command::new("ip")
                        .args(["neigh", "show", "dev", "virbr0"])
                        .output()
                        .await
                        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                        .unwrap_or_default();
                    let fdb = Command::new("bridge")
                        .args(["fdb", "show", "br", "virbr0"])
                        .output()
                        .await
                        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                        .unwrap_or_default();
                    let mut capture_output = String::new();
                    for (endpoint_id, path) in &capture_paths {
                        let _ = engine
                            .stop_capture(&json!({"runtime_endpoint_id": endpoint_id}))
                            .await;
                        let capture = Command::new("tcpdump")
                            .args(["-nn", "-e", "-tt", "-r"])
                            .arg(path)
                            .arg("arp or icmp")
                            .output()
                            .await
                            .map(|output| {
                                format!(
                                    "{}{}",
                                    String::from_utf8_lossy(&output.stdout),
                                    String::from_utf8_lossy(&output.stderr)
                                )
                            })
                            .unwrap_or_else(|error| format!("capture decode failed: {error}"));
                        capture_output.push_str(&format!("\n{endpoint_id}:\n{capture}"));
                    }
                    return Err(format!(
                        "Alpine could not ping {destination}: {}{}; guest_neighbors={neighbors}; host_virbr0_neighbors={host_neighbors}; tap={tap_state}; tap_stats={tap_stats}; virbr0_stats={bridge_stats}; virbr0_fdb={fdb}; captures={capture_output}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    ));
                }
            }
            Ok(())
        }
        .await;

        let _ = engine.detach_endpoint(&nat_endpoint).await;
        let _ = engine.detach_endpoint(&docker_endpoint).await;
        let _ = ubridge.delete_veth(&docker_host).await;
        if let Some(container_id) = container_id.as_deref() {
            let _ = docker
                .remove_node_resources(&docker_id, Some(container_id))
                .await;
        }
        let nat_tap_remaining = Command::new("ip")
            .args(["link", "show", "dev", &nat_tap])
            .output()
            .await
            .expect("inspect cleaned NAT TAP");
        let docker_veth_remaining = Command::new("ip")
            .args(["link", "show", "dev", &docker_host])
            .output()
            .await
            .expect("inspect cleaned Docker veth");
        assert!(
            !nat_tap_remaining.status.success(),
            "NAT bridge TAP remained: {nat_tap}"
        );
        assert!(
            !docker_veth_remaining.status.success(),
            "Docker host veth remained: {docker_host}"
        );
        assert!(
            !ubridge
                .has_link_endpoint(nat_endpoint["runtime_endpoint_id"].as_str().unwrap())
                .await
        );
        assert!(
            !ubridge
                .has_link_endpoint(docker_endpoint["runtime_endpoint_id"].as_str().unwrap())
                .await
        );
        assert!(run.is_ok(), "NAT live test failed: {run:?}");
    }
}
