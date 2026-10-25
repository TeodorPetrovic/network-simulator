use super::*;

impl QemuRuntime {
    pub async fn create_vm(
        &self,
        project_id: &str,
        node_id: &str,
        image: &str,
        memory_mb: u64,
        cpus: u32,
        adapters: u32,
    ) -> Result<QemuVm, RuntimeError> {
        self.create_vm_with_options(
            project_id,
            node_id,
            image,
            memory_mb,
            cpus,
            adapters,
            &QemuTemplateOptions::default(),
        )
        .await
    }

    pub async fn create_vm_with_options(
        &self,
        project_id: &str,
        node_id: &str,
        image: &str,
        memory_mb: u64,
        cpus: u32,
        adapters: u32,
        options: &QemuTemplateOptions,
    ) -> Result<QemuVm, RuntimeError> {
        let project_id = parse_id(project_id)?;
        let node_id = parse_id(node_id)?;
        let runtime_platform = self.platform();
        // The selected QEMU binary determines the architecture this runtime can launch.
        // The template must not advertise or persist another platform.
        let platform = options
            .platform
            .as_deref()
            .unwrap_or(runtime_platform.as_str())
            .to_owned();
        if platform != runtime_platform {
            return Err(RuntimeError::Failed(format!(
                "QEMU platform '{platform}' requires a matching qemu-system binary; this runtime uses '{}'. Configure NETWORK_SIMULATOR_QEMU for that architecture",
                self.binary.display()
            )));
        }
        let adapter_type = options.adapter_type.as_deref().unwrap_or("e1000");
        if !valid_adapter_type(adapter_type) {
            return Err(RuntimeError::Failed(
                "Unsupported QEMU network adapter model".to_owned(),
            ));
        }
        let disk_interface = options.hda_disk_interface.as_deref().unwrap_or("ide");
        if !valid_disk_interface(disk_interface) {
            return Err(RuntimeError::Failed(
                "Unsupported QEMU primary disk interface".to_owned(),
            ));
        }
        let boot_priority = options.boot_priority.as_deref().unwrap_or("c");
        if !matches!(
            boot_priority,
            "c" | "d" | "n" | "cn" | "cd" | "dn" | "dc" | "nc" | "nd"
        ) {
            return Err(RuntimeError::Failed(
                "Unsupported QEMU boot priority".to_owned(),
            ));
        }
        let image_source = options
            .hda_disk_image_backing_file
            .as_deref()
            .filter(|path| !path.trim().is_empty())
            .unwrap_or(image);
        let vm_dir = self
            .state_dir
            .join(project_id.to_string())
            .join(node_id.to_string());
        fs::create_dir_all(&vm_dir).map_err(|error| {
            RuntimeError::Failed(format!("Could not create QEMU state directory: {error}"))
        })?;
        fs::create_dir_all(&self.qmp_dir).map_err(|error| {
            RuntimeError::Failed(format!("Could not create QEMU socket directory: {error}"))
        })?;
        let image = self.resolve_node_image(image_source, &vm_dir)?;
        let config_disk_image = options
            .create_config_disk
            .unwrap_or(false)
            .then(|| self.config_disk_image())
            .transpose()?;
        if let Some(path) = &config_disk_image {
            if self.image_format(path).await? != "raw" {
                return Err(RuntimeError::Failed(
                    "QEMU config disk image must use the raw format".to_owned(),
                ));
            }
        }

        let linked_clone = options.linked_clone.unwrap_or(true);
        for disk in &options.disk_images_to_reset {
            if !matches!(
                disk.as_str(),
                "hda_disk_image" | "hdb_disk_image" | "hdc_disk_image" | "hdd_disk_image"
            ) {
                return Err(RuntimeError::Failed(format!(
                    "Unsupported QEMU disk reset property '{disk}'"
                )));
            }
        }
        let mut disks = Vec::new();
        let mut primary_overlay = image.clone();
        let disk_slots = [
            (
                "hda",
                Some(image.display().to_string()),
                Some(disk_interface),
                options.hda_disk_image_backing_file.as_deref(),
            ),
            (
                "hdb",
                options.hdb_disk_image.clone(),
                options.hdb_disk_interface.as_deref(),
                options.hdb_disk_image_backing_file.as_deref(),
            ),
            (
                "hdc",
                options.hdc_disk_image.clone(),
                options.hdc_disk_interface.as_deref(),
                options.hdc_disk_image_backing_file.as_deref(),
            ),
            (
                "hdd",
                options.hdd_disk_image.clone(),
                options.hdd_disk_interface.as_deref(),
                options.hdd_disk_image_backing_file.as_deref(),
            ),
        ];
        for (slot, source, interface, backing_file) in disk_slots {
            if slot == "hdd" && options.create_config_disk.unwrap_or(false) {
                continue;
            }
            let source = source
                .filter(|source| !source.trim().is_empty())
                .or_else(|| {
                    backing_file
                        .filter(|path| !path.trim().is_empty())
                        .map(str::to_owned)
                });
            let Some(source) = source else {
                continue;
            };
            let disk_image = if slot == "hda" {
                image.clone()
            } else {
                self.resolve_node_image(
                    backing_file
                        .filter(|path| !path.trim().is_empty())
                        .unwrap_or(&source),
                    &vm_dir,
                )?
            };
            let format = self.image_format(&disk_image).await?;
            let active_file = if linked_clone {
                let overlay = if slot == "hda" {
                    vm_dir.join(format!("{node_id}.qcow2"))
                } else {
                    vm_dir.join(format!("{node_id}-{slot}.qcow2"))
                };
                if options
                    .disk_images_to_reset
                    .iter()
                    .any(|reset| reset == &format!("{slot}_disk_image"))
                {
                    remove_disk_and_checksum(&overlay)?;
                }
                self.create_overlay(&disk_image, &format, &overlay).await?;
                if slot == "hda" {
                    primary_overlay = overlay.clone();
                }
                overlay
            } else {
                disk_image.clone()
            };
            let interface = interface.unwrap_or(if slot == "hda" { "ide" } else { "none" });
            if !valid_disk_interface(interface) {
                return Err(RuntimeError::Failed(format!(
                    "Unsupported QEMU {slot} disk interface"
                )));
            }
            disks.push(QemuDisk {
                slot: slot.to_owned(),
                image: disk_image,
                active_file,
                interface: interface.to_owned(),
                format: if linked_clone {
                    "qcow2".to_owned()
                } else {
                    format
                },
            });
        }
        if options.create_config_disk.unwrap_or(false) {
            let config_image = config_disk_image.expect("config disk image was resolved above");
            let config_disk = vm_dir.join("config.img");
            if !config_disk.exists() {
                fs::copy(&config_image, &config_disk).map_err(|error| {
                    RuntimeError::Failed(format!("Could not create QEMU config disk: {error}"))
                })?;
            }
            let interface = options
                .hdd_disk_interface
                .as_deref()
                .filter(|interface| *interface != "none")
                .unwrap_or(disk_interface);
            if !valid_disk_interface(interface) {
                return Err(RuntimeError::Failed(
                    "Unsupported QEMU config-disk interface".to_owned(),
                ));
            }
            disks.push(QemuDisk {
                slot: "hdd".to_owned(),
                image: config_image,
                active_file: config_disk,
                interface: interface.to_owned(),
                format: "raw".to_owned(),
            });
        }
        if disks.first().is_none_or(|disk| disk.slot != "hda") {
            return Err(RuntimeError::Failed("QEMU HDA disk is required".to_owned()));
        }
        if options
            .cdrom_image
            .as_deref()
            .is_some_and(|path| !path.trim().is_empty())
            && options
                .hdc_disk_image
                .as_deref()
                .is_some_and(|path| !path.trim().is_empty())
        {
            return Err(RuntimeError::Failed(
                "QEMU cannot use an HDC disk image and a CD-ROM image at the same time".to_owned(),
            ));
        }
        let resolve_optional = |path: Option<&String>| {
            path.filter(|path| !path.trim().is_empty())
                .map(|path| self.resolve_auxiliary_file(path))
                .transpose()
        };
        let cdrom_image = resolve_optional(options.cdrom_image.as_ref())?;
        let cdrom_image2 = resolve_optional(options.cdrom_image2.as_ref())?;
        let bios_image = resolve_optional(options.bios_image.as_ref())?;
        let kernel_image = resolve_optional(options.kernel_image.as_ref())?;
        let initrd = resolve_optional(options.initrd.as_ref())?;
        let console_type = options.console_type.as_deref().unwrap_or("telnet");
        let aux_type = options.aux_type.as_deref().unwrap_or("none");
        if !valid_console_type(console_type) || !valid_console_type(aux_type) {
            return Err(RuntimeError::Failed(
                "Unsupported QEMU console type".to_owned(),
            ));
        }
        let on_close = options.on_close.as_deref().unwrap_or("power_off");
        if !matches!(on_close, "power_off" | "shutdown_signal" | "save_vm_state") {
            return Err(RuntimeError::Failed(
                "Unsupported QEMU on_close action".to_owned(),
            ));
        }
        let process_priority = options.process_priority.as_deref().unwrap_or("normal");
        if !matches!(
            process_priority,
            "realtime" | "very high" | "high" | "normal" | "low" | "very low"
        ) {
            return Err(RuntimeError::Failed(
                "Unsupported QEMU process priority".to_owned(),
            ));
        }
        let extra_options = options.options.as_deref().unwrap_or_default();
        validate_extra_options(extra_options)?;
        let maxcpus = options.maxcpus.unwrap_or(cpus).max(cpus);
        if maxcpus > 255 || cpus == 0 || cpus > 255 || adapters > 275 {
            return Err(RuntimeError::Failed(
                "QEMU CPUs/maxcpus must be from 1 to 255 and adapters from 0 to 275".to_owned(),
            ));
        }
        let cpu_throttling = options.cpu_throttling.unwrap_or(0);
        if cpu_throttling > 800 {
            return Err(RuntimeError::Failed(
                "QEMU cpu_throttling must be from 0 to 800".to_owned(),
            ));
        }
        let console_port = if matches!(console_type, "vnc" | "spice" | "spice+agent") {
            Some(if console_type == "vnc" {
                available_vnc_port().await?
            } else {
                available_tcp_port().await?
            })
        } else if console_type != "none" {
            Some(available_tcp_port().await?)
        } else {
            None
        };
        let aux_port = if aux_type != "none" && matches!(aux_type, "vnc" | "spice" | "spice+agent")
        {
            Some(if aux_type == "vnc" {
                available_vnc_port().await?
            } else {
                available_tcp_port().await?
            })
        } else if aux_type != "none" {
            Some(available_tcp_port().await?)
        } else {
            None
        };
        let vm = QemuVm {
            node_id: node_id.to_string(),
            image,
            platform,
            overlay: primary_overlay,
            qmp_socket: self.qmp_dir.join(format!("{node_id}.sock")),
            serial_socket: self.qmp_dir.join(format!("{node_id}.serial.sock")),
            pid_file: vm_dir.join(format!("{node_id}.pid")),
            log_file: vm_dir.join(format!("{node_id}.log")),
            memory_mb,
            cpus,
            maxcpus,
            adapters,
            adapter_type: adapter_type.to_owned(),
            disk_interface: disk_interface.to_owned(),
            boot_priority: boot_priority.to_owned(),
            disks,
            cdrom_image,
            cdrom_image2,
            bios_image,
            kernel_image,
            initrd,
            kernel_command_line: options.kernel_command_line.clone().unwrap_or_default(),
            linked_clone,
            console_type: console_type.to_owned(),
            console_port,
            aux_type: aux_type.to_owned(),
            aux_port,
            on_close: on_close.to_owned(),
            process_priority: process_priority.to_owned(),
            extra_options: extra_options.to_owned(),
            tpm: options.tpm.unwrap_or(false),
            uefi: options.uefi.unwrap_or(false),
            replicate_network_connection_state: options
                .replicate_network_connection_state
                .unwrap_or(true),
            create_config_disk: options.create_config_disk.unwrap_or(false),
            cpu_throttling,
        };
        Ok(vm)
    }
}
