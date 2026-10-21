
use std::{env, fs, path::PathBuf, time::Duration};

use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::UnixListener,
    process::Command,
    sync::Mutex,
    time::timeout,
};

use super::{
    UbridgeRuntime, endpoint_name, escape_ubridge_arg, guest_veth_name, host_veth_name,
    is_final_reply, stop_existing_ubridge,
};
use crate::{
    runtime::{DockerRuntime, NodeAction, RuntimeError},
    vpcs::VpcsRuntime,
};

#[test]
fn generates_endpoint_and_veth_names_that_fit_linux_limits() {
    let id = "01234567-89ab-cdef-0123-456789abcdef";
    for name in [
        host_veth_name(id).unwrap(),
        guest_veth_name(id).unwrap(),
        endpoint_name(id).unwrap(),
    ] {
        assert!(name.len() <= 15);
        assert!(
            name.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        );
    }
    assert_eq!(super::compact_id(id).unwrap(), "0123456789ab");
}

#[test]
fn recognizes_the_final_line_in_the_upstream_control_protocol() {
    assert!(!is_final_reply("101 bridge is running\n"));
    assert!(is_final_reply("100-bridge started\n"));
    assert!(is_final_reply("206-Could not create bridge\n"));
}

#[tokio::test]
async fn ubridge_arguments_cannot_inject_a_second_command_line() {
    let runtime = UbridgeRuntime::with_paths("unused-ubridge", "/tmp/unused-ubridge.sock");
    let error = runtime
        .command("x\nhypervisor stop\n")
        .await
        .expect_err("control characters must be rejected before connecting");
    assert!(error.message().contains("control characters"));
    assert_eq!(
        escape_ubridge_arg("x\nhypervisor stop\n"),
        "x%0Ahypervisor stop%0A"
    );
}

#[tokio::test]
async fn invalid_filter_replacement_does_not_reset_active_rules_and_frequency_drop_works() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("ubridge.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let commands = std::sync::Arc::new(Mutex::new(Vec::new()));
    let received = commands.clone();
    let server = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let (read, mut write) = stream.into_split();
            let mut reader = BufReader::new(read);
            let mut command = String::new();
            if reader.read_line(&mut command).await.unwrap_or_default() == 0 {
                continue;
            }
            received.lock().await.push(command.trim().to_owned());
            let _ = write.write_all(b"100-OK\n").await;
        }
    });
    let runtime = UbridgeRuntime::with_paths("unused-ubridge", &socket);
    let child = Command::new("sleep")
        .arg("60")
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    *runtime.daemon.lock().await = Some(child);
    let endpoint_id = "01234567-89ab-cdef-0123-456789abcdef";
    runtime
        .link_bridges
        .lock()
        .await
        .insert(endpoint_id.to_owned(), endpoint_name(endpoint_id).unwrap());

    runtime
        .update_link_endpoint(
            endpoint_id,
            &serde_json::json!({"packet_loss": [25]}),
            false,
        )
        .await
        .expect("initial filter set is installed");
    let working_commands = commands.lock().await.clone();
    assert_eq!(working_commands.len(), 2);

    let invalid = runtime
        .update_link_endpoint(endpoint_id, &serde_json::json!({"unsupported": [1]}), false)
        .await;
    assert!(invalid.is_err());
    assert_eq!(*commands.lock().await, working_commands);

    runtime
        .update_link_endpoint(
            endpoint_id,
            &serde_json::json!({"frequency_drop": [3]}),
            false,
        )
        .await
        .expect("frequency_drop is a supported uBridge rule");
    assert_eq!(
        *commands.lock().await,
        vec![
            format!(
                "bridge reset_packet_filters {}",
                endpoint_name(endpoint_id).unwrap()
            ),
            format!(
                "bridge add_packet_filter {} filter0 packet_loss 25",
                endpoint_name(endpoint_id).unwrap()
            ),
            format!(
                "bridge reset_packet_filters {}",
                endpoint_name(endpoint_id).unwrap()
            ),
            format!(
                "bridge add_packet_filter {} filter0 frequency_drop 3",
                endpoint_name(endpoint_id).unwrap()
            )
        ]
    );

    runtime.shutdown().await;
    server.abort();
}

#[tokio::test]
async fn persisted_udp_port_claims_are_idempotent_and_owner_scoped() {
    let runtime = UbridgeRuntime::with_paths("unused-ubridge", "/tmp/unused-ubridge.sock");
    let (allocated_a, allocated_b) = tokio::join!(
        runtime.reserve_udp_port_for("allocated-a"),
        runtime.reserve_udp_port_for("allocated-b")
    );
    let allocated_a = allocated_a.unwrap();
    let allocated_b = allocated_b.unwrap();
    assert_ne!(allocated_a, allocated_b);
    runtime
        .release_udp_ports("allocated-a", [allocated_a])
        .await;
    runtime
        .release_udp_ports("allocated-b", [allocated_b])
        .await;

    runtime.claim_udp_port("link-a", 39001).await.unwrap();
    runtime.claim_udp_port("link-a", 39001).await.unwrap();
    assert!(runtime.claim_udp_port("link-b", 39001).await.is_err());
    runtime.release_udp_ports("link-b", [39001]).await;
    assert!(runtime.claim_udp_port("link-b", 39001).await.is_err());
    runtime.release_udp_ports("link-a", [39001]).await;
    runtime.claim_udp_port("link-b", 39001).await.unwrap();
}

#[tokio::test]
async fn startup_recovery_stops_a_surviving_unowned_daemon() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("ubridge.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut reader = BufReader::new(read);
        let mut command = String::new();
        reader.read_line(&mut command).await.unwrap();
        assert_eq!(command, "hypervisor stop\n");
        write.write_all(b"100-OK\n").await.unwrap();
    });

    stop_existing_ubridge(&socket).await.unwrap();
    server.await.unwrap();
    assert!(!socket.exists());
}

#[tokio::test]
async fn shutdown_kills_owned_daemon_and_removes_its_socket() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("run/ubridge.sock");
    fs::create_dir_all(socket.parent().unwrap()).unwrap();
    fs::write(&socket, b"stale control socket").unwrap();
    let runtime = UbridgeRuntime::with_paths("unused-ubridge", &socket);
    let child = Command::new("sleep").arg("60").spawn().unwrap();
    let pid = child.id().unwrap();
    *runtime.daemon.lock().await = Some(child);

    runtime.shutdown().await;

    assert!(!socket.exists());
    assert!(!PathBuf::from(format!("/proc/{pid}")).exists());
    assert!(runtime.daemon.lock().await.is_none());
}

#[tokio::test]
#[ignore = "requires Docker, alpine:latest, upstream VPCS, and uBridge CAP_NET_ADMIN/CAP_NET_RAW"]
async fn alpine_and_vpcs_exchange_icmp_over_reciprocal_udp_nio_bridges() {
    let ubridge_binary = env::var_os("NETWORK_SIMULATOR_TEST_UBRIDGE")
        .expect("NETWORK_SIMULATOR_TEST_UBRIDGE names a capable upstream uBridge binary");
    let vpcs_binary = env::var_os("NETWORK_SIMULATOR_TEST_VPCS")
        .expect("NETWORK_SIMULATOR_TEST_VPCS names the upstream VPCS binary");
    let directory = tempfile::tempdir().expect("temporary uBridge and VPCS state");
    let ubridge = UbridgeRuntime::with_paths(
        PathBuf::from(ubridge_binary),
        directory.path().join("ubridge.sock"),
    );
    let vpcs = VpcsRuntime::with_paths(PathBuf::from(vpcs_binary), directory.path().join("vpcs"));
    let docker = DockerRuntime::new();
    let project_id = uuid::Uuid::new_v4().to_string();
    let vpcs_id = uuid::Uuid::new_v4().to_string();
    let docker_node_id = uuid::Uuid::new_v4().to_string();
    let vpcs_endpoint_id = uuid::Uuid::new_v4().to_string();
    let docker_endpoint_id = uuid::Uuid::new_v4().to_string();
    let vpcs_link_port = ubridge.reserve_udp_port().await.expect("VPCS link port");
    let docker_link_port = ubridge.reserve_udp_port().await.expect("Docker link port");
    let adapter_port = ubridge.reserve_udp_port().await.expect("VPCS adapter port");
    let runtime_port = ubridge.reserve_udp_port().await.expect("VPCS runtime port");
    let docker_host = host_veth_name(&docker_endpoint_id).expect("stable Docker host veth");
    let docker_guest = guest_veth_name(&docker_endpoint_id).expect("stable Docker guest veth");
    let docker_binary = env::var_os("NETWORK_SIMULATOR_DOCKER")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("docker"));
    let mut container_id: Option<String> = None;
    let mut vpcs_started = false;

    let run: Result<(), RuntimeError> = async {
        let (created_container, _) = docker
            .create_container(
                &project_id,
                &docker_node_id,
                "alpine:latest",
                "uBridge Alpine peer",
                Some("/bin/sh -c 'sleep 3600'"),
                None,
                None,
            )
            .await?;
        container_id = Some(created_container.clone());
        docker
            .lifecycle(&created_container, NodeAction::Start)
            .await?;
        let namespace_pid = docker.container_pid(&created_container).await?;
        ubridge
            .create_veth_pair(&docker_host, &docker_guest)
            .await?;
        ubridge
            .set_docker_mac_address(&docker_guest, "02:00:00:00:00:01")
            .await?;
        ubridge.set_interface_up(&docker_guest).await?;
        ubridge.set_interface_up(&docker_host).await?;
        ubridge
            .move_veth_to_container(&docker_guest, namespace_pid, "eth0")
            .await?;

        vpcs.set_nio_ports(&vpcs_id, adapter_port, runtime_port)
            .await;
        vpcs.lifecycle(
            &project_id,
            &vpcs_id,
            "uBridge VPCS peer",
            Some("ip 10.0.0.1/24"),
            NodeAction::Start,
        )
        .await?;
        vpcs_started = true;

        ubridge
            .attach_link_endpoint(
                &docker_endpoint_id,
                docker_link_port,
                vpcs_link_port,
                None,
                None,
                Some(&docker_host),
                &serde_json::json!({}),
                false,
                None,
            )
            .await?;
        ubridge
            .attach_link_endpoint(
                &vpcs_endpoint_id,
                vpcs_link_port,
                docker_link_port,
                Some(adapter_port),
                Some(runtime_port),
                None,
                &serde_json::json!({}),
                false,
                None,
            )
            .await?;

        for args in [
            vec!["ip", "addr", "add", "10.0.0.2/24", "dev", "eth0"],
            vec!["ip", "link", "set", "eth0", "up"],
        ] {
            let output = Command::new(&docker_binary)
                .args(["exec", &created_container])
                .args(args)
                .output()
                .await
                .map_err(|error| {
                    RuntimeError::Unavailable(format!("Could not configure Alpine eth0: {error}"))
                })?;
            if !output.status.success() {
                return Err(RuntimeError::Failed(format!(
                    "Could not configure Alpine eth0: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                )));
            }
        }

        let mut console = vpcs.connect_console(&vpcs_id).await?;
        let mut output = Vec::new();
        timeout(Duration::from_secs(5), async {
            loop {
                let mut chunk = [0_u8; 4096];
                let length = console.read(&mut chunk).await?;
                output.extend_from_slice(&chunk[..length]);
                if String::from_utf8_lossy(&output).contains("VPCS>") {
                    break Ok::<(), std::io::Error>(());
                }
            }
        })
        .await
        .map_err(|_| RuntimeError::Unavailable("VPCS prompt timed out".to_owned()))?
        .map_err(|error| RuntimeError::Unavailable(format!("VPCS console failed: {error}")))?;
        console
            .write_all(b"ping 10.0.0.2 -c 1\r")
            .await
            .map_err(|error| RuntimeError::Unavailable(format!("VPCS console failed: {error}")))?;
        let mut ping_output = Vec::new();
        timeout(Duration::from_secs(15), async {
            loop {
                let mut chunk = [0_u8; 4096];
                let length = console.read(&mut chunk).await?;
                ping_output.extend_from_slice(&chunk[..length]);
                if String::from_utf8_lossy(&ping_output).contains("bytes from 10.0.0.2") {
                    break Ok::<(), std::io::Error>(());
                }
            }
        })
        .await
        .map_err(|_| {
            RuntimeError::Unavailable(format!(
                "No ICMP reply from Alpine: {}",
                String::from_utf8_lossy(&ping_output).trim()
            ))
        })?
        .map_err(|error| RuntimeError::Unavailable(format!("VPCS console failed: {error}")))?;
        Ok(())
    }
    .await;

    let _ = ubridge.delete_link_endpoint(&vpcs_endpoint_id).await;
    let _ = ubridge.delete_link_endpoint(&docker_endpoint_id).await;
    if vpcs_started {
        let _ = vpcs.stop(&vpcs_id).await;
    }
    if let Some(container_id) = container_id.as_deref() {
        let _ = docker
            .remove_node_resources(&docker_node_id, Some(container_id))
            .await;
    }
    let _ = ubridge.delete_veth(&docker_host).await;
    if let Err(error) = run {
        panic!(
            "real Alpine-to-VPCS UDP NIO test failed: {}",
            error.message()
        );
    }
}

#[tokio::test]
#[ignore = "requires Docker, alpine:latest, and uBridge CAP_NET_ADMIN/CAP_NET_RAW"]
async fn alpine_containers_exchange_icmp_over_reciprocal_udp_nio_bridges() {
    let ubridge_binary = env::var_os("NETWORK_SIMULATOR_TEST_UBRIDGE")
        .expect("NETWORK_SIMULATOR_TEST_UBRIDGE names a capable upstream uBridge binary");
    let directory = tempfile::tempdir().expect("temporary uBridge state");
    let ubridge = UbridgeRuntime::with_paths(
        PathBuf::from(ubridge_binary),
        directory.path().join("ubridge.sock"),
    );
    let docker = DockerRuntime::new();
    let project_id = uuid::Uuid::new_v4().to_string();
    let nodes = [
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    ];
    let endpoints = [
        uuid::Uuid::new_v4().to_string(),
        uuid::Uuid::new_v4().to_string(),
    ];
    let link_ports = [
        ubridge.reserve_udp_port().await.expect("first link port"),
        ubridge.reserve_udp_port().await.expect("second link port"),
    ];
    let hosts = [
        host_veth_name(&endpoints[0]).expect("first host veth name"),
        host_veth_name(&endpoints[1]).expect("second host veth name"),
    ];
    let guests = [
        guest_veth_name(&endpoints[0]).expect("first guest veth name"),
        guest_veth_name(&endpoints[1]).expect("second guest veth name"),
    ];
    let mut containers = [None::<String>, None::<String>];

    let run: Result<(), RuntimeError> = async {
        for index in 0..2 {
            let (container_id, _) = docker
                .create_container(
                    &project_id,
                    &nodes[index],
                    "alpine:latest",
                    &format!("UDP NIO Alpine {index}"),
                    Some("/bin/sh -c 'sleep 3600'"),
                    None,
                    None,
                )
                .await?;
            containers[index] = Some(container_id.clone());
            docker.lifecycle(&container_id, NodeAction::Start).await?;
            let pid = docker.container_pid(&container_id).await?;
            ubridge
                .create_veth_pair(&hosts[index], &guests[index])
                .await?;
            ubridge
                .set_docker_mac_address(
                    &guests[index],
                    if index == 0 {
                        "02:00:00:00:00:01"
                    } else {
                        "02:00:00:00:00:02"
                    },
                )
                .await?;
            ubridge.set_interface_up(&guests[index]).await?;
            ubridge.set_interface_up(&hosts[index]).await?;
            ubridge
                .move_veth_to_container(&guests[index], pid, "eth0")
                .await?;
            ubridge
                .attach_link_endpoint(
                    &endpoints[index],
                    link_ports[index],
                    link_ports[1 - index],
                    None,
                    None,
                    Some(&hosts[index]),
                    &serde_json::json!({}),
                    false,
                    None,
                )
                .await?;
            let docker_binary = env::var_os("NETWORK_SIMULATOR_DOCKER")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("docker"));
            for args in [
                vec![
                    "ip",
                    "addr",
                    "add",
                    if index == 0 {
                        "10.0.0.1/24"
                    } else {
                        "10.0.0.2/24"
                    },
                    "dev",
                    "eth0",
                ],
                vec!["ip", "link", "set", "eth0", "up"],
            ] {
                let output = Command::new(&docker_binary)
                    .args(["exec", &container_id])
                    .args(args)
                    .output()
                    .await
                    .map_err(|error| {
                        RuntimeError::Unavailable(format!(
                            "Could not configure Alpine interface: {error}"
                        ))
                    })?;
                if !output.status.success() {
                    return Err(RuntimeError::Failed(format!(
                        "Could not configure Alpine interface: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    )));
                }
            }
        }

        let docker_binary = env::var_os("NETWORK_SIMULATOR_DOCKER")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("docker"));
        let output = Command::new(docker_binary)
            .args([
                "exec",
                containers[0].as_deref().unwrap_or_default(),
                "ping",
                "-c",
                "1",
                "-W",
                "3",
                "10.0.0.2",
            ])
            .output()
            .await
            .map_err(|error| {
                RuntimeError::Unavailable(format!("Could not run Alpine-to-Alpine ping: {error}"))
            })?;
        if !output.status.success()
            || !String::from_utf8_lossy(&output.stdout).contains("1 packets received")
        {
            return Err(RuntimeError::Failed(format!(
                "Alpine-to-Alpine ICMP failed: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        Ok(())
    }
    .await;

    for endpoint in &endpoints {
        let _ = ubridge.delete_link_endpoint(endpoint).await;
    }
    for (index, container_id) in containers.iter().enumerate() {
        if let Some(container_id) = container_id.as_deref() {
            let _ = docker
                .remove_node_resources(&nodes[index], Some(container_id))
                .await;
        }
        let _ = ubridge.delete_veth(&hosts[index]).await;
    }
    for host in &hosts {
        let output = Command::new("ip")
            .args(["link", "show", "dev", host])
            .output()
            .await
            .expect("inspect cleaned-up veth");
        assert!(
            !output.status.success(),
            "orphan host veth remained: {host}"
        );
    }
    if let Err(error) = run {
        panic!(
            "real Alpine-to-Alpine UDP NIO test failed: {}",
            error.message()
        );
    }
}
