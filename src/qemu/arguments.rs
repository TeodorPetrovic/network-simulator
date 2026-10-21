use super::*;

pub(super) fn qemu_runtime_arguments(vm: &QemuVm) -> Result<Vec<String>, RuntimeError> {
    if vm.uefi && vm.bios_image.is_some() {
        return Err(RuntimeError::Failed(
            "QEMU cannot use a custom BIOS image and UEFI boot at the same time".to_owned(),
        ));
    }
    if vm.aux_type != "none" && vm.aux_type == vm.console_type {
        return Err(RuntimeError::Failed(format!(
            "QEMU auxiliary console type '{}' cannot match the main console type",
            vm.aux_type
        )));
    }
    let mut args = vec![
        "-name".to_owned(),
        vm.node_id.clone(),
        "-uuid".to_owned(),
        vm.node_id.clone(),
        "-accel".to_owned(),
        if kvm_accessible() { "kvm" } else { "tcg" }.to_owned(),
        "-m".to_owned(),
        vm.memory_mb.to_string(),
        "-smp".to_owned(),
        format!(
            "cpus={},maxcpus={},sockets=1",
            vm.cpus,
            vm.maxcpus.max(vm.cpus)
        ),
        "-boot".to_owned(),
        format!("order={}", vm.boot_priority),
    ];

    let mut graphical = false;
    for (index, (console_type, port)) in [
        (&vm.console_type, vm.console_port),
        (&vm.aux_type, vm.aux_port),
    ]
    .into_iter()
    .enumerate()
    {
        let Some(port) = port else {
            continue;
        };
        match console_type.as_str() {
            "telnet" | "ssh" if index == 0 => {}
            "telnet" | "ssh" => args.extend([
                "-serial".to_owned(),
                format!("telnet:127.0.0.1:{port},server=on,wait=off"),
            ]),
            "vnc" => {
                graphical = true;
                let display = port
                    .checked_sub(5900)
                    .filter(|display| *display < 1000)
                    .ok_or_else(|| {
                        RuntimeError::Failed(
                            "QEMU VNC console port is outside the display range".to_owned(),
                        )
                    })?;
                args.extend(["-vnc".to_owned(), format!("127.0.0.1:{display}")]);
                if console_type == &vm.console_type {
                    args.extend(["-machine".to_owned(), "usb=on".to_owned()]);
                    args.extend(["-device".to_owned(), "usb-tablet".to_owned()]);
                }
            }
            "spice" | "spice+agent" => {
                graphical = true;
                args.extend([
                    "-spice".to_owned(),
                    format!("addr=127.0.0.1,port={port},disable-ticketing"),
                    "-vga".to_owned(),
                    "qxl".to_owned(),
                ]);
                if console_type == "spice+agent" {
                    args.extend([
                        "-device".to_owned(),
                        "virtio-serial".to_owned(),
                        "-chardev".to_owned(),
                        "spicevmc,id=vdagent,debug=0,name=vdagent".to_owned(),
                        "-device".to_owned(),
                        "virtserialport,chardev=vdagent,name=com.redhat.spice.0".to_owned(),
                    ]);
                }
            }
            "none" => {}
            other => {
                return Err(RuntimeError::Failed(format!(
                    "Unsupported QEMU console type '{other}'"
                )));
            }
        }
    }
    if !graphical {
        args.extend(["-display".to_owned(), "none".to_owned()]);
    }

    for disk in &vm.disks {
        args.extend(qemu_disk_arguments(disk)?);
    }
    if let Some(cdrom) = &vm.cdrom_image {
        args.extend([
            "-cdrom".to_owned(),
            qemu_keyval_escape(&cdrom.display().to_string()),
        ]);
    }
    if let Some(kernel) = &vm.kernel_image {
        args.extend([
            "-kernel".to_owned(),
            qemu_keyval_escape(&kernel.display().to_string()),
        ]);
    }
    if let Some(initrd) = &vm.initrd {
        args.extend([
            "-initrd".to_owned(),
            qemu_keyval_escape(&initrd.display().to_string()),
        ]);
    }
    if !vm.kernel_command_line.is_empty() {
        args.extend(["-append".to_owned(), vm.kernel_command_line.clone()]);
    }
    if let Some(bios) = &vm.bios_image {
        args.extend([
            "-bios".to_owned(),
            qemu_keyval_escape(&bios.display().to_string()),
        ]);
    } else if vm.uefi {
        args.extend(uefi_arguments(vm)?);
    }
    if vm.tpm {
        args.extend([
            "-chardev".to_owned(),
            format!(
                "socket,id=chrtpm,path={}",
                qemu_keyval_escape(&tpm_socket_path(vm).display().to_string())
            ),
            "-tpmdev".to_owned(),
            "emulator,id=tpm0,chardev=chrtpm".to_owned(),
            "-device".to_owned(),
            "tpm-tis,tpmdev=tpm0".to_owned(),
        ]);
    }
    if vm.on_close == "save_vm_state" {
        args.extend(["-loadvm".to_owned(), "GNS3_SAVED_STATE".to_owned()]);
    }
    validate_extra_options(&vm.extra_options)?;
    let additional = shlex::split(&vm.extra_options).expect("validated QEMU option syntax");
    args.extend(additional);
    Ok(args)
}

pub(super) fn validate_extra_options(options: &str) -> Result<(), RuntimeError> {
    if options.len() > 16 * 1024 {
        return Err(RuntimeError::Failed(
            "QEMU additional options are too long".to_owned(),
        ));
    }
    let tokens = shlex::split(options).ok_or_else(|| {
        RuntimeError::Failed("QEMU additional options contain unmatched quotes".to_owned())
    })?;
    let mut index = 0;
    while index < tokens.len() {
        let option = tokens[index].as_str();
        match option {
            "-no-reboot" | "-no-shutdown" | "-no-hpet" | "-no-acpi" | "-nodefaults"
            | "-no-user-config" => index += 1,
            "-cpu" | "-rtc" => {
                let value = tokens.get(index + 1).ok_or_else(|| {
                    RuntimeError::Failed(format!("QEMU additional option '{option}' needs a value"))
                })?;
                if value.is_empty()
                    || !value.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric()
                            || matches!(byte, b'_' | b'-' | b'.' | b'=' | b'+' | b',')
                    })
                {
                    return Err(RuntimeError::Failed(format!(
                        "QEMU additional option '{option}' has an unsupported value"
                    )));
                }
                index += 2;
            }
            _ => {
                return Err(RuntimeError::Failed(format!(
                    "QEMU additional option '{option}' is not in the safe option allowlist"
                )));
            }
        }
    }
    Ok(())
}

pub(super) fn qemu_disk_arguments(disk: &QemuDisk) -> Result<Vec<String>, RuntimeError> {
    let interface = disk.interface.as_str();
    if interface == "none" {
        return Ok(Vec::new());
    }
    if disk.format.is_empty() || !disk.format.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err(RuntimeError::Failed(
            "QEMU disk format contains unsupported characters".to_owned(),
        ));
    }
    let index = match disk.slot.as_str() {
        "hda" => 0,
        "hdb" => 1,
        "hdc" => 2,
        "hdd" => 3,
        other => {
            return Err(RuntimeError::Failed(format!(
                "Unknown QEMU disk slot '{other}'"
            )));
        }
    };
    let file = qemu_keyval_escape(&disk.active_file.display().to_string());
    let format = &disk.format;
    let options = match interface {
        "sata" => vec![
            "-device".to_owned(),
            format!("ahci,id=ahci{index}"),
            "-drive".to_owned(),
            format!("file={file},if=none,id=drive{index},index={index},media=disk,format={format}"),
            "-device".to_owned(),
            format!("ide-hd,drive=drive{index},bus=ahci{index}.0,id=drive{index}"),
        ],
        "nvme" => vec![
            "-drive".to_owned(),
            format!("file={file},if=none,id=drive{index},index={index},media=disk,format={format}"),
            "-device".to_owned(),
            format!("nvme,drive=drive{index},serial={index}"),
        ],
        "scsi" => vec![
            "-device".to_owned(),
            format!("virtio-scsi-pci,id=scsi{index}"),
            "-drive".to_owned(),
            format!("file={file},if=none,id=drive{index},index={index},media=disk,format={format}"),
            "-device".to_owned(),
            format!("scsi-hd,drive=drive{index}"),
        ],
        "sd" => vec![
            "-drive".to_owned(),
            format!("file={file},if=none,id=drive{index},index={index},media=disk,format={format}"),
            "-device".to_owned(),
            format!("sd-card,drive=drive{index},id=drive{index}"),
        ],
        "ide" | "virtio" | "mtd" | "floppy" | "pflash" => vec![
            "-drive".to_owned(),
            format!(
                "file={file},if={interface},index={index},media=disk,id=drive{index},format={format}"
            ),
        ],
        other => {
            return Err(RuntimeError::Failed(format!(
                "Unsupported QEMU disk interface '{other}'"
            )));
        }
    };
    Ok(options)
}

pub(super) fn uefi_arguments(vm: &QemuVm) -> Result<Vec<String>, RuntimeError> {
    let code = locate_ovmf_file("OVMF_CODE_4M.fd", &vm.image)?;
    let vars_source = locate_ovmf_file("OVMF_VARS_4M.fd", &vm.image)?;
    let vars_path = vm
        .pid_file
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("OVMF_VARS_4M.fd");
    if !vars_path.exists() {
        fs::copy(&vars_source, &vars_path).map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not create node UEFI variables file: {error}"
            ))
        })?;
    }
    Ok(vec![
        "-drive".to_owned(),
        format!(
            "if=pflash,format=raw,readonly,file={}",
            qemu_keyval_escape(&code.display().to_string())
        ),
        "-drive".to_owned(),
        format!(
            "if=pflash,format=raw,file={}",
            qemu_keyval_escape(&vars_path.display().to_string())
        ),
        "-object".to_owned(),
        "rng-random,filename=/dev/urandom,id=rng0".to_owned(),
        "-device".to_owned(),
        "virtio-rng-pci,rng=rng0".to_owned(),
    ])
}

pub(super) fn locate_ovmf_file(name: &str, image: &Path) -> Result<PathBuf, RuntimeError> {
    let mut candidates = Vec::new();
    if let Some(directory) = std::env::var_os("NETWORK_SIMULATOR_OVMF_DIR") {
        candidates.push(PathBuf::from(directory).join(name));
    }
    if let Some(directory) = image.parent() {
        candidates.push(directory.join(name));
    }
    for directory in ["/usr/share/OVMF", "/usr/share/edk2/ovmf", "/usr/share/edk2"] {
        candidates.push(Path::new(directory).join(name));
    }
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .ok_or_else(|| {
            RuntimeError::NotFound(format!("Could not find required UEFI firmware '{name}'"))
        })
}

pub(super) fn set_child_process_priority(
    command: &mut Command,
    priority: &str,
) -> Result<(), RuntimeError> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let nice = match priority {
            "realtime" => -20,
            "very high" => -15,
            "high" => -5,
            "normal" => 0,
            "low" => 5,
            "very low" => 19,
            other => {
                return Err(RuntimeError::Failed(format!(
                    "Unsupported QEMU process priority '{other}'"
                )));
            }
        };
        // The daemonized QEMU process inherits this value from its launcher.
        unsafe {
            command.as_std_mut().pre_exec(move || {
                let _ = libc::setpriority(libc::PRIO_PROCESS, 0, nice);
                Ok(())
            });
        }
    }
    #[cfg(not(unix))]
    if priority != "normal" {
        return Err(RuntimeError::Failed(
            "QEMU process priority is only supported on Unix hosts".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn cpu_limiter_arguments(pid: u32, percentage: u32) -> [String; 3] {
    [
        "--lazy".to_owned(),
        format!("--pid={pid}"),
        format!("--limit={percentage}"),
    ]
}
