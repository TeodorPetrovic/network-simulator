use super::*;

#[tokio::test]
async fn performs_qmp_handshake_and_skips_events_before_matching_reply() {
    let directory = TempDir::new().expect("temporary QMP directory");
    let socket = directory.path().join("qmp.sock");
    let listener = UnixListener::bind(&socket).expect("QMP listener binds");
    let thread = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("QMP client connects");
        use std::io::Write;
        stream
            .write_all(b"{\"QMP\":{\"version\":{}}}\r\n")
            .expect("QMP greeting writes");
        let mut reader = std::io::BufReader::new(stream.try_clone().expect("socket clones"));
        let mut line = String::new();
        reader.read_line(&mut line).expect("capabilities arrive");
        stream
            .write_all(b"{\"return\":{},\"id\":\"capabilities\"}\r\n")
            .expect("capability reply writes");
        line.clear();
        reader.read_line(&mut line).expect("status query arrives");
        stream
            .write_all(b"{\"event\":\"STOP\",\"data\":{}}\r\n{\"return\":{\"status\":\"running\"},\"id\":\"request\"}\r\n")
            .expect("status reply writes");
        thread::sleep(Duration::from_millis(20));
    });
    let response = qmp_execute_inner(&socket, "query-status", None)
        .await
        .expect("QMP status query succeeds");
    assert_eq!(response["status"], "running");
    thread.join().expect("QMP fixture finishes");
}

#[tokio::test]
async fn qemu_templates_are_real_files_inside_the_configured_image_directory() {
    let directory = TempDir::new().expect("temporary QEMU directory");
    let images = directory.path().join("images");
    fs::create_dir_all(&images).expect("image directory exists");
    fs::write(images.join("router.qcow2"), b"test image").expect("image is present");
    fs::write(images.join("notes.txt"), b"ignore").expect("non-image file is present");
    let qemu = directory.path().join("qemu-test-double");
    let qemu_img = directory.path().join("qemu-img-test-double");
    for (path, script) in [
        (
            &qemu,
            "#!/bin/sh\n[ \"$1\" = -version ] && echo 'QEMU emulator version 9.0'\n",
        ),
        (
            &qemu_img,
            "#!/bin/sh\n[ \"$1\" = --version ] && echo 'qemu-img version 9.0'\n",
        ),
    ] {
        fs::write(path, script).expect("write utility double");
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))
            .expect("utility double is executable");
    }
    let runtime = QemuRuntime::with_paths(
        qemu,
        qemu_img,
        &images,
        directory.path().join("state"),
        directory.path().join("sockets"),
    );
    let templates = runtime.templates().await.expect("templates list");
    assert_eq!(templates.len(), 1);
    assert_eq!(templates[0]["template_type"], "qemu");
    assert_eq!(templates[0]["name"], "router.qcow2");
    assert!(templates[0]["template_id"].as_str().is_some());
}

#[tokio::test]
async fn qemu_img_disk_create_and_resize_helpers_execute_with_validated_arguments() {
    let directory = TempDir::new().expect("temporary QEMU directory");
    let qemu_img = directory.path().join("qemu-img-test-double");
    let log = directory.path().join("qemu-img-args.log");
    fs::write(
        &qemu_img,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nif [ \"$1\" = create ]; then touch \"$4\"; fi\n",
            log.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&qemu_img, fs::Permissions::from_mode(0o755)).unwrap();
    let runtime = QemuRuntime::with_paths(
        "qemu-system-x86_64",
        &qemu_img,
        directory.path().join("images"),
        directory.path().join("state"),
        directory.path().join("sockets"),
    );
    let project_id = "00000000-0000-4000-8000-000000000001";
    let node_id = "00000000-0000-4000-8000-000000000002";
    let image = runtime
        .create_disk_image(
            project_id,
            node_id,
            "extra.qcow2",
            "qcow2",
            64,
            &std::collections::BTreeMap::new(),
        )
        .await
        .unwrap();
    runtime
        .resize_disk_image(project_id, node_id, "extra.qcow2", 128)
        .await
        .unwrap();
    fs::write(image.with_file_name("extra.qcow2.md5sum"), "cached").unwrap();
    runtime
        .delete_disk_image(project_id, node_id, "extra.qcow2")
        .unwrap();
    let commands = fs::read_to_string(log).unwrap();
    assert!(commands.contains("create -f qcow2"));
    assert!(commands.contains("resize "));
    assert!(commands.contains("+128M"));
    assert!(!image.exists());
    assert!(!image.with_file_name("extra.qcow2.md5sum").exists());
}

#[tokio::test]
#[ignore = "requires qemu-system-x86_64 and qemu-img; set NETWORK_SIMULATOR_TEST_QEMU and NETWORK_SIMULATOR_TEST_QEMU_IMG"]
async fn qemu_live_vm_launches_reloads_and_stops_through_qmp() {
    let qemu = std::env::var_os("NETWORK_SIMULATOR_TEST_QEMU")
        .expect("NETWORK_SIMULATOR_TEST_QEMU is set for the live test");
    let qemu_img = std::env::var_os("NETWORK_SIMULATOR_TEST_QEMU_IMG")
        .expect("NETWORK_SIMULATOR_TEST_QEMU_IMG is set for the live test");
    let directory = TempDir::new().expect("temporary live QEMU directory");
    let images = directory.path().join("images");
    fs::create_dir_all(&images).unwrap();
    let image_creation = std::process::Command::new(&qemu_img)
        .args(["create", "-f", "qcow2"])
        .arg(images.join("router.qcow2"))
        .arg("16M")
        .status()
        .expect("qemu-img executable runs");
    assert!(
        image_creation.success(),
        "qemu-img creates the live test disk"
    );
    let runtime = QemuRuntime::with_paths(
        PathBuf::from(qemu),
        PathBuf::from(qemu_img),
        &images,
        directory.path().join("state"),
        directory.path().join("sockets"),
    );
    let vm = runtime
        .create_vm_with_options(
            "00000000-0000-4000-8000-000000000001",
            "00000000-0000-4000-8000-000000000002",
            images.join("router.qcow2").to_str().unwrap(),
            64,
            1,
            0,
            &super::super::QemuTemplateOptions::default(),
        )
        .await
        .expect("QEMU VM config is created");
    assert_eq!(
        runtime
            .lifecycle(&vm, crate::runtime::NodeAction::Start, &[])
            .await
            .unwrap(),
        "started"
    );
    let reload_status = runtime
        .lifecycle(&vm, crate::runtime::NodeAction::Reload, &[])
        .await;
    if let Err(error) = reload_status {
        let _ = runtime
            .lifecycle(&vm, crate::runtime::NodeAction::Stop, &[])
            .await;
        panic!("live QEMU reload failed: {}", error.message());
    }
    assert_eq!(reload_status.unwrap(), "started");
    assert_eq!(runtime.status(&vm).await.unwrap(), "started");
    assert_eq!(
        runtime
            .lifecycle(&vm, crate::runtime::NodeAction::Stop, &[])
            .await
            .unwrap(),
        "stopped"
    );
    runtime.remove_vm(&vm).await.unwrap();
}

#[test]
fn qemu_upload_names_and_formats_are_checked_before_becoming_templates() {
    assert_eq!(
        safe_disk_image_name("router.qcow2").unwrap(),
        "router.qcow2"
    );
    assert!(safe_disk_image_name("../router.qcow2").is_err());
    assert!(safe_disk_image_name("router.iso").is_err());
    assert!(safe_disk_image_name(".hidden.qcow2").is_err());
    assert!(image_format_matches(
        std::path::Path::new("router.qcow2"),
        "qcow2"
    ));
    assert!(image_format_matches(
        std::path::Path::new("router.img"),
        "raw"
    ));
    assert!(!image_format_matches(
        std::path::Path::new("router.qcow2"),
        "raw"
    ));
}
