use super::{
    DynamipsOptions, DynamipsRuntime, DynamipsVm, ManagedRouter, RUNTIME_MANIFEST_FILE,
    available_tcp_port, default_iomem, default_slot0, dynamips_version_error,
    ghost_router_initialization_commands, idlepc_candidate, idlepc_cpu_rate,
    integrated_slot_module, integrated_slot0, kill_dynamips_process_tree, process_cpu_rate,
    process_thread_cpu_ticks, quote, reap_orphaned_dynamips_processes,
    remove_auto_delete_disk_files, remove_empty_runtime_directories,
    remove_generated_runtime_configs, remove_transient_runtime_files,
    router_initialization_commands, router_start_idlepc_command, router_start_settings_commands,
    runtime_manifest_contents, safe_file_name, supported_dynamips_version,
    validate_dynamips_options, validate_ios_image, validate_platform_settings,
    validate_router_name,
};
use super::config::resolve_working_dir;
use crate::runtime::RuntimeError;
use serde_json::json;
use std::{
    env, fs,
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
    process::Stdio,
};
use uuid::Uuid;

#[test]
fn resolves_and_creates_absolute_working_directory() {
    let relative = PathBuf::from(format!(".dynamips-path-test-{}", Uuid::new_v4()));
    let resolved = resolve_working_dir(&relative).expect("working directory is created");
    assert!(resolved.is_absolute());
    assert!(resolved.is_dir());
    assert!(resolved.ends_with(&relative));
    fs::remove_dir_all(resolved).unwrap();
}

#[test]
fn validates_dynamips_compatible_ios_elf_header() {
    let directory = tempfile::tempdir().unwrap();
    let image = directory.path().join("router.image");
    fs::write(&image, [0x7f, b'E', b'L', b'F', 1, 2, 1, 0]).unwrap();
    assert!(validate_ios_image(&image).is_ok());
    fs::write(&image, b"not an IOS image").unwrap();
    assert!(validate_ios_image(&image).is_err());
}

#[test]
fn parses_idlepc_candidates_and_scores_cpu_delta() {
    assert_eq!(idlepc_candidate("0x1234abcd 90%"), Some("0x1234abcd"));
    assert_eq!(idlepc_candidate("not-a-pointer 90%"), None);
    assert_eq!(idlepc_candidate("0x1234 90%"), None);
    assert!(idlepc_cpu_rate(100, 102, std::time::Duration::from_secs(3)) < 70.0);
    assert_eq!(
        idlepc_cpu_rate(0, 1000, std::time::Duration::from_secs(1)),
        100.0
    );
}

#[test]
fn measures_managed_process_cpu_from_proc_ticks() {
    assert_eq!(
        process_cpu_rate(100, 110, std::time::Duration::from_secs(2), 100),
        5.0
    );
    assert_eq!(
        process_cpu_rate(110, 100, std::time::Duration::from_secs(2), 100),
        0.0
    );
    assert!(process_cpu_rate(0, 10, std::time::Duration::from_secs(1), 0).is_infinite());
    assert_eq!(
        process_thread_cpu_ticks("42 (dynamips (worker)) S 1 2 3 4 5 6 7 8 9 10 11 12 13"),
        Some(23)
    );
}

#[test]
fn ghost_ios_requires_mmap_support() {
    let options = DynamipsOptions {
        ghost_ios: true,
        mmap: false,
        ..DynamipsOptions::default()
    };
    assert!(validate_dynamips_options("c7200", None, None, &options).is_err());
    let supported = DynamipsOptions {
        ghost_ios: true,
        mmap: true,
        ..DynamipsOptions::default()
    };
    assert!(validate_dynamips_options("c7200", None, Some("C7200-IO-FE"), &supported).is_ok());
}

#[test]
fn dynamips_quoted_strings_and_names_cannot_emit_protocol_newlines() {
    let injected = "R1\nvm clean_delete \"R2\"\r";
    let quoted = quote(injected);
    assert!(!quoted.chars().any(char::is_control));
    assert!(quoted.contains("\\x0a"));
    assert!(quoted.contains("\\x0d"));
    assert!(validate_router_name(injected).is_err());
    assert!(validate_router_name("Router-1").is_ok());
}

#[test]
fn imported_dynamips_runtime_fields_must_match_controller_state() {
    let directory = tempfile::tempdir().unwrap();
    let image_dir = directory.path().join("ios");
    let state_dir = directory.path().join("state");
    let project_id = Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap();
    let node_id = Uuid::parse_str("00000000-0000-4000-8000-000000000002").unwrap();
    let working_dir = state_dir
        .join(project_id.to_string())
        .join(node_id.to_string());
    fs::create_dir_all(&image_dir).unwrap();
    fs::create_dir_all(&working_dir).unwrap();
    let image = image_dir.join("router.image");
    fs::write(&image, [0x7f, b'E', b'L', b'F', 1, 2, 1, 0]).unwrap();
    let runtime = DynamipsRuntime::with_paths("dynamips", &image_dir, &state_dir);
    let manifest = runtime_manifest_contents(&project_id, &node_id, 7200, 7201, None);
    fs::write(working_dir.join(RUNTIME_MANIFEST_FILE), manifest).unwrap();
    let node = json!({
        "project_id": project_id,
        "node_id": node_id,
        "name": "R1",
        "platform": "c7200",
        "slot0": "C7200-IO-FE",
        "image": image,
        "ram": 256,
        "nvram": 128,
        "console": 7201,
        "aux": null,
        "runtime_hypervisor_port": 7200,
        "runtime_working_dir": working_dir,
        "runtime_log_file": working_dir.join("dynamips_stdout.log"),
        "runtime_ghost_file": null
    });
    assert!(runtime.vm_from_node(&node).is_ok());
    for (field, value) in [
        ("runtime_working_dir", json!("/tmp/controller-victim")),
        ("runtime_log_file", json!("/tmp/victim.log")),
        ("runtime_hypervisor_port", json!(22)),
        ("name", json!("R1\nvm clean_delete R2")),
        ("image", json!("/etc/passwd.image")),
    ] {
        let mut imported = node.clone();
        imported[field] = value;
        assert!(
            runtime.vm_from_node(&imported).is_err(),
            "imported Dynamips field {field} was accepted"
        );
    }
}

#[test]
fn dynamips_runtime_accepts_canonical_paths_from_relative_state_dir() {
    let relative_state = PathBuf::from(format!(".dynamips-relative-state-{}", Uuid::new_v4()));
    let image_dir = std::env::temp_dir().join(format!("dynamips-images-{}", Uuid::new_v4()));
    fs::create_dir_all(&image_dir).unwrap();
    let image = image_dir.join("router.image");
    fs::write(&image, [0x7f, b'E', b'L', b'F', 1, 2, 1, 0]).unwrap();
    let project_id = Uuid::new_v4();
    let node_id = Uuid::new_v4();
    let runtime = DynamipsRuntime::with_paths("dynamips", &image_dir, &relative_state);
    let working_dir = resolve_working_dir(&relative_state.join(project_id.to_string()).join(node_id.to_string())).unwrap();
    let manifest = runtime_manifest_contents(&project_id, &node_id, 7200, 7201, None);
    fs::write(working_dir.join(RUNTIME_MANIFEST_FILE), manifest).unwrap();
    let node = json!({
        "project_id": project_id, "node_id": node_id, "name": "R1", "platform": "c7200",
        "slot0": "C7200-IO-FE", "image": image, "ram": 256, "nvram": 128,
        "console": 7201, "aux": null, "runtime_hypervisor_port": 7200,
        "runtime_working_dir": working_dir, "runtime_log_file": working_dir.join("dynamips_stdout.log"),
        "runtime_ghost_file": null
    });
    assert!(runtime.vm_from_node(&node).is_ok());
    let mut outside = node.clone();
    outside["runtime_working_dir"] = json!(std::env::temp_dir().join("outside-runtime"));
    assert!(runtime.vm_from_node(&outside).is_err());
    fs::remove_dir_all(&relative_state).unwrap();
    fs::remove_dir_all(&image_dir).unwrap();
}

#[test]
fn upload_names_cannot_escape_or_overwrite_the_image_directory() {
    assert_eq!(safe_file_name("router.image").unwrap(), "router.image");
    assert!(safe_file_name("../router.image").is_err());
    assert!(safe_file_name("router image.image").is_err());
}

#[test]
fn requires_the_minimum_hypervisor_version_supported_by_gns3() {
    assert!(!supported_dynamips_version("0.2.10"));
    assert!(supported_dynamips_version("0.2.11"));
    assert!(!supported_dynamips_version("0.2.14-amd64/Linux stable"));
    assert!(supported_dynamips_version("0.3.0"));
    assert!(!supported_dynamips_version("unknown"));
    assert!(dynamips_version_error("0.2.10").unwrap().contains("0.2.11"));
    assert!(
        dynamips_version_error("0.2.14-amd64/Linux stable")
            .unwrap()
            .contains("crashes while starting IOS")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn reports_and_refuses_the_known_bad_0214_binary() {
    use crate::runtime::NodeAction;
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::tempdir().unwrap();
    let binary = directory.path().join("dynamips");
    fs::write(
            &binary,
            "#!/bin/sh\nprintf '%s\\n' 'Cisco Router Simulation Platform (version 0.2.14-amd64/Linux stable)'\n",
        )
        .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
    let runtime = DynamipsRuntime::with_paths(
        &binary,
        directory.path().join("images"),
        directory.path().join("state"),
    );

    let info = runtime.info().await;
    assert!(!info.available);
    assert_eq!(info.version.as_deref(), Some("0.2.14-amd64/Linux stable"));
    assert!(
        info.error
            .as_deref()
            .unwrap()
            .contains("crashes while starting IOS")
    );
    let vm = DynamipsVm {
        node_id: "00000000-0000-4000-8000-000000000001".to_owned(),
        name: "R1".to_owned(),
        platform: "c3725".to_owned(),
        chassis: None,
        slot0: "GT96100-FE".to_owned(),
        image: directory.path().join("router.image"),
        ram_mb: 128,
        nvram: 256,
        idle_pc: None,
        startup_config: String::new(),
        working_dir: directory.path().join("state/router"),
        hypervisor_port: 7200,
        console_port: 7201,
        aux_port: None,
        log_file: directory.path().join("state/router/dynamips_stdout.log"),
        options: DynamipsOptions::default(),
        ghost_file: None,
    };
    let error = runtime.lifecycle(&vm, NodeAction::Start).await.unwrap_err();
    assert!(error.message().contains("crashes while starting IOS"));
}

#[test]
fn uses_gns3_platform_defaults_for_iomem_and_integrated_slots() {
    assert_eq!(default_iomem("c1700"), Some(15));
    assert_eq!(default_iomem("c2600"), Some(15));
    assert_eq!(default_iomem("c3600"), Some(5));
    assert_eq!(default_iomem("c7200"), None);
    assert_eq!(
        integrated_slot_module("c1700", Some("1760"), 1),
        Some("C1700-MB-WIC1")
    );

    let mut options = DynamipsOptions::default();
    options.slots[1] = Some("C1700-MB-WIC1".to_owned());
    let vm = DynamipsVm {
        node_id: "00000000-0000-0000-0000-00000000000a".to_owned(),
        name: "R1".to_owned(),
        platform: "c1700".to_owned(),
        chassis: Some("1760".to_owned()),
        slot0: "C1700-MB-1FE".to_owned(),
        image: PathBuf::from("/images/c1700.image"),
        ram_mb: 160,
        nvram: 128,
        idle_pc: None,
        startup_config: String::new(),
        working_dir: PathBuf::from("/state/router"),
        hypervisor_port: 7200,
        console_port: 7201,
        aux_port: None,
        log_file: PathBuf::from("/state/router/dynamips_stdout.log"),
        options,
        ghost_file: None,
    };
    let commands = router_initialization_commands(&vm);
    assert!(commands.contains(&"c1700 set_iomem \"R1\" 15".to_owned()));
    assert!(!commands.contains(&"vm slot_add_binding \"R1\" 1 0 C1700-MB-WIC1".to_owned()));
}

#[test]
fn rejects_wics_that_the_selected_gns3_adapter_does_not_expose() {
    let mut options = DynamipsOptions::default();
    options.wics[2] = Some("WIC-1T".to_owned());
    assert!(validate_dynamips_options("c1700", Some("1720"), None, &options).is_err());

    let mut options = DynamipsOptions::default();
    options.wics[0] = Some("WIC-1T".to_owned());
    assert!(validate_dynamips_options("c7200", None, None, &options).is_err());
    assert!(validate_dynamips_options("c3745", None, None, &options).is_ok());
}

#[test]
fn configures_a_real_gns3_cisco_3620_platform_with_ethernet_module() {
    validate_platform_settings("c3600", Some("3620"), None).unwrap();
    let vm = DynamipsVm {
        node_id: "00000000-0000-0000-0000-00000000000a".to_owned(),
        name: "R1".to_owned(),
        platform: "c3600".to_owned(),
        chassis: Some("3620".to_owned()),
        slot0: "NM-1FE-TX".to_owned(),
        image: PathBuf::from("/images/c3620.image"),
        ram_mb: 192,
        nvram: 256,
        idle_pc: Some("0x603a8bac".to_owned()),
        startup_config: String::new(),
        working_dir: PathBuf::from("/tmp/router"),
        hypervisor_port: 7200,
        console_port: 7201,
        aux_port: None,
        log_file: PathBuf::from("/tmp/router/dynamips.log"),
        options: super::DynamipsOptions::default(),
        ghost_file: None,
    };
    let commands = router_initialization_commands(&vm);
    assert_eq!(commands[0], "vm create \"R1\" 1 c3600");
    assert!(commands.contains(&"vm set_nvram \"R1\" 256".to_owned()));
    assert!(commands.contains(&"c3600 set_chassis \"R1\" 3620".to_owned()));
    assert!(commands.contains(&"vm slot_add_binding \"R1\" 0 0 NM-1FE-TX".to_owned()));
    assert!(commands.contains(&"vm set_idle_pc \"R1\" 0x603a8bac".to_owned()));
    assert!(validate_platform_settings("c3600", Some("3600"), None).is_err());
    assert!(validate_platform_settings("c3600", Some("3620"), Some("C7200-IO-FE")).is_err());
}

#[test]
fn platform_integrated_slot_zero_adapters_are_not_added_twice() {
    let cases = [
        ("c1700", Some("1720"), "C1700-MB-1FE", false),
        ("c2600", Some("2651XM"), "C2600-MB-2FE", false),
        ("c2691", None, "GT96100-FE", false),
        ("c3600", Some("3660"), "Leopard-2FE", false),
        ("c3725", None, "GT96100-FE", false),
        ("c3745", None, "GT96100-FE", false),
        // The c7200 IO controller and replaceable modules are attached
        // explicitly after `vm create` by GNS3 and Dynamips.
        ("c7200", None, "C7200-IO-FE", true),
        ("c3600", Some("3620"), "NM-1FE-TX", true),
    ];
    for (platform, chassis, module, expect_binding) in cases {
        let vm = DynamipsVm {
            node_id: "00000000-0000-0000-0000-00000000000a".to_owned(),
            name: "R1".to_owned(),
            platform: platform.to_owned(),
            chassis: chassis.map(str::to_owned),
            slot0: module.to_owned(),
            image: PathBuf::from("/images/router.image"),
            ram_mb: 256,
            nvram: 128,
            idle_pc: None,
            startup_config: String::new(),
            working_dir: PathBuf::from("/tmp/router"),
            hypervisor_port: 7200,
            console_port: 7201,
            aux_port: None,
            log_file: PathBuf::from("/tmp/router/dynamips.log"),
            options: DynamipsOptions::default(),
            ghost_file: None,
        };
        let binding = format!("vm slot_add_binding \"R1\" 0 0 {module}");
        assert_eq!(
            router_initialization_commands(&vm)
                .iter()
                .any(|command| command == &binding),
            expect_binding,
            "unexpected slot zero binding for {platform} {chassis:?} {module}"
        );
        assert_eq!(
            integrated_slot0(platform, chassis),
            (!expect_binding).then_some(module)
        );
    }
}

#[tokio::test]
async fn exited_hypervisor_is_reported_stopped_and_reaped_during_cleanup() {
    use tokio::process::Command;

    let directory = tempfile::tempdir().unwrap();
    let log_file = directory.path().join("dynamips_stdout.log");
    let engine_log = directory.path().join("dynamips_engine.log");
    fs::write(&log_file, b"closed hypervisor log").unwrap();
    fs::write(&engine_log, b"closed hypervisor engine log").unwrap();
    let mut child = Command::new("sh")
        .args(["-c", "exit 0"])
        .process_group(0)
        .spawn()
        .unwrap();
    let process_group_id = child.id().unwrap();
    child.wait().await.unwrap();

    let runtime = DynamipsRuntime::with_paths(
        "dynamips",
        directory.path().join("images"),
        directory.path().join("state"),
    );
    runtime.routers.lock().await.insert(
        "dead-node".to_owned(),
        ManagedRouter {
            child,
            process_group_id,
            group_killed: false,
            hypervisor_port: 7200,
            log_file: log_file.clone(),
        },
    );
    runtime
        .bindings
        .lock()
        .await
        .insert("dead-node".to_owned(), 7202);
    let vm = DynamipsVm {
        node_id: "dead-node".to_owned(),
        name: "R1".to_owned(),
        platform: "c3725".to_owned(),
        chassis: None,
        slot0: "GT96100-FE".to_owned(),
        image: directory.path().join("router.image"),
        ram_mb: 128,
        nvram: 256,
        idle_pc: None,
        startup_config: String::new(),
        working_dir: directory.path().join("state/router"),
        hypervisor_port: 7200,
        console_port: 7201,
        aux_port: None,
        log_file: log_file.clone(),
        options: DynamipsOptions::default(),
        ghost_file: None,
    };

    assert_eq!(runtime.status(&vm).await.unwrap(), "stopped");
    assert_eq!(
        runtime
            .lifecycle(&vm, crate::runtime::NodeAction::Stop)
            .await
            .unwrap(),
        "stopped"
    );
    assert!(!runtime.routers.lock().await.contains_key("dead-node"));
    assert!(!runtime.bindings.lock().await.contains_key("dead-node"));
    assert!(!log_file.exists());
    assert!(!engine_log.exists());
}

#[test]
fn dynamips_launch_commands_apply_platform_hardware_and_runtime_settings() {
    let directory = tempfile::tempdir().unwrap();
    let ghost = directory.path().join("router.image-512.ghost");
    fs::write(&ghost, b"ghost cache").unwrap();
    let options = DynamipsOptions {
        dynamips_id: 42,
        clock_divisor: 16,
        mmap: true,
        sparsemem: false,
        idlemax: 900,
        idlesleep: 80,
        exec_area: 128,
        disk0: 256,
        disk1: 512,
        auto_delete_disks: true,
        mac_addr: Some("aabb.ccdd.eeff".to_owned()),
        system_id: Some("FTX0945W0MY".to_owned()),
        slots: vec![
            Some("C7200-IO-FE".to_owned()),
            Some("PA-4E".to_owned()),
            None,
            None,
            None,
            None,
            None,
        ],
        wics: vec![None; 3],
        npe: Some("npe-200".to_owned()),
        midplane: Some("std".to_owned()),
        iomem: None,
        ghost_ios: true,
        ..DynamipsOptions::default()
    };
    validate_dynamips_options("c7200", None, Some("C7200-IO-FE"), &options).unwrap();
    let vm = DynamipsVm {
        node_id: "00000000-0000-0000-0000-00000000000a".to_owned(),
        name: "R42".to_owned(),
        platform: "c7200".to_owned(),
        chassis: None,
        slot0: "C7200-IO-FE".to_owned(),
        image: PathBuf::from("/images/router.image"),
        ram_mb: 512,
        nvram: 512,
        idle_pc: Some("0x603a8bac".to_owned()),
        startup_config: String::new(),
        working_dir: directory.path().join("router"),
        hypervisor_port: 7200,
        console_port: 7201,
        aux_port: None,
        log_file: directory.path().join("router/dynamips.log"),
        options,
        ghost_file: Some(ghost.clone()),
    };
    assert_eq!(
        router_start_idlepc_command(&vm).as_deref(),
        Some("vm set_idle_pc \"R42\" 0x603a8bac")
    );
    let mut no_selected_idlepc = vm.clone();
    no_selected_idlepc.idle_pc = None;
    assert_eq!(router_start_idlepc_command(&no_selected_idlepc), None);
    let commands = router_initialization_commands(&vm);
    assert_eq!(
        ghost_router_initialization_commands(&vm, &ghost),
        vec![
            "vm create \"Ghost\" 0 c7200".to_owned(),
            "vm set_ios \"Ghost\" \"/images/router.image\"".to_owned(),
            "vm set_ghost_status \"Ghost\" 1".to_owned(),
            format!("vm set_ghost_file \"Ghost\" \"{}\"", ghost.display()),
            "vm set_ram \"Ghost\" 512".to_owned(),
        ]
    );
    for expected in [
        "vm create \"R42\" 42 c7200",
        "vm set_clock_divisor \"R42\" 16",
        "vm set_ram_mmap \"R42\" 1",
        "vm set_sparse_mem \"R42\" 0",
        "vm set_exec_area \"R42\" 128",
        "vm set_disk0 \"R42\" 256",
        "vm set_disk1 \"R42\" 512",
        "c7200 set_npe \"R42\" npe-200",
        "c7200 set_midplane \"R42\" std",
        "c7200 set_mac_addr \"R42\" aabb.ccdd.eeff",
        "c7200 set_system_id \"R42\" FTX0945W0MY",
        "vm slot_add_binding \"R42\" 1 0 PA-4E",
        "vm set_ghost_status \"R42\" 2",
    ] {
        assert!(
            commands.iter().any(|command| command == expected),
            "missing {expected}"
        );
    }
    assert!(
        commands
            .iter()
            .any(|command| command.contains(&ghost.display().to_string()))
    );
    assert_eq!(
        router_start_settings_commands(&vm)
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "vm set_idle_max \"R42\" 0 900",
            "vm set_idle_sleep_time \"R42\" 0 80"
        ]
    );
    // GNS3 defaults leave Dynamips' own 500 / 30000 µs idle loop alone;
    // sending `30` would mean a 30 µs sleep and a busy host core.
    let mut gns3_defaults = vm.clone();
    gns3_defaults.options.idlemax = 500;
    gns3_defaults.options.idlesleep = 30;
    assert!(router_start_settings_commands(&gns3_defaults).is_empty());
    gns3_defaults.options.idlesleep = 30_000;
    assert_eq!(
        router_start_settings_commands(&gns3_defaults),
        ["vm set_idle_sleep_time \"R42\" 0 30000"]
    );
}

#[test]
fn accepts_every_upstream_dynamips_platform_and_chassis_family() {
    for platform in [
        "c1700", "c2600", "c2691", "c3600", "c3725", "c3745", "c7200",
    ] {
        let chassis = match platform {
            "c1700" => Some("1760"),
            "c2600" => Some("2651XM"),
            "c3600" => Some("3660"),
            _ => None,
        };
        validate_platform_settings(platform, chassis, None).unwrap();
    }
    assert_eq!(default_slot0("c3600", Some("3660")), Some("Leopard-2FE"));
    assert_eq!(default_slot0("c2600", Some("2651XM")), Some("C2600-MB-2FE"));
    validate_platform_settings("c2600", Some("2611"), None).unwrap();
}

#[path = "tests/runtime_cleanup.rs"]
mod runtime_cleanup;
