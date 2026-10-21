use super::*;

impl QemuRuntime {
    pub fn vm_from_node(&self, node: &Value) -> Result<QemuVm, RuntimeError> {
        let project_id = parse_id(node["project_id"].as_str().unwrap_or_default())?;
        let node_id = parse_id(node["node_id"].as_str().unwrap_or_default())?;
        let vm_dir = self
            .state_dir
            .join(project_id.to_string())
            .join(node_id.to_string());
        let image = self.resolve_node_image(
            node["hda_disk_image"].as_str().ok_or_else(|| {
                RuntimeError::Failed("QEMU node is missing 'hda_disk_image'".to_owned())
            })?,
            &vm_dir,
        )?;
        let linked_clone = node["linked_clone"].as_bool().unwrap_or(true);
        let expected_overlay = if linked_clone {
            vm_dir.join(format!("{node_id}.qcow2"))
        } else {
            image.clone()
        };
        let overlay = validated_runtime_path(node, "runtime_overlay", &expected_overlay)?;
        let qmp_socket = validated_runtime_path(
            node,
            "runtime_qmp_socket",
            &self.qmp_dir.join(format!("{node_id}.sock")),
        )?;
        let serial_socket = validated_optional_runtime_path(
            node,
            "runtime_serial_socket",
            &self.qmp_dir.join(format!("{node_id}.serial.sock")),
        )?;
        let pid_file = validated_runtime_path(
            node,
            "runtime_pid_file",
            &vm_dir.join(format!("{node_id}.pid")),
        )?;
        let log_file = validated_runtime_path(
            node,
            "runtime_log_file",
            &vm_dir.join(format!("{node_id}.log")),
        )?;
        let platform = node["platform"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| self.platform());
        if platform != self.platform() {
            return Err(RuntimeError::Failed(format!(
                "QEMU node platform '{platform}' does not match configured binary '{}'",
                self.binary.display()
            )));
        }
        let disk_interface = node["hda_disk_interface"]
            .as_str()
            .unwrap_or("ide")
            .to_owned();
        let mut disks = Vec::new();
        for (slot, image_key, overlay_key, interface_key, format_key) in [
            (
                "hda",
                "hda_disk_image",
                "runtime_overlay",
                "hda_disk_interface",
                "runtime_hda_format",
            ),
            (
                "hdb",
                "hdb_disk_image",
                "runtime_hdb_overlay",
                "hdb_disk_interface",
                "runtime_hdb_format",
            ),
            (
                "hdc",
                "hdc_disk_image",
                "runtime_hdc_overlay",
                "hdc_disk_interface",
                "runtime_hdc_format",
            ),
            (
                "hdd",
                "hdd_disk_image",
                "runtime_hdd_overlay",
                "hdd_disk_interface",
                "runtime_hdd_format",
            ),
        ] {
            let Some(image_text) = node[image_key]
                .as_str()
                .filter(|path| !path.trim().is_empty())
            else {
                continue;
            };
            let image_path = if slot == "hda" {
                image.clone()
            } else if slot == "hdd" && node["create_config_disk"].as_bool().unwrap_or(false) {
                self.config_disk_image()?
            } else {
                self.resolve_node_image(image_text, &vm_dir)?
            };
            let expected_active_file =
                if slot == "hdd" && node["create_config_disk"].as_bool().unwrap_or(false) {
                    vm_dir.join("config.img")
                } else if linked_clone {
                    vm_dir.join(if slot == "hda" {
                        format!("{node_id}.qcow2")
                    } else {
                        format!("{node_id}-{slot}.qcow2")
                    })
                } else {
                    image_path.clone()
                };
            let active_file = if slot == "hda" {
                overlay.clone()
            } else {
                validated_optional_runtime_path(node, overlay_key, &expected_active_file)?
            };
            let interface =
                node[interface_key]
                    .as_str()
                    .unwrap_or(if slot == "hda" { "ide" } else { "none" });
            if !valid_disk_interface(interface) {
                return Err(RuntimeError::Failed(format!(
                    "QEMU node has an invalid {slot} disk interface"
                )));
            }
            disks.push(QemuDisk {
                slot: slot.to_owned(),
                image: image_path,
                active_file,
                interface: interface.to_owned(),
                format: node[format_key]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| if linked_clone { "qcow2" } else { "" }.to_owned()),
            });
        }
        if disks.first().is_none_or(|disk| disk.slot != "hda") {
            return Err(RuntimeError::Failed("QEMU node has no HDA disk".to_owned()));
        }
        Ok(QemuVm {
            node_id: node_id.to_string(),
            image,
            platform,
            overlay,
            qmp_socket,
            serial_socket,
            pid_file,
            log_file,
            memory_mb: node["memory"].as_u64().unwrap_or(512).max(1),
            cpus: node["cpus"].as_u64().unwrap_or(1).max(1) as u32,
            maxcpus: node["maxcpus"]
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .unwrap_or_else(|| node["cpus"].as_u64().unwrap_or(1) as u32)
                .max(node["cpus"].as_u64().unwrap_or(1) as u32),
            adapters: node["adapters"].as_u64().unwrap_or(8).min(275) as u32,
            adapter_type: node["adapter_type"].as_str().unwrap_or("e1000").to_owned(),
            disk_interface,
            boot_priority: node["boot_priority"].as_str().unwrap_or("c").to_owned(),
            disks,
            cdrom_image: self.resolve_optional_auxiliary_file(node, "cdrom_image")?,
            bios_image: self.resolve_optional_auxiliary_file(node, "bios_image")?,
            kernel_image: self.resolve_optional_auxiliary_file(node, "kernel_image")?,
            initrd: self.resolve_optional_auxiliary_file(node, "initrd")?,
            kernel_command_line: node["kernel_command_line"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            linked_clone,
            console_type: node["console_type"].as_str().unwrap_or("telnet").to_owned(),
            console_port: node["console"]
                .as_u64()
                .and_then(|port| u16::try_from(port).ok()),
            aux_type: node["aux_type"].as_str().unwrap_or("none").to_owned(),
            aux_port: node["aux"]
                .as_u64()
                .and_then(|port| u16::try_from(port).ok()),
            on_close: node["on_close"].as_str().unwrap_or("power_off").to_owned(),
            process_priority: node["process_priority"]
                .as_str()
                .unwrap_or("normal")
                .to_owned(),
            extra_options: node["options"].as_str().unwrap_or_default().to_owned(),
            tpm: node["tpm"].as_bool().unwrap_or(false),
            uefi: node["uefi"].as_bool().unwrap_or(false),
            replicate_network_connection_state: node["replicate_network_connection_state"]
                .as_bool()
                .unwrap_or(true),
            create_config_disk: node["create_config_disk"].as_bool().unwrap_or(false),
            cpu_throttling: node["cpu_throttling"].as_u64().unwrap_or(0) as u32,
        })
        .and_then(|vm| {
            validate_extra_options(&vm.extra_options)?;
            if vm.disks.iter().any(|disk| {
                !disk.format.is_empty()
                    && !disk.format.bytes().all(|byte| byte.is_ascii_alphanumeric())
            }) {
                return Err(RuntimeError::Failed(
                    "QEMU node has an invalid disk format".to_owned(),
                ));
            }
            Ok(vm)
        })
    }

    pub fn runtime_fields(&self, vm: &QemuVm) -> Value {
        let mut runtime_fields = json!({
            "hda_disk_image": vm.image,
            "platform": vm.platform,
            "adapter_type": vm.adapter_type,
            "hda_disk_interface": vm.disk_interface,
            "boot_priority": vm.boot_priority,
            "linked_clone": vm.linked_clone,
            "maxcpus": vm.maxcpus,
            "cdrom_image": vm.cdrom_image,
            "bios_image": vm.bios_image,
            "kernel_image": vm.kernel_image,
            "initrd": vm.initrd,
            "kernel_command_line": vm.kernel_command_line,
            "console_type": vm.console_type,
            "aux_type": vm.aux_type,
            "on_close": vm.on_close,
            "process_priority": vm.process_priority,
            "options": vm.extra_options,
            "tpm": vm.tpm,
            "uefi": vm.uefi,
            "replicate_network_connection_state": vm.replicate_network_connection_state,
            "create_config_disk": vm.create_config_disk,
            "cpu_throttling": vm.cpu_throttling,
            "runtime_overlay": vm.overlay,
            "runtime_qmp_socket": vm.qmp_socket,
            "runtime_serial_socket": vm.serial_socket,
            "runtime_pid_file": vm.pid_file,
            "runtime_log_file": vm.log_file,
            "runtime_hda_format": vm.disks.first().map(|disk| disk.format.as_str()),
            "console": vm.console_port,
            "aux": vm.aux_port
        });
        if let Some(fields) = runtime_fields.as_object_mut() {
            for disk in &vm.disks {
                if disk.active_file != disk.image {
                    fields.insert(
                        format!("{}_disk_image_backing_file", disk.slot),
                        json!(disk.image.file_name().unwrap_or_default().to_string_lossy()),
                    );
                }
                let checksum = cached_md5(
                    &disk.active_file,
                    vm.log_file.parent().unwrap_or(Path::new(".")),
                );
                fields.insert(format!("{}_disk_image_md5sum", disk.slot), json!(checksum));
                if disk.slot == "hda" {
                    continue;
                }
                fields.insert(format!("{}_disk_image", disk.slot), json!(disk.image));
                fields.insert(
                    format!("{}_disk_interface", disk.slot),
                    json!(disk.interface),
                );
                fields.insert(format!("runtime_{}_format", disk.slot), json!(disk.format));
                if disk.active_file != disk.image {
                    fields.insert(
                        format!("runtime_{}_overlay", disk.slot),
                        json!(disk.active_file),
                    );
                }
            }
            for (key, path) in [
                ("cdrom_image_md5sum", vm.cdrom_image.as_deref()),
                ("bios_image_md5sum", vm.bios_image.as_deref()),
                ("kernel_image_md5sum", vm.kernel_image.as_deref()),
                ("initrd_md5sum", vm.initrd.as_deref()),
            ] {
                fields.insert(
                    key.to_owned(),
                    json!(path.and_then(|path| cached_md5(
                        path,
                        vm.log_file.parent().unwrap_or(Path::new("."))
                    ))),
                );
            }
        }
        runtime_fields
    }

}
