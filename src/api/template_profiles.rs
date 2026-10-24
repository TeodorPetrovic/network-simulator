use super::*;

pub(super) fn validate_template_profile_patch(
    template: &Value,
    patch: Value,
) -> Result<Value, ApiError> {
    let patch = patch
        .as_object()
        .ok_or_else(|| ApiError::bad_request("template settings must be an object"))?;
    let kind = template["template_type"].as_str().unwrap_or_default();
    let allowed: &[&str] = match kind {
        "qemu" => &[
            "name",
            "adapters",
            "memory",
            "cpus",
            "maxcpus",
            "linked_clone",
            "adapter_type",
            "hda_disk_interface",
            "hdb_disk_image",
            "hdb_disk_interface",
            "hdb_disk_image_backing_file",
            "hdc_disk_image",
            "hdc_disk_interface",
            "hdc_disk_image_backing_file",
            "hdd_disk_image",
            "hdd_disk_interface",
            "hdd_disk_image_backing_file",
            "hda_disk_image_backing_file",
            "cdrom_image",
            "bios_image",
            "kernel_image",
            "initrd",
            "kernel_command_line",
            "boot_priority",
            "console_type",
            "aux_type",
            "on_close",
            "process_priority",
            "options",
            "tpm",
            "uefi",
            "replicate_network_connection_state",
            "create_config_disk",
            "cpu_throttling",
        ],
        "docker" => &[
            "name",
            "adapters",
            "memory",
            "cpus",
            "start_command",
            "environment",
            "extra_hosts",
            "extra_volumes",
            "extra_configs",
            "startup_config_content",
            "console_type",
            "console_resolution",
            "console_http_port",
            "console_http_path",
            "aux_type",
            "custom_adapters",
        ],
        "dynamips" => &[
            "name",
            "platform",
            "chassis",
            "memory",
            "nvram",
            "idlepc",
            "mmap",
            "sparsemem",
            "clock_divisor",
            "idlemax",
            "idlesleep",
            "exec_area",
            "disk0",
            "disk1",
            "auto_delete_disks",
            "private_config",
            "mac_addr",
            "system_id",
            "slot0",
            "slot1",
            "slot2",
            "slot3",
            "slot4",
            "slot5",
            "slot6",
            "wic0",
            "wic1",
            "wic2",
            "npe",
            "midplane",
            "iomem",
            "ghost_ios",
            "console_type",
            "aux_type",
        ],
        "vpcs" => &[
            "name",
            "startup_script",
            "base_script_file",
            "console_type",
            "console_auto_start",
        ],
        "ethernet_switch" => &["name", "ports_mapping", "console_type"],
        "ethernet_hub" => &["name", "ports_mapping"],
        "cloud" => &[
            "name",
            "ports_mapping",
            "remote_console_host",
            "remote_console_port",
            "remote_console_type",
            "remote_console_http_path",
        ],
        "nat" => &["name", "ports_mapping"],
        _ => return Err(ApiError::bad_request("unsupported template runtime")),
    };
    let mut validated = serde_json::Map::new();
    for (key, value) in patch {
        if !allowed.contains(&key.as_str()) {
            return Err(ApiError::bad_request(format!(
                "'{key}' is not an editable {kind} template setting"
            )));
        }
        match key.as_str() {
            "name" => {
                let name = value
                    .as_str()
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| ApiError::bad_request("template name must not be empty"))?;
                if name.len() > 120 {
                    return Err(ApiError::bad_request("template name is too long"));
                }
                validated.insert(key.clone(), json!(name));
            }
            "ports_mapping" => {
                let mapping =
                    crate::builtin::BuiltinRuntime::normalize_template_ports(kind, value.clone())
                        .map_err(ApiError::bad_request)?;
                validated.insert(key.clone(), mapping);
            }
            "console_type" if kind == "ethernet_switch" => {
                let console_type = value
                    .as_str()
                    .filter(|value| matches!(*value, "none" | "telnet"))
                    .ok_or_else(|| ApiError::bad_request("unsupported switch console_type"))?;
                validated.insert(key.clone(), json!(console_type));
            }
            "remote_console_host" => {
                let host = value
                    .as_str()
                    .filter(|host| {
                        host.len() <= 255 && !host.bytes().any(|byte| byte.is_ascii_control())
                    })
                    .ok_or_else(|| ApiError::bad_request("remote_console_host is invalid"))?;
                validated.insert(key.clone(), json!(host));
            }
            "remote_console_port" => {
                let port = value
                    .as_u64()
                    .filter(|port| (1..=65535).contains(port))
                    .ok_or_else(|| {
                        ApiError::bad_request("remote_console_port must be from 1 to 65535")
                    })?;
                validated.insert(key.clone(), json!(port));
            }
            "remote_console_type" => {
                let console_type = value
                    .as_str()
                    .filter(|value| {
                        matches!(
                            *value,
                            "none" | "telnet" | "ssh" | "vnc" | "spice" | "http" | "https"
                        )
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request("unsupported cloud remote console type")
                    })?;
                validated.insert(key.clone(), json!(console_type));
            }
            "remote_console_http_path" => {
                let path = value
                    .as_str()
                    .filter(|path| {
                        path.starts_with('/') && !path.bytes().any(|byte| byte.is_ascii_control())
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request("remote_console_http_path must be an absolute path")
                    })?;
                validated.insert(key.clone(), json!(path));
            }
            "adapters" => {
                let adapters = value
                    .as_u64()
                    .filter(|value| {
                        if kind == "qemu" {
                            *value <= 275
                        } else if kind == "dynamips" {
                            (1..=99).contains(value)
                        } else {
                            (1..=99).contains(value)
                        }
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request(if kind == "qemu" {
                            "QEMU adapters must be an integer from 0 to 275"
                        } else {
                            "adapters must be an integer from 1 to 99"
                        })
                    })?;
                validated.insert(key.clone(), json!(adapters));
            }
            "memory" => {
                let memory = value
                    .as_u64()
                    .filter(|value| {
                        if kind == "qemu" {
                            (16..=1_048_576).contains(value)
                        } else if kind == "dynamips" {
                            (1..=1_048_576).contains(value)
                        } else {
                            *value <= 1_048_576
                        }
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request("memory is outside the supported range")
                    })?;
                validated.insert(key.clone(), json!(memory));
            }
            "cpus" if kind == "docker" => {
                let cpus = value
                    .as_f64()
                    .filter(|value| (0.0..=256.0).contains(value))
                    .ok_or_else(|| {
                        ApiError::bad_request("Docker CPU limit must be between 0 and 256")
                    })?;
                validated.insert(key.clone(), json!(cpus));
            }
            "cpus" => {
                let cpus = value
                    .as_u64()
                    .filter(|value| (1..=255).contains(value))
                    .ok_or_else(|| {
                        ApiError::bad_request("CPU count must be an integer from 1 to 256")
                    })?;
                validated.insert(key.clone(), json!(cpus));
            }
            "adapter_type" if kind == "qemu" => {
                let model = value
                    .as_str()
                    .filter(|value| {
                        matches!(
                            *value,
                            "e1000"
                                | "e1000-82544gc"
                                | "e1000-82545em"
                                | "e1000e"
                                | "i82550"
                                | "i82551"
                                | "i82557a"
                                | "i82557b"
                                | "i82557c"
                                | "i82558a"
                                | "i82558b"
                                | "i82559a"
                                | "i82559b"
                                | "i82559c"
                                | "i82559er"
                                | "i82562"
                                | "i82801"
                                | "igb"
                                | "ne2k_pci"
                                | "pcnet"
                                | "rocker"
                                | "rtl8139"
                                | "virtio"
                                | "virtio-net-pci"
                                | "vmxnet3"
                        )
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request("unsupported QEMU network adapter model")
                    })?;
                validated.insert(key.clone(), json!(model));
            }
            "hda_disk_interface" if kind == "qemu" => {
                let interface = value
                    .as_str()
                    .filter(|value| {
                        matches!(
                            *value,
                            "ide"
                                | "sata"
                                | "nvme"
                                | "scsi"
                                | "virtio"
                                | "sd"
                                | "mtd"
                                | "floppy"
                                | "pflash"
                                | "none"
                        )
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request("unsupported QEMU primary disk interface")
                    })?;
                validated.insert(key.clone(), json!(interface));
            }
            "boot_priority" if kind == "qemu" => {
                let order = value
                    .as_str()
                    .filter(|value| {
                        matches!(
                            *value,
                            "c" | "d" | "n" | "cn" | "cd" | "dn" | "dc" | "nc" | "nd"
                        )
                    })
                    .ok_or_else(|| ApiError::bad_request("unsupported QEMU boot priority"))?;
                validated.insert(key.clone(), json!(order));
            }
            "maxcpus" if kind == "qemu" => {
                let value = value
                    .as_u64()
                    .filter(|value| (1..=255).contains(value))
                    .ok_or_else(|| ApiError::bad_request("QEMU maxcpus must be from 1 to 255"))?;
                validated.insert(key.clone(), json!(value));
            }
            "linked_clone"
            | "tpm"
            | "uefi"
            | "replicate_network_connection_state"
            | "create_config_disk"
                if kind == "qemu" =>
            {
                let value = value
                    .as_bool()
                    .ok_or_else(|| ApiError::bad_request(format!("{key} must be a boolean")))?;
                validated.insert(key.clone(), json!(value));
            }
            "hdb_disk_interface" | "hdc_disk_interface" | "hdd_disk_interface"
                if kind == "qemu" =>
            {
                let interface = value
                    .as_str()
                    .filter(|value| {
                        matches!(
                            *value,
                            "ide"
                                | "sata"
                                | "nvme"
                                | "scsi"
                                | "virtio"
                                | "sd"
                                | "mtd"
                                | "floppy"
                                | "pflash"
                                | "none"
                        )
                    })
                    .ok_or_else(|| ApiError::bad_request(format!("unsupported QEMU {key}")))?;
                validated.insert(key.clone(), json!(interface));
            }
            "hdb_disk_image" | "hdc_disk_image" | "hdd_disk_image" | "cdrom_image"
            | "bios_image" | "kernel_image" | "initrd"
                if kind == "qemu" =>
            {
                let path = value
                    .as_str()
                    .filter(|value| value.len() <= 4096)
                    .ok_or_else(|| {
                        ApiError::bad_request(format!("{key} must be a path under 4096 characters"))
                    })?;
                validated.insert(key.clone(), json!(path));
            }
            "kernel_command_line" | "options" if kind == "qemu" => {
                let max = if key == "options" {
                    16 * 1024
                } else {
                    64 * 1024
                };
                let text = value
                    .as_str()
                    .filter(|text| text.len() <= max)
                    .ok_or_else(|| ApiError::bad_request(format!("{key} is too long")))?;
                validated.insert(key.clone(), json!(text));
            }
            "console_type" | "aux_type" if kind == "qemu" => {
                let value = value
                    .as_str()
                    .filter(|value| {
                        matches!(
                            *value,
                            "telnet" | "ssh" | "vnc" | "spice" | "spice+agent" | "none"
                        )
                    })
                    .ok_or_else(|| ApiError::bad_request(format!("unsupported QEMU {key}")))?;
                validated.insert(key.clone(), json!(value));
            }
            "on_close" if kind == "qemu" => {
                let value = value
                    .as_str()
                    .filter(|value| {
                        matches!(*value, "power_off" | "shutdown_signal" | "save_vm_state")
                    })
                    .ok_or_else(|| ApiError::bad_request("unsupported QEMU on_close action"))?;
                validated.insert(key.clone(), json!(value));
            }
            "process_priority" if kind == "qemu" => {
                let value = value
                    .as_str()
                    .filter(|value| {
                        matches!(
                            *value,
                            "realtime" | "very high" | "high" | "normal" | "low" | "very low"
                        )
                    })
                    .ok_or_else(|| ApiError::bad_request("unsupported QEMU process priority"))?;
                validated.insert(key.clone(), json!(value));
            }
            "cpu_throttling" if kind == "qemu" => {
                let value = value
                    .as_u64()
                    .filter(|value| *value <= 800)
                    .ok_or_else(|| {
                        ApiError::bad_request("QEMU cpu_throttling must be from 0 to 800")
                    })?;
                validated.insert(key.clone(), json!(value));
            }
            "platform" if kind == "dynamips" => {
                let platform = value
                    .as_str()
                    .filter(|value| {
                        matches!(
                            *value,
                            "c1700" | "c2600" | "c2691" | "c3600" | "c3725" | "c3745" | "c7200"
                        )
                    })
                    .ok_or_else(|| ApiError::bad_request("unsupported Dynamips platform"))?;
                validated.insert(key.clone(), json!(platform));
            }
            "chassis" | "slot0" | "slot1" | "slot2" | "slot3" | "slot4" | "slot5" | "slot6"
            | "wic0" | "wic1" | "wic2" | "npe" | "midplane" | "system_id" | "mac_addr"
                if kind == "dynamips" =>
            {
                let setting = value
                    .as_str()
                    .ok_or_else(|| ApiError::bad_request("Dynamips setting must be text"))?;
                validated.insert(key.clone(), json!(setting));
            }
            "nvram" | "clock_divisor" | "idlemax" | "idlesleep" | "exec_area" | "disk0"
            | "disk1" | "iomem"
                if kind == "dynamips" =>
            {
                let value = value
                    .as_u64()
                    .filter(|value| match key.as_str() {
                        "iomem" => *value <= 100,
                        "clock_divisor" => (1..=4096).contains(value),
                        "disk0" | "disk1" | "exec_area" => *value <= 16_384,
                        _ => *value <= 1_048_576,
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request(format!("{key} is outside the supported range"))
                    })?;
                validated.insert(key.clone(), json!(value));
            }
            "idlepc" if kind == "dynamips" => {
                let value = value
                    .as_str()
                    .filter(|value| value.is_empty() || value.starts_with("0x"))
                    .ok_or_else(|| {
                        ApiError::bad_request("Idle-PC must be empty or a hexadecimal value")
                    })?;
                validated.insert(key.clone(), json!(value));
            }
            "mmap" | "sparsemem" | "auto_delete_disks" | "ghost_ios" | "console_auto_start"
                if kind == "dynamips" || kind == "vpcs" =>
            {
                let value = value
                    .as_bool()
                    .ok_or_else(|| ApiError::bad_request(format!("{key} must be a boolean")))?;
                validated.insert(key.clone(), json!(value));
            }
            "private_config" | "startup_script" | "base_script_file"
                if kind == "dynamips" || kind == "vpcs" =>
            {
                let value = value
                    .as_str()
                    .filter(|value| value.len() <= 1_048_576)
                    .ok_or_else(|| {
                        ApiError::bad_request(format!("{key} must be text under 1 MiB"))
                    })?;
                validated.insert(key.clone(), json!(value));
            }
            "console_type" | "aux_type" if kind == "dynamips" => {
                let value = value
                    .as_str()
                    .filter(|value| matches!(*value, "telnet" | "ssh" | "none"))
                    .ok_or_else(|| ApiError::bad_request(format!("unsupported Dynamips {key}")))?;
                validated.insert(key.clone(), json!(value));
            }
            "console_type" if kind == "docker" => {
                let value = value
                    .as_str()
                    .filter(|value| matches!(*value, "none" | "telnet" | "vnc" | "http" | "https"))
                    .ok_or_else(|| ApiError::bad_request("unsupported Docker console_type"))?;
                validated.insert(key.clone(), json!(value));
            }
            "aux_type" if kind == "docker" => {
                let value = value
                    .as_str()
                    .filter(|value| matches!(*value, "none" | "telnet" | "ssh"))
                    .ok_or_else(|| ApiError::bad_request("unsupported Docker aux_type"))?;
                validated.insert(key.clone(), json!(value));
            }
            "console_resolution" if kind == "docker" => {
                let resolution = value
                    .as_str()
                    .filter(|value| {
                        value.split_once('x').is_some_and(|(w, h)| {
                            !w.is_empty()
                                && !h.is_empty()
                                && w.bytes().all(|byte| byte.is_ascii_digit())
                                && h.bytes().all(|byte| byte.is_ascii_digit())
                        })
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request("console_resolution must use WIDTHxHEIGHT")
                    })?;
                validated.insert(key.clone(), json!(resolution));
            }
            "console_http_port" if kind == "docker" => {
                let port = value
                    .as_u64()
                    .filter(|port| (1..=65535).contains(port))
                    .ok_or_else(|| {
                        ApiError::bad_request("console_http_port must be from 1 to 65535")
                    })?;
                validated.insert(key.clone(), json!(port));
            }
            "console_http_path" if kind == "docker" => {
                let path = value
                    .as_str()
                    .filter(|path| {
                        path.starts_with('/') && !path.bytes().any(|byte| byte.is_ascii_control())
                    })
                    .ok_or_else(|| {
                        ApiError::bad_request("console_http_path must be an absolute URL path")
                    })?;
                validated.insert(key.clone(), json!(path));
            }
            "custom_adapters" if kind == "docker" => {
                if !value.is_array() || value.as_array().is_some_and(|adapters| adapters.len() > 99)
                {
                    return Err(ApiError::bad_request(
                        "custom_adapters must contain at most 99 entries",
                    ));
                }
                serde_json::from_value::<Vec<crate::runtime::DockerCustomAdapter>>(value.clone())
                    .map_err(|_| {
                    ApiError::bad_request("custom_adapters contain an invalid definition")
                })?;
                validated.insert(key.clone(), value.clone());
            }
            "console_type" if kind == "vpcs" => {
                let value = value
                    .as_str()
                    .filter(|value| matches!(*value, "telnet" | "ssh" | "none"))
                    .ok_or_else(|| {
                        ApiError::bad_request("VPCS console_type must be telnet, ssh, or none")
                    })?;
                validated.insert(key.clone(), json!(value));
            }
            "extra_volumes" if kind == "docker" => {
                let volumes = value.as_array().ok_or_else(|| {
                    ApiError::bad_request("extra_volumes must be an array of container paths")
                })?;
                if volumes.len() > 32
                    || volumes.iter().any(|volume| {
                        volume
                            .as_str()
                            .is_none_or(|path| !valid_container_path(path))
                    })
                {
                    return Err(ApiError::bad_request(
                        "Docker volume paths must be absolute, safe container paths (up to 32)",
                    ));
                }
                validated.insert(key.clone(), value.clone());
            }
            "environment" | "extra_hosts" | "start_command" | "startup_script"
                if matches!(kind, "docker" | "vpcs") =>
            {
                let text = value
                    .as_str()
                    .ok_or_else(|| ApiError::bad_request(format!("{key} must be text")))?;
                if text.len() > 64 * 1024 {
                    return Err(ApiError::bad_request(format!("{key} is too long")));
                }
                if key == "start_command" && !text.trim().is_empty() {
                    parse_start_command(text)
                        .map_err(|error| ApiError::bad_request(error.message()))?;
                }
                validated.insert(key.clone(), json!(text));
            }
            "startup_config_content" if kind == "docker" => {
                let text = value
                    .as_str()
                    .filter(|text| text.len() <= 1024 * 1024)
                    .ok_or_else(|| {
                        ApiError::bad_request("startup_config_content must be text under 1 MiB")
                    })?;
                validated.insert(key.clone(), json!(text));
            }
            "extra_configs" if kind == "docker" => {
                let configs = validate_docker_extra_configs_api(Some(value), None)?;
                validated.insert(
                    key.clone(),
                    serde_json::to_value(configs).map_err(internal_json_error)?,
                );
            }
            _ => {
                return Err(ApiError::bad_request(format!(
                    "'{key}' is not valid for {kind}"
                )));
            }
        }
    }
    Ok(Value::Object(validated))
}
