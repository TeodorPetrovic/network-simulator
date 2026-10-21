use super::*;

#[test]
fn auto_delete_disks_removes_only_hypervisor_artifacts() {
    let directory = tempfile::tempdir().unwrap();
    let disk = directory.path().join("c7200_i42_disk0");
    let nvram = directory.path().join("c7200_i42_nvram");
    let startup = directory.path().join("configs/i42_startup-config.cfg");
    fs::create_dir_all(startup.parent().unwrap()).unwrap();
    fs::write(&disk, b"disk").unwrap();
    fs::write(&nvram, b"nvram").unwrap();
    fs::write(&startup, b"hostname R42\n").unwrap();

    remove_auto_delete_disk_files(directory.path(), "c7200", 42).unwrap();

    assert!(!disk.exists());
    assert!(!nvram.exists());
    assert!(startup.exists());
}

#[test]
fn transient_cleanup_keeps_ownership_manifest_and_router_state() {
    let directory = tempfile::tempdir().unwrap();
    let kept = [
        RUNTIME_MANIFEST_FILE,
        "c3600_i3_nvram",
        "c3600_i3_disk0",
        "c1700_i1_rommon_vars.bak",
        "notes_log.txt",
    ];
    let removed = [
        "dynamips_stdout.log",
        "dynamips_engine.log",
        "ilt_7200",
        "c3600_i3_lock",
        "c3600_i3_log.txt",
        "c3600_i3_rommon_vars",
        "c7200_i0_log.txt",
    ];
    for name in kept.iter().chain(&removed) {
        fs::write(directory.path().join(name), b"x").unwrap();
    }

    remove_transient_runtime_files(directory.path()).unwrap();

    for name in kept {
        assert!(directory.path().join(name).exists(), "{name} was removed");
    }
    for name in removed {
        assert!(!directory.path().join(name).exists(), "{name} was kept");
    }
}

#[test]
fn removes_only_empty_dynamips_runtime_directories() {
    let directory = tempfile::tempdir().unwrap();
    let project = directory.path().join("project");
    let working = project.join("node");
    fs::create_dir_all(working.join("configs")).unwrap();

    remove_empty_runtime_directories(&working).unwrap();
    assert!(!project.exists());

    fs::create_dir_all(working.join("configs")).unwrap();
    let nvram = working.join("c7200_i1_nvram");
    fs::write(&nvram, b"preserve").unwrap();
    remove_empty_runtime_directories(&working).unwrap();
    assert!(working.exists());
    assert!(nvram.exists());
    assert!(!working.join("configs").exists());
}

#[test]
fn removes_deleted_node_configs_without_touching_virtual_disks() {
    let directory = tempfile::tempdir().unwrap();
    let configs = directory.path().join("configs");
    let startup = configs.join("i1_startup-config.cfg");
    let disk = directory.path().join("c7200_i1_disk0");
    fs::create_dir_all(&configs).unwrap();
    fs::write(&startup, b"hostname R1\n").unwrap();
    fs::write(&disk, b"preserve virtual disk").unwrap();

    remove_generated_runtime_configs(directory.path()).unwrap();

    assert!(!configs.exists());
    assert!(disk.exists());
}

#[tokio::test]
#[ignore = "requires the real Dynamips executable; set NETWORK_SIMULATOR_TEST_DYNAMIPS and run with --ignored"]
async fn upstream_dynamips_creates_and_reports_a_real_c7200_hypervisor_instance() {
    let binary = env::var_os("NETWORK_SIMULATOR_TEST_DYNAMIPS")
        .expect("NETWORK_SIMULATOR_TEST_DYNAMIPS points to upstream Dynamips");
    let directory = tempfile::tempdir().unwrap();
    let runtime = DynamipsRuntime::with_paths(
        binary,
        directory.path().join("ios"),
        directory.path().join("state"),
    );
    fs::create_dir_all(directory.path().join("ios")).unwrap();
    let working_dir = directory.path().join("state/c7200-probe");
    fs::create_dir_all(&working_dir).unwrap();
    let vm = super::DynamipsVm {
        node_id: "00000000-0000-0000-0000-00000000000a".to_owned(),
        name: "R1".to_owned(),
        platform: "c7200".to_owned(),
        chassis: None,
        slot0: "C7200-IO-FE".to_owned(),
        image: directory.path().join("IOS image supplied by the operator"),
        ram_mb: 256,
        nvram: 128,
        idle_pc: None,
        startup_config: String::new(),
        working_dir: working_dir.clone(),
        hypervisor_port: super::available_tcp_port().await.unwrap(),
        console_port: super::available_tcp_port().await.unwrap(),
        aux_port: None,
        log_file: working_dir.join("dynamips.log"),
        options: super::DynamipsOptions::default(),
        ghost_file: None,
    };
    let router = runtime
        .spawn_hypervisor(&vm)
        .await
        .expect("real Dynamips hypervisor starts");
    runtime
        .routers
        .lock()
        .await
        .insert(vm.node_id.clone(), router);
    runtime
        .send(vm.hypervisor_port, "hypervisor version")
        .await
        .unwrap();
    runtime
        .send(
            vm.hypervisor_port,
            &format!(
                "hypervisor working_dir {}",
                super::quote(&working_dir.display().to_string())
            ),
        )
        .await
        .unwrap();
    runtime
        .send(
            vm.hypervisor_port,
            &format!("vm create {} 1 c7200", super::quote(&vm.name)),
        )
        .await
        .unwrap();
    runtime
        .send(
            vm.hypervisor_port,
            &format!(
                "vm slot_add_binding {} 0 0 C7200-IO-FE",
                super::quote(&vm.name)
            ),
        )
        .await
        .unwrap();
    assert_eq!(runtime.status(&vm).await.unwrap(), "stopped");
    let switch_socket = tokio::net::UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let switch_port = switch_socket.local_addr().unwrap().port();
    let router_udp_port = runtime
        .switch_endpoint(&vm, 0, 0, switch_port)
        .await
        .unwrap();
    assert_ne!(router_udp_port, 0);
    runtime.remove_switch_endpoint(&vm).await.unwrap();
    runtime
        .send(
            vm.hypervisor_port,
            &format!("vm clean_delete {}", super::quote(&vm.name)),
        )
        .await
        .unwrap();
    runtime.stop_process(&vm).await.unwrap();
}

#[cfg(target_os = "linux")]
fn process_running(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            stat.rsplit_once(')')?
                .1
                .split_whitespace()
                .next()?
                .chars()
                .next()
        })
        .is_some_and(|state| !matches!(state, 'Z' | 'X'))
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn dropping_managed_hypervisor_kills_detached_descendants() {
    use tokio::{io::AsyncBufReadExt, process::Command};

    let mut child = Command::new("sh")
        .args(["-c", "setsid sleep 300 & echo $!; wait"])
        .stdout(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let root_pid = child.id().unwrap();
    let mut descendant_line = String::new();
    tokio::io::BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut descendant_line)
        .await
        .unwrap();
    let descendant_pid = descendant_line.trim().parse::<u32>().unwrap();
    let managed = ManagedRouter {
        child,
        process_group_id: root_pid,
        group_killed: false,
        hypervisor_port: 7200,
        log_file: PathBuf::from("/dev/null"),
    };

    drop(managed);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while process_running(root_pid) || process_running(descendant_pid) {
        assert!(
            tokio::time::Instant::now() < deadline,
            "Dynamips child process survived Drop"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let mut status = 0;
    // SAFETY: the root child was spawned by this test and has already exited.
    unsafe {
        libc::waitpid(root_pid as libc::pid_t, &mut status, libc::WNOHANG);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn startup_reaper_only_kills_hypervisors_inside_its_state_directory() {
    use std::os::unix::process::CommandExt;

    let directory = tempfile::tempdir().unwrap();
    let binary_dir = directory.path().join("bin");
    let state_dir = directory.path().join("state");
    let node_dir = state_dir.join("project/node");
    fs::create_dir_all(&binary_dir).unwrap();
    fs::create_dir_all(&node_dir).unwrap();
    let binary = binary_dir.join("dynamips");
    fs::copy("/bin/dash", &binary).unwrap();
    let mut inside = std::process::Command::new(&binary)
        .args(["-c", "sleep 300 & wait"])
        .current_dir(&node_dir)
        .process_group(0)
        .spawn()
        .unwrap();
    let mut outside = std::process::Command::new(&binary)
        .args(["-c", "sleep 300 & wait"])
        .current_dir(directory.path())
        .process_group(0)
        .spawn()
        .unwrap();

    assert_eq!(reap_orphaned_dynamips_processes(&binary, &state_dir), 1);
    let inside_status = inside.wait().unwrap();
    assert!(!inside_status.success());
    assert!(process_running(outside.id()));
    kill_dynamips_process_tree(outside.id());
    outside.wait().unwrap();
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn kill_all_stops_every_owned_hypervisor_group() {
    use tokio::process::Command;

    let directory = tempfile::tempdir().unwrap();
    let child = Command::new("sleep")
        .arg("300")
        .process_group(0)
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();
    let runtime = DynamipsRuntime::with_paths(
        "dynamips",
        directory.path().join("images"),
        directory.path().join("state"),
    );
    runtime.routers.lock().await.insert(
        "owned".to_owned(),
        ManagedRouter {
            child,
            process_group_id: pid,
            group_killed: false,
            hypervisor_port: 7200,
            log_file: directory.path().join("dynamips.log"),
        },
    );

    runtime.kill_all().await;
    assert!(runtime.routers.lock().await.is_empty());
    assert!(!process_running(pid));
}

#[tokio::test]
#[ignore = "requires the installed Cisco 3620 IOS image and Dynamips; set NETWORK_SIMULATOR_TEST_DYNAMIPS and NETWORK_SIMULATOR_TEST_C3600_IMAGE"]
async fn upstream_dynamips_boots_a_cisco_3620_and_binds_its_fast_ethernet_port() {
    use crate::runtime::NodeAction;

    let binary = env::var_os("NETWORK_SIMULATOR_TEST_DYNAMIPS")
        .expect("NETWORK_SIMULATOR_TEST_DYNAMIPS points to upstream Dynamips");
    let image = env::var_os("NETWORK_SIMULATOR_TEST_C3600_IMAGE")
        .expect("NETWORK_SIMULATOR_TEST_C3600_IMAGE points to a Cisco 3620 IOS image");
    let image = PathBuf::from(image);
    let image_directory = image.parent().expect("IOS image has a parent directory");
    let directory = tempfile::tempdir().unwrap();
    let runtime =
        DynamipsRuntime::with_paths(binary, image_directory, directory.path().join("state"));
    let project_id = "00000000-0000-0000-0000-00000000000b";
    let node_id = "00000000-0000-0000-0000-00000000000c";
    let vm = runtime
        .create_router(
            project_id,
            node_id,
            "R3620",
            image.to_str().unwrap(),
            "c3600",
            Some("3620"),
            Some("NM-1FE-TX"),
            192,
            256,
            Some("0x603a8bac"),
            "hostname R3620\n",
        )
        .await
        .expect("Dynamips creates the c3600/3620 router and Fast Ethernet module");

    let exercise = async {
        runtime.lifecycle(&vm, NodeAction::Start).await?;
        let switch_socket = tokio::net::UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|error| super::RuntimeError::Unavailable(error.to_string()))?;
        let switch_port = switch_socket.local_addr().unwrap().port();
        runtime.switch_endpoint(&vm, 0, 0, switch_port).await?;
        runtime.remove_switch_endpoint(&vm).await?;
        Ok::<(), super::RuntimeError>(())
    }
    .await;
    let cleanup = runtime.remove_router(&vm).await;
    cleanup.expect("the Dynamips hypervisor and temporary node files are removed");
    exercise.expect("the Cisco 3620 IOS image boots under Dynamips and exposes Ethernet");
}
