
    use std::{
        fs,
        io::BufRead,
        os::unix::{fs::PermissionsExt, net::UnixListener},
        path::PathBuf,
        thread,
        time::Duration,
    };

    use serde_json::{Value, json};
    use tempfile::TempDir;

    use super::{
        QemuDisk, QemuLink, QemuRuntime, QemuVm, cached_md5, cpu_limiter_arguments,
        image_format_matches, mac_address, qemu_keyval_escape, qemu_netdev_argument,
        qemu_runtime_arguments, qmp_execute_inner, safe_disk_image_name, valid_qemu_disk_size,
        validate_extra_options,
    };

    #[test]
    fn validates_qemu_disk_size_units() {
        for size in ["1", "512M", "4GiB", "2T"] {
            assert!(valid_qemu_disk_size(size), "{size} should be accepted");
        }
        for size in ["", "0M", "-1G", "+1G", "1G;touch", "1.5G"] {
            assert!(!valid_qemu_disk_size(size), "{size} should be rejected");
        }
    }

    #[test]
    fn caches_md5_checksums_for_runtime_disk_files() {
        let directory = TempDir::new().unwrap();
        let disk = directory.path().join("disk.qcow2");
        fs::write(&disk, b"hello").unwrap();
        assert_eq!(
            cached_md5(&disk, directory.path()).as_deref(),
            Some("5d41402abc4b2a76b9719d911017c592")
        );
        fs::write(directory.path().join("disk.qcow2.md5sum"), "not-a-digest").unwrap();
        assert_eq!(
            cached_md5(&disk, directory.path()).as_deref(),
            Some("5d41402abc4b2a76b9719d911017c592")
        );
    }

    fn qemu_img_test_double(directory: &std::path::Path) -> PathBuf {
        let binary = directory.join("qemu-img-test-double");
        fs::write(
            &binary,
            "#!/bin/sh\ncase \"$1\" in\n  info) case \"$3\" in */config.img) echo '{\"format\":\"raw\"}';; *) echo '{\"format\":\"qcow2\"}';; esac ;;\n  create) for target do :; done; : > \"$target\" ;;\n  *) exit 1 ;;\nesac\n",
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
        binary
    }

    #[tokio::test]
    async fn honors_qemu_backing_file_overrides_and_resets_requested_overlay() {
        let directory = TempDir::new().unwrap();
        let images = directory.path().join("images");
        fs::create_dir_all(&images).unwrap();
        fs::write(images.join("router.qcow2"), b"original").unwrap();
        fs::write(images.join("replacement.qcow2"), b"replacement").unwrap();
        let project_id = "00000000-0000-4000-8000-000000000001";
        let node_id = "00000000-0000-4000-8000-000000000002";
        let state = directory.path().join("state");
        let node_dir = state.join(project_id).join(node_id);
        fs::create_dir_all(&node_dir).unwrap();
        fs::write(node_dir.join(format!("{node_id}.qcow2")), b"stale overlay").unwrap();
        fs::write(
            node_dir.join(format!("{node_id}.qcow2.md5sum")),
            b"stale digest",
        )
        .unwrap();
        let runtime = QemuRuntime::with_paths(
            directory.path().join("qemu-system-x86_64"),
            qemu_img_test_double(directory.path()),
            &images,
            &state,
            directory.path().join("sockets"),
        );
        let options = super::QemuTemplateOptions {
            hda_disk_image_backing_file: Some("replacement.qcow2".to_owned()),
            disk_images_to_reset: vec!["hda_disk_image".to_owned()],
            ..super::QemuTemplateOptions::default()
        };
        let vm = runtime
            .create_vm_with_options(project_id, node_id, "router.qcow2", 512, 1, 1, &options)
            .await
            .unwrap();
        assert_eq!(
            vm.image,
            images.join("replacement.qcow2").canonicalize().unwrap()
        );
        assert_eq!(fs::read(&vm.overlay).unwrap(), b"");
        assert!(!node_dir.join(format!("{node_id}.qcow2.md5sum")).exists());
        let fields = runtime.runtime_fields(&vm);
        assert_eq!(fields["hda_disk_image_backing_file"], "replacement.qcow2");
        assert_eq!(
            fields["hda_disk_image_md5sum"],
            "d41d8cd98f00b204e9800998ecf8427e"
        );
    }

    #[tokio::test]
    async fn creates_a_private_raw_qemu_config_disk_on_hdd() {
        let directory = TempDir::new().unwrap();
        let images = directory.path().join("images");
        fs::create_dir_all(&images).unwrap();
        fs::write(images.join("router.qcow2"), b"original").unwrap();
        fs::write(images.join("config.img"), b"raw config").unwrap();
        let runtime = QemuRuntime::with_paths(
            directory.path().join("qemu-system-x86_64"),
            qemu_img_test_double(directory.path()),
            &images,
            directory.path().join("state"),
            directory.path().join("sockets"),
        );
        let options = super::QemuTemplateOptions {
            create_config_disk: Some(true),
            ..super::QemuTemplateOptions::default()
        };
        let vm = runtime
            .create_vm_with_options(
                "00000000-0000-4000-8000-000000000001",
                "00000000-0000-4000-8000-000000000002",
                "router.qcow2",
                512,
                1,
                1,
                &options,
            )
            .await
            .unwrap();
        let config = vm.disks.iter().find(|disk| disk.slot == "hdd").unwrap();
        assert_eq!(config.format, "raw");
        assert_eq!(config.interface, "ide");
        assert_eq!(fs::read(&config.active_file).unwrap(), b"raw config");
        assert!(
            qemu_runtime_arguments(&vm)
                .unwrap()
                .iter()
                .any(|argument| argument.contains("format=raw"))
        );
    }

    #[test]
    fn qmp_process_fixture() {
        let Ok(socket_path) = std::env::var("NETWORK_SIMULATOR_TEST_QMP_SOCKET") else {
            return;
        };
        let pid_file = std::env::var("NETWORK_SIMULATOR_TEST_QMP_PIDFILE")
            .expect("QMP fixture pid file is provided");
        let state_file = format!("{socket_path}.state");
        for stale in [&socket_path, &state_file] {
            let _ = fs::remove_file(stale);
        }
        fs::write(&state_file, "running").expect("QMP state file is created");
        let listener = UnixListener::bind(&socket_path).expect("QMP fixture socket binds");
        let mut alive = true;
        while alive {
            let (mut connection, _) = listener.accept().expect("QMP fixture accepts client");
            use std::io::{BufRead, BufReader, Write};
            connection
                .write_all(b"{\"QMP\":{\"version\":{}}}\n")
                .expect("QMP fixture greeting writes");
            let mut reader = BufReader::new(connection.try_clone().expect("QMP socket clones"));
            let mut line = String::new();
            reader
                .read_line(&mut line)
                .expect("QMP capabilities arrive");
            connection
                .write_all(b"{\"return\":{},\"id\":\"capabilities\"}\n")
                .expect("QMP capabilities reply writes");
            line.clear();
            reader.read_line(&mut line).expect("QMP request arrives");
            let request: Value = serde_json::from_str(&line).expect("QMP request is JSON");
            let command = request["execute"].as_str().expect("QMP command exists");
            let result = match command {
                "query-status" => json!({
                    "return": { "status": fs::read_to_string(&state_file).expect("QMP state reads") },
                    "id": request["id"]
                }),
                "stop" => {
                    fs::write(&state_file, "paused").expect("paused state is stored");
                    json!({ "return": {}, "id": request["id"] })
                }
                "cont" => {
                    fs::write(&state_file, "running").expect("running state is stored");
                    json!({ "return": {}, "id": request["id"] })
                }
                "quit" => {
                    alive = false;
                    json!({ "return": {}, "id": request["id"] })
                }
                _ => json!({ "return": {}, "id": request["id"] }),
            };
            connection
                .write_all(format!("{}\n", result).as_bytes())
                .expect("QMP response writes");
            drop(reader);
            drop(connection);
        }
        drop(listener);
        for stale in [&socket_path, &state_file, &pid_file] {
            let _ = fs::remove_file(stale);
        }
    }

    #[test]
    fn escapes_qemu_key_value_arguments_and_generates_local_macs() {
        assert_eq!(
            qemu_keyval_escape("/tmp/vm, one\\disk.qcow2"),
            "/tmp/vm\\, one\\\\disk.qcow2"
        );
        assert_eq!(
            mac_address("00000000-0000-0000-0000-000000000001", 0),
            "52:54:00:00:00:01"
        );
        assert_ne!(
            mac_address("00000000-0000-0000-0000-000000000001", 0),
            mac_address("00000000-0000-0000-0000-000000000001", 1)
        );
    }

    #[test]
    fn qemu_runtime_arguments_cover_disks_direct_boot_console_and_extra_options() {
        let vm = QemuVm {
            node_id: "00000000-0000-0000-0000-000000000001".to_owned(),
            image: "/images/router.qcow2".into(),
            platform: "x86_64".to_owned(),
            overlay: "/state/router.qcow2".into(),
            qmp_socket: "/state/qmp.sock".into(),
            serial_socket: "/state/serial.sock".into(),
            pid_file: "/state/router.pid".into(),
            log_file: "/state/router.log".into(),
            memory_mb: 1024,
            cpus: 2,
            maxcpus: 4,
            adapters: 0,
            adapter_type: "virtio-net-pci".to_owned(),
            disk_interface: "ide".to_owned(),
            boot_priority: "dc".to_owned(),
            disks: vec![
                QemuDisk {
                    slot: "hda".to_owned(),
                    image: "/images/router.qcow2".into(),
                    active_file: "/state/router.qcow2".into(),
                    interface: "ide".to_owned(),
                    format: "qcow2".to_owned(),
                },
                QemuDisk {
                    slot: "hdb".to_owned(),
                    image: "/images/data.raw".into(),
                    active_file: "/state/data.raw".into(),
                    interface: "scsi".to_owned(),
                    format: "raw".to_owned(),
                },
            ],
            cdrom_image: None,
            bios_image: None,
            kernel_image: Some("/images/bzImage".into()),
            initrd: Some("/images/initrd.img".into()),
            kernel_command_line: "console=ttyS0 root=/dev/sda".to_owned(),
            linked_clone: true,
            console_type: "telnet".to_owned(),
            console_port: Some(2001),
            aux_type: "ssh".to_owned(),
            aux_port: Some(2002),
            on_close: "save_vm_state".to_owned(),
            process_priority: "normal".to_owned(),
            extra_options: "-cpu Haswell".to_owned(),
            tpm: false,
            uefi: false,
            replicate_network_connection_state: true,
            create_config_disk: false,
            cpu_throttling: 0,
        };
        let args = qemu_runtime_arguments(&vm).unwrap();
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-smp", "cpus=2,maxcpus=4,sockets=1"])
        );
        assert!(args.windows(2).any(|pair| pair == ["-boot", "order=dc"]));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-kernel", "/images/bzImage"])
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-initrd", "/images/initrd.img"])
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-append", "console=ttyS0 root=/dev/sda"])
        );
        assert!(args.windows(2).any(|pair| pair == ["-cpu", "Haswell"]));
        assert!(args.iter().any(|arg| arg.contains("format=qcow2")));
        assert!(args.iter().any(|arg| arg.contains("format=raw")));
        assert!(args.iter().any(|arg| arg.contains("telnet:127.0.0.1:2002")));
        assert!(!args.iter().any(|arg| arg.contains("telnet:127.0.0.1:2001")));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["-loadvm", "GNS3_SAVED_STATE"])
        );
        assert!(cpu_limiter_arguments(123, 75).contains(&"--limit=75".to_owned()));
    }

    #[test]
    fn qemu_extra_options_use_a_safe_allowlist() {
        validate_extra_options("-cpu Haswell -rtc base=utc,clock=host -no-reboot").unwrap();
        for options in [
            "-fw_cfg name=opt/example,file=/etc/passwd",
            "-drive file=/etc/passwd",
            "-readconfig /etc/qemu.conf",
            "-chardev file,id=host,path=/etc/passwd",
            "-chardev socket,id=host,path=/tmp/host.sock",
            "-fsdev local,id=fsdev0,path=/",
            "-virtfs local,path=/,mount_tag=host",
            "-kernel /etc/passwd",
            "-initrd /etc/passwd",
            "-monitor tcp:0.0.0.0:4444,server=on,wait=off",
            "-qmp tcp:0.0.0.0:4444,server=on,wait=off",
            "-serial /dev/ttyS0",
            "-parallel /dev/lp0",
            "-runas root",
            "-daemonize",
            "-cpu Haswell,file=/etc/passwd",
        ] {
            assert!(
                validate_extra_options(options).is_err(),
                "unsafe QEMU option was accepted: {options}"
            );
        }
    }

    #[test]
    fn qemu_links_use_a_local_udp_nio_to_the_ubridge_adapter_port() {
        let link = QemuLink {
            link_id: "00000000-0000-0000-0000-000000000001".to_owned(),
            adapter_number: 2,
            adapter_port: 20_000,
            runtime_port: 20_001,
        };
        assert_eq!(
            qemu_netdev_argument(&link, "ns_00000000000000000000000000000001"),
            "socket,id=ns_00000000000000000000000000000001,udp=127.0.0.1:20000,localaddr=127.0.0.1:20001"
        );
    }

    #[test]
    fn serial_socket_is_persisted_and_derived_for_older_qemu_nodes() {
        let directory = TempDir::new().expect("temporary QEMU directory");
        let runtime = QemuRuntime::with_paths(
            "qemu-system-x86_64",
            "qemu-img",
            directory.path().join("images"),
            directory.path().join("state"),
            directory.path().join("sockets"),
        );
        let project_id = "00000000-0000-4000-8000-000000000001";
        let node_id = "00000000-0000-4000-8000-000000000002";
        let image_dir = directory.path().join("images");
        let image = image_dir.join("router.qcow2");
        let state_dir = directory.path().join("state");
        let vm_dir = state_dir.join(project_id).join(node_id);
        fs::create_dir_all(&image_dir).unwrap();
        fs::create_dir_all(&vm_dir).unwrap();
        fs::create_dir_all(directory.path().join("sockets")).unwrap();
        fs::write(&image, b"image").unwrap();
        fs::write(vm_dir.join(format!("{node_id}.qcow2")), b"overlay").unwrap();
        let node = json!({
            "project_id": project_id,
            "node_id": node_id,
            "hda_disk_image": image,
            "linked_clone": true,
            "runtime_overlay": vm_dir.join(format!("{node_id}.qcow2")),
            "runtime_qmp_socket": directory.path().join("sockets").join(format!("{node_id}.sock")),
            "runtime_pid_file": vm_dir.join(format!("{node_id}.pid")),
            "runtime_log_file": vm_dir.join(format!("{node_id}.log"))
        });
        let vm = runtime.vm_from_node(&node).expect("node runtime is valid");
        assert_eq!(
            vm.serial_socket,
            directory
                .path()
                .join("sockets")
                .join(format!("{node_id}.serial.sock"))
        );
        assert_eq!(
            runtime.runtime_fields(&vm)["runtime_serial_socket"],
            vm.serial_socket.display().to_string()
        );
    }

    #[test]
    fn imported_qemu_runtime_paths_cannot_escape_the_configured_state_directories() {
        let directory = TempDir::new().expect("temporary QEMU directory");
        let project_id = "00000000-0000-4000-8000-000000000001";
        let node_id = "00000000-0000-4000-8000-000000000002";
        let image_dir = directory.path().join("images");
        let image = image_dir.join("router.qcow2");
        let state_dir = directory.path().join("state");
        let vm_dir = state_dir.join(project_id).join(node_id);
        let qmp_dir = directory.path().join("sockets");
        fs::create_dir_all(&image_dir).unwrap();
        fs::create_dir_all(&vm_dir).unwrap();
        fs::create_dir_all(&qmp_dir).unwrap();
        fs::write(&image, b"image").unwrap();
        let overlay = vm_dir.join(format!("{node_id}.qcow2"));
        fs::write(&overlay, b"overlay").unwrap();
        let runtime = QemuRuntime::with_paths(
            "qemu-system-x86_64",
            "qemu-img",
            &image_dir,
            &state_dir,
            &qmp_dir,
        );
        let valid = json!({
            "project_id": project_id,
            "node_id": node_id,
            "hda_disk_image": image,
            "linked_clone": true,
            "runtime_overlay": overlay,
            "runtime_qmp_socket": qmp_dir.join(format!("{node_id}.sock")),
            "runtime_serial_socket": qmp_dir.join(format!("{node_id}.serial.sock")),
            "runtime_pid_file": vm_dir.join(format!("{node_id}.pid")),
            "runtime_log_file": vm_dir.join(format!("{node_id}.log"))
        });
        assert!(runtime.vm_from_node(&valid).is_ok());
        for (field, path) in [
            ("runtime_overlay", "/tmp/victim.qcow2"),
            ("runtime_qmp_socket", "/tmp/victim.sock"),
            ("runtime_serial_socket", "/tmp/victim.serial.sock"),
            ("runtime_pid_file", "/tmp/victim.pid"),
            ("runtime_log_file", "/tmp/victim.log"),
        ] {
            let mut imported = valid.clone();
            imported[field] = json!(path);
            assert!(
                runtime.vm_from_node(&imported).is_err(),
                "imported runtime field {field} escaped confinement"
            );
        }
        let mut imported = valid;
        imported["hda_disk_image"] = json!("/etc/passwd.qcow2");
        assert!(runtime.vm_from_node(&imported).is_err());
    }

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
                &super::QemuTemplateOptions::default(),
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
