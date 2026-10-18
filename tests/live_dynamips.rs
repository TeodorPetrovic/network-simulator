use std::{
    fs::{self, File},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

fn processes_with_argument(needle: &str) -> Vec<u32> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let pid = entry.file_name().to_string_lossy().parse::<u32>().ok()?;
            let command_line = fs::read(entry.path().join("cmdline")).ok()?;
            let command_line = String::from_utf8_lossy(&command_line);
            (command_line.contains("ubridge") && command_line.contains(needle)).then_some(pid)
        })
        .collect()
}

/// Live appliance test. It is ignored by default because it boots IOS, uses
/// Docker/ubridge/VPCS, and requires operator-supplied IOS files in the global
/// image library. Run with:
/// `cargo test --locked --test live_dynamips -- --ignored --nocapture`.
#[test]
#[ignore = "boots licensed IOS images and requires Dynamips, VPCS, Docker, ubridge, and Node.js"]
fn imported_ios_appliances_boot_and_route_a_live_lab() {
    let data = tempfile::tempdir().expect("create isolated live-test state");
    let state = data.path().join("state");
    let ubridge_socket = state.join("ubridge/ubridge.sock");
    let listener = TcpListener::bind("127.0.0.1:0").expect("reserve a local controller port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let log_path = data.path().join("controller.log");
    let log = File::create(&log_path).expect("create controller log");
    let error_log = log.try_clone().expect("clone controller log handle");
    let mut controller = Command::new(env!("CARGO_BIN_EXE_network-simulator"))
        .current_dir(root)
        .env(
            "NETWORK_SIMULATOR_DB",
            data.path().join("controller.sqlite"),
        )
        .env("NETWORK_SIMULATOR_BIND", format!("127.0.0.1:{port}"))
        .env("NETWORK_SIMULATOR_DYNAMIPS_STATE", state.join("dynamips"))
        .env("NETWORK_SIMULATOR_VPCS_STATE", state.join("vpcs"))
        .env("NETWORK_SIMULATOR_DOCKER_STATE", state.join("docker"))
        .env("NETWORK_SIMULATOR_UBRIDGE_STATE", state.join("ubridge"))
        .env("NETWORK_SIMULATOR_QEMU_STATE", state.join("qemu"))
        .env("NETWORK_SIMULATOR_CAPTURE_HOME", state.join("captures"))
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(error_log))
        .spawn()
        .expect("start isolated current-source controller");

    let deadline = Instant::now() + Duration::from_secs(45);
    let mut ready = false;
    while Instant::now() < deadline {
        if controller
            .try_wait()
            .expect("inspect controller process")
            .is_some()
        {
            break;
        }
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) {
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("set health probe timeout");
            stream
                .write_all(b"GET /healthz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .expect("send health probe");
            let mut response = String::new();
            let _ = stream.read_to_string(&mut response);
            if response.contains("200 OK") && response.contains("\"status\":\"ok\"") {
                ready = true;
                break;
            }
        }
        thread::sleep(Duration::from_millis(200));
    }

    let result = if ready {
        Command::new("node")
            .arg("scripts/live_dynamips_lab.mjs")
            .current_dir(root)
            .env(
                "NETWORK_SIMULATOR_BASE_URL",
                format!("http://127.0.0.1:{port}"),
            )
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    } else {
        false
    };

    let ubridge_was_running =
        !processes_with_argument(&ubridge_socket.display().to_string()).is_empty();
    #[cfg(unix)]
    {
        let pid = controller.id();
        // Exercise the same graceful shutdown path used by Ctrl-C so the
        // controller can stop the shared uBridge daemon it owns.
        let _ = unsafe { libc::kill(pid as i32, libc::SIGINT) };
    }
    #[cfg(not(unix))]
    let _ = controller.kill();
    let shutdown_deadline = Instant::now() + Duration::from_secs(5);
    let mut controller_exited = false;
    while Instant::now() < shutdown_deadline {
        if controller
            .try_wait()
            .expect("inspect controller shutdown")
            .is_some()
        {
            controller_exited = true;
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    if !controller_exited {
        let _ = controller.kill();
        let _ = controller.wait();
    }
    let orphaned_ubridge = processes_with_argument(&ubridge_socket.display().to_string());
    let mut log_contents = String::new();
    let _ = File::open(&log_path).and_then(|mut file| file.read_to_string(&mut log_contents));
    assert!(
        ready,
        "isolated controller did not start; log:\n{log_contents}"
    );
    assert!(
        result,
        "live IOS lab script failed; controller log:\n{log_contents}"
    );
    assert!(
        ubridge_was_running,
        "live link lab did not start its isolated uBridge daemon"
    );
    assert!(
        controller_exited,
        "isolated controller did not complete graceful shutdown"
    );
    assert!(
        orphaned_ubridge.is_empty(),
        "controller shutdown left orphaned uBridge processes: {orphaned_ubridge:?}"
    );
}
