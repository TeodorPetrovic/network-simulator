
    use std::{env, fs, path::PathBuf, time::Duration};

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        time::timeout,
    };
    use uuid::Uuid;

    use std::{
        net::{Ipv4Addr, SocketAddr, SocketAddrV4},
        process::Stdio,
    };

    use tokio::process::Command;

    use super::{
        ManagedVpcs, VpcsRuntime, connect_vpcs_console, kill_vpcs_startup_tree, mac_offset,
        write_startup_script,
    };

    #[test]
    fn derives_mac_offset_from_the_node_identifier() {
        let id = Uuid::parse_str("00000000-0000-0000-0000-00000000002a").unwrap();
        assert_eq!(mac_offset(&id), 42);
    }

    #[test]
    fn startup_script_write_clears_old_content_and_expands_hostname() {
        let directory = tempfile::tempdir().unwrap();
        let path = write_startup_script(directory.path(), Some("name %h"), "PC 1").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "name PC 1");
        write_startup_script(directory.path(), None, "PC 1").unwrap();
        assert_eq!(fs::read_to_string(path).unwrap(), "");
    }

    #[test]
    fn separate_instances_keep_independent_base_startup_scripts() {
        let directory = tempfile::tempdir().unwrap();
        let first_dir = directory.path().join("project/first");
        let second_dir = directory.path().join("project/second");
        fs::create_dir_all(&first_dir).unwrap();
        fs::create_dir_all(&second_dir).unwrap();
        let first =
            write_startup_script(&first_dir, Some("ip 192.0.2.1/24 name %h"), "PC1").unwrap();
        let second =
            write_startup_script(&second_dir, Some("ip 192.0.2.2/24 name %h"), "PC2").unwrap();
        assert_eq!(
            fs::read_to_string(first).unwrap(),
            "ip 192.0.2.1/24 name PC1"
        );
        assert_eq!(
            fs::read_to_string(second).unwrap(),
            "ip 192.0.2.2/24 name PC2"
        );
    }

    #[tokio::test]
    #[ignore = "requires upstream VPCS and uBridge; set NETWORK_SIMULATOR_TEST_VPCS and NETWORK_SIMULATOR_TEST_UBRIDGE"]
    async fn upstream_vpcs_nodes_exchange_ethernet_frames_over_gns3_udp_nios() {
        let binary = env::var_os("NETWORK_SIMULATOR_TEST_VPCS")
            .expect("NETWORK_SIMULATOR_TEST_VPCS is set for this integration test");
        let directory = tempfile::tempdir().expect("temporary VPCS state directory");
        let runtime = VpcsRuntime::with_paths(PathBuf::from(binary), directory.path());
        let ubridge_binary = env::var_os("NETWORK_SIMULATOR_TEST_UBRIDGE")
            .expect("NETWORK_SIMULATOR_TEST_UBRIDGE is set for this integration test");
        let ubridge = crate::ubridge::UbridgeRuntime::with_paths(
            PathBuf::from(ubridge_binary),
            directory.path().join("ubridge.sock"),
        );
        runtime
            .ensure_available()
            .await
            .expect("upstream VPCS is available");

        let project_id = "00000000-0000-0000-0000-000000000001";
        let node_a = "00000000-0000-0000-0000-00000000000a";
        let node_b = "00000000-0000-0000-0000-00000000000b";
        let endpoint_a = "a0000000-0000-0000-0000-000000000001";
        let endpoint_b = "b0000000-0000-0000-0000-000000000002";
        let link_a = ubridge
            .reserve_udp_port()
            .await
            .expect("first link NIO port");
        let link_b = ubridge
            .reserve_udp_port()
            .await
            .expect("second link NIO port");
        let adapter_a = ubridge
            .reserve_udp_port()
            .await
            .expect("first adapter NIO port");
        let adapter_b = ubridge
            .reserve_udp_port()
            .await
            .expect("second adapter NIO port");
        let runtime_a = ubridge
            .reserve_udp_port()
            .await
            .expect("first VPCS NIO port");
        let runtime_b = ubridge
            .reserve_udp_port()
            .await
            .expect("second VPCS NIO port");
        runtime.set_nio_ports(node_a, adapter_a, runtime_a).await;
        runtime.set_nio_ports(node_b, adapter_b, runtime_b).await;
        runtime
            .lifecycle(
                project_id,
                node_a,
                "PC A",
                Some("ip 10.0.0.1/24"),
                crate::runtime::NodeAction::Start,
            )
            .await
            .expect("first VPCS starts");
        runtime
            .lifecycle(
                project_id,
                node_b,
                "PC B",
                Some("ip 10.0.0.2/24"),
                crate::runtime::NodeAction::Start,
            )
            .await
            .expect("second VPCS starts");
        ubridge
            .attach_link_endpoint(
                endpoint_a,
                link_a,
                link_b,
                Some(adapter_a),
                Some(runtime_a),
                None,
                &serde_json::json!({}),
                false,
                None,
            )
            .await
            .expect("first VPCS uBridge NIO attaches");
        ubridge
            .attach_link_endpoint(
                endpoint_b,
                link_b,
                link_a,
                Some(adapter_b),
                Some(runtime_b),
                None,
                &serde_json::json!({}),
                false,
                None,
            )
            .await
            .expect("second VPCS uBridge NIO attaches");

        let address = runtime
            .console_address(node_a)
            .await
            .expect("VPCS console address");
        let mut console = connect_vpcs_console(address)
            .await
            .expect("VPCS Telnet console connects");
        let mut output = Vec::new();
        timeout(Duration::from_secs(5), async {
            loop {
                let mut chunk = [0_u8; 4096];
                let length = console
                    .read(&mut chunk)
                    .await
                    .expect("VPCS console is readable");
                output.extend_from_slice(&chunk[..length]);
                if String::from_utf8_lossy(&output).contains("VPCS>") {
                    break;
                }
            }
        })
        .await
        .expect("VPCS startup script reaches the interactive prompt");
        output.clear();
        console
            .write_all(b"ping 10.0.0.2 -c 1\r")
            .await
            .expect("ping command reaches VPCS");

        let ping_finished = timeout(Duration::from_secs(12), async {
            loop {
                let mut chunk = [0_u8; 4096];
                let length = console
                    .read(&mut chunk)
                    .await
                    .expect("VPCS console remains open");
                if length == 0 {
                    break;
                }
                output.extend_from_slice(&chunk[..length]);
                if String::from_utf8_lossy(&output).contains("bytes from 10.0.0.2") {
                    break;
                }
            }
        })
        .await;
        let output = String::from_utf8_lossy(&output);
        assert!(
            ping_finished.is_ok(),
            "VPCS did not return ping output: {output}"
        );
        assert!(
            output.contains("bytes from 10.0.0.2"),
            "unexpected VPCS console output: {output}"
        );

        drop(console);
        ubridge
            .delete_link_endpoint(endpoint_a)
            .await
            .expect("first endpoint detaches");
        ubridge
            .delete_link_endpoint(endpoint_b)
            .await
            .expect("second endpoint detaches");
        runtime.stop(node_a).await.expect("first VPCS stops");
        runtime.stop(node_b).await.expect("second VPCS stops");
        assert!(
            connect_vpcs_console(address).await.is_err(),
            "stopped VPCS console should no longer accept connections"
        );
    }

    #[tokio::test]
    async fn dropping_a_managed_process_kills_its_forked_children() {
        use tokio::io::AsyncReadExt as _;

        // Mimic VPCS: a tracked daemon that forks a long-lived child.
        let mut child = Command::new("sh")
            .args(["-c", "setsid sleep 300 & echo $!; wait"])
            .stdout(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true)
            .spawn()
            .expect("spawn fake VPCS");
        let mut stdout = child.stdout.take().expect("stdout pipe");
        let mut buffer = [0_u8; 32];
        let read = stdout.read(&mut buffer).await.expect("fork pid");
        let forked: libc::pid_t = String::from_utf8_lossy(&buffer[..read])
            .trim()
            .parse()
            .expect("numeric pid");
        // SAFETY: signal 0 only probes for existence.
        assert_eq!(unsafe { libc::kill(forked, 0) }, 0, "fork is running");

        drop(ManagedVpcs {
            child,
            console: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 1)),
        });
        for _ in 0..50 {
            // SAFETY: as above.
            if unsafe { libc::kill(forked, 0) } != 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("forked child survived the drop of its managed process");
    }

    #[tokio::test]
    async fn failed_start_cleanup_kills_the_vpcs_process_tree() {
        use tokio::io::AsyncReadExt as _;

        let mut child = Command::new("sh")
            .args(["-c", "setsid sleep 300 & echo $!; wait"])
            .stdout(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true)
            .spawn()
            .expect("spawn fake VPCS startup");
        let mut stdout = child.stdout.take().expect("stdout pipe");
        let mut buffer = [0_u8; 32];
        let read = stdout.read(&mut buffer).await.expect("fork pid");
        let forked: libc::pid_t = String::from_utf8_lossy(&buffer[..read])
            .trim()
            .parse()
            .expect("numeric fork pid");
        // SAFETY: signal 0 only probes that the test child is running.
        assert_eq!(unsafe { libc::kill(forked, 0) }, 0);

        kill_vpcs_startup_tree(&mut child).await;
        for _ in 0..50 {
            // SAFETY: signal 0 only probes for existence.
            if unsafe { libc::kill(forked, 0) } != 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("forked process survived failed-start cleanup");
    }
