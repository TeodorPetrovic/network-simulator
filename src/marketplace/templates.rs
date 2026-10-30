use super::*;

pub(super) fn appliance_runtime_templates(
    record: &ApplianceRecord,
    appliance: &Value,
    base_templates: &[Value],
    bindings: &[PersistedImageBinding],
) -> Vec<Value> {
    let versions = appliance
        .get("versions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| vec![Value::Null]);
    let id = Uuid::parse_str(&record.appliance_id).ok();
    let mut templates = Vec::new();
    for version in versions {
        let version_name = version
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let version_settings = version.get("settings").and_then(Value::as_str);
        let version_summary = record.versions.iter().find(|variant| {
            variant.name
                == if version_name.is_empty() {
                    "Default"
                } else {
                    version_name
                }
        });
        let runtime = version_summary
            .and_then(|version| version.template_type.as_deref())
            .or_else(|| {
                version_settings
                    .and_then(|settings_name| {
                        appliance
                            .get("settings")
                            .and_then(Value::as_array)
                            .and_then(|settings| {
                                settings.iter().find(|setting| {
                                    setting.get("name").and_then(Value::as_str)
                                        == Some(settings_name)
                                })
                            })
                            .and_then(|setting| setting.get("template_type"))
                            .and_then(Value::as_str)
                    })
                    .or_else(|| appliance_runtime_type(appliance))
            });
        let Some(runtime) =
            runtime.filter(|runtime| matches!(*runtime, "qemu" | "dynamips" | "docker" | "vpcs"))
        else {
            continue;
        };
        // Do not create a runnable QEMU palette entry when the appliance
        // requires image slots this runtime cannot attach yet (HDB/HDC/HDD,
        // BIOS, or other auxiliary files). Showing only HDA would look ready
        // while silently discarding part of the registry configuration.
        if runtime == "qemu"
            && version_summary
                .is_some_and(|summary| summary.images.iter().any(|image| !image.assignable))
        {
            continue;
        }
        let properties = appliance_properties(appliance, runtime);
        let settings_properties =
            appliance_settings(appliance, runtime, version_settings).unwrap_or(properties);
        if runtime == "dynamips" && !dynamips_configuration_supported(settings_properties) {
            continue;
        }
        let image_name = match runtime {
            "qemu" => version["images"]["hda_disk_image"]
                .as_str()
                .or_else(|| properties["hda_disk_image"].as_str()),
            "dynamips" => version["images"]["image"]
                .as_str()
                .or_else(|| properties["image"].as_str()),
            "docker" => properties["image"].as_str(),
            "vpcs" => Some(""),
            _ => None,
        };
        let Some(image_name) = image_name else {
            continue;
        };
        let image_key = if runtime == "qemu" {
            "hda_disk_image"
        } else if runtime == "dynamips" {
            "image"
        } else {
            ""
        };
        let binding = bindings.iter().find(|binding| {
            binding.version
                == if version_name.is_empty() {
                    "Default"
                } else {
                    version_name
                }
                && binding.image_key == image_key
        });
        let local_template = base_templates.iter().find(|template| {
            template["template_type"].as_str() == Some(runtime)
                && if runtime == "vpcs" {
                    true
                } else if let Some(binding) = binding {
                    let path = if runtime == "qemu" {
                        template["hda_disk_image"].as_str()
                    } else {
                        template["image"].as_str()
                    };
                    path.is_some_and(|path| path == binding.image_path)
                } else {
                    let has_expected_hash = record
                        .versions
                        .iter()
                        .find(|variant| {
                            variant.name
                                == if version_name.is_empty() {
                                    "Default"
                                } else {
                                    version_name
                                }
                        })
                        .and_then(|variant| {
                            variant.images.iter().find(|image| image.key == image_key)
                        })
                        .and_then(|image| image.md5sum.as_ref())
                        .is_some();
                    (!has_expected_hash && template_image_matches(template, runtime, image_name))
                        || (runtime == "docker"
                            && template_image_matches(template, runtime, image_name))
                }
        });
        let Some(base_template) = local_template else {
            continue;
        };
        let mut template = base_template.clone();
        let display_name = if version_name.is_empty() {
            record.name.clone()
        } else {
            format!("{} · {}", record.name, version_name)
        };
        template["name"] = json!(display_name);
        if let Some(id) = id {
            template["template_id"] = json!(Uuid::new_v5(
                &id,
                format!("{runtime}:{}:{version_name}", base_template["template_id"]).as_bytes()
            ));
        }
        template["appliance_id"] = json!(record.appliance_id);
        template["appliance_file"] = json!(record.file_name);
        template["appliance_description"] = json!(record.description);
        template["appliance_version"] = json!(version_name);
        if let Some(adapters) = settings_properties["adapters"].as_u64() {
            template["adapters"] = json!(adapters);
        }
        if let Some(memory) = settings_properties["ram"].as_u64() {
            template["memory"] = json!(memory);
        }
        if let Some(cpus) = settings_properties["cpus"].as_u64() {
            template["cpus"] = json!(cpus);
        }
        if runtime == "qemu" {
            for key in [
                "platform",
                "adapter_type",
                "hda_disk_interface",
                "boot_priority",
                "linked_clone",
                "maxcpus",
                "hdb_disk_image",
                "hdb_disk_interface",
                "hdc_disk_image",
                "hdc_disk_interface",
                "hdd_disk_image",
                "hdd_disk_interface",
                "cdrom_image",
                "cdrom_image2",
                "bios_image",
                "kernel_image",
                "initrd",
                "kernel_command_line",
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
            ] {
                if let Some(value) = settings_properties.get(key) {
                    template[key] = value.clone();
                }
            }
            if settings_properties["ram"].is_u64() {
                template["memory"] = settings_properties["ram"].clone();
            }
        } else if runtime == "docker" {
            for key in [
                "environment",
                "extra_hosts",
                "extra_volumes",
                "start_command",
                "console_type",
                "console_resolution",
                "console_http_port",
                "console_http_path",
                "aux_type",
                "custom_adapters",
            ] {
                if let Some(value) = settings_properties.get(key) {
                    template[key] = value.clone();
                }
            }
            if settings_properties["ram"].is_u64() {
                template["memory"] = settings_properties["ram"].clone();
            }
            if let Some(image) = settings_properties["image"].as_str() {
                template["image"] = json!(image);
            }
        }
        if runtime == "dynamips" {
            if let Some(platform) = settings_properties["platform"].as_str() {
                template["platform"] = json!(platform);
                template["nvram"] = json!(
                    settings_properties["nvram"]
                        .as_u64()
                        .unwrap_or(if platform == "c3600" { 256 } else { 128 })
                );
                for key in [
                    "chassis",
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
                    "npe",
                    "midplane",
                    "iomem",
                    "ghost_ios",
                    "console_type",
                    "aux_type",
                ] {
                    if let Some(value) = settings_properties.get(key) {
                        template[key] = value.clone();
                    }
                }
                if settings_properties.get("slot0").is_none() {
                    let chassis = settings_properties["chassis"]
                        .as_str()
                        .or_else(|| template["chassis"].as_str())
                        .unwrap_or_default();
                    template["slot0"] = json!(default_dynamips_slot0(platform, chassis));
                }
            }
            let idlepc = settings_properties["idlepc"]
                .as_str()
                .or_else(|| version["idlepc"].as_str());
            if let Some(idlepc) = idlepc {
                template["idlepc"] = json!(idlepc);
            }
        } else if runtime == "vpcs" {
            for key in [
                "startup_script",
                "base_script_file",
                "console_type",
                "console_auto_start",
            ] {
                if let Some(value) = settings_properties.get(key) {
                    template[key] = value.clone();
                }
            }
        }
        templates.push(template);
    }
    templates
}

pub(super) fn dynamips_configuration_supported(properties: &Value) -> bool {
    let platform = properties["platform"].as_str().unwrap_or_default();
    let chassis = properties["chassis"].as_str().unwrap_or_default();
    let slot0 = properties["slot0"].as_str().unwrap_or_default();
    let chassis_valid = match platform {
        "c1700" => matches!(chassis, "" | "1720" | "1721" | "1750" | "1751" | "1760"),
        "c2600" => matches!(
            chassis,
            "" | "2610"
                | "2611"
                | "2620"
                | "2610XM"
                | "2620XM"
                | "2650XM"
                | "2621"
                | "2611XM"
                | "2621XM"
                | "2651XM"
        ),
        "c3600" => matches!(chassis, "3620" | "3640" | "3660"),
        "c2691" | "c3725" | "c3745" | "c7200" => chassis.is_empty(),
        _ => false,
    };
    matches!(
        platform,
        "c1700" | "c2600" | "c2691" | "c3600" | "c3725" | "c3745" | "c7200"
    ) && chassis_valid
        && dynamips_module_supported(platform, chassis, 0, slot0)
        && (1..=6).all(|slot| {
            properties
                .get(format!("slot{slot}"))
                .and_then(Value::as_str)
                .is_none_or(|module| dynamips_module_supported(platform, chassis, slot, module))
        })
}

pub(crate) fn default_dynamips_slot0(platform: &str, chassis: &str) -> &'static str {
    match platform {
        "c7200" => "C7200-IO-FE",
        "c1700" => "C1700-MB-1FE",
        "c2600" => match chassis {
            "2611" => "C2600-MB-2E",
            "2620" | "2610XM" | "2620XM" | "2650XM" => "C2600-MB-1FE",
            "2621" | "2611XM" | "2621XM" | "2651XM" => "C2600-MB-2FE",
            _ => "C2600-MB-1FE",
        },
        "c2691" | "c3725" | "c3745" => "GT96100-FE",
        "c3600" if chassis == "3660" => "Leopard-2FE",
        "c3600" => "NM-1FE-TX",
        _ => "",
    }
}

/// Infer a Dynamips platform from common IOS image filenames. IOS ELF headers
/// identify the executable format, not the emulated chassis, so filenames are
/// the available source of platform identity for standalone registered images.
pub(crate) fn detect_dynamips_platform(filename: &str) -> Option<(&'static str, &'static str)> {
    let name = Path::new(filename)
        .file_name()
        .and_then(|name| name.to_str())?
        .to_ascii_lowercase();
    // iShare names IOS files as `ishare-<id>-<original filename>`. Find the
    // platform token anywhere in the filename instead of assuming it starts
    // at byte zero; otherwise these registered images silently become 7200s.
    let platforms = [
        ("c2600", "c2600", ""),
        ("c2691", "c2691", ""),
        ("c3660", "c3600", "3660"),
        ("c3620", "c3600", "3620"),
        ("c3640", "c3600", "3640"),
        ("c3725", "c3725", ""),
        ("c3745", "c3745", ""),
        ("c7200", "c7200", ""),
        ("c1700", "c1700", ""),
        ("c1710", "c1700", ""),
    ];
    platforms
        .into_iter()
        .find(|(model, _, _)| {
            name.split(['-', '_', '.', '[', ']'])
                .any(|part| part == *model)
        })
        .map(|(_, platform, chassis)| (platform, chassis))
}

pub(super) fn dynamips_module_supported(
    platform: &str,
    chassis: &str,
    slot: usize,
    module: &str,
) -> bool {
    if module.is_empty() {
        return true;
    }
    match module {
        "C7200-IO-2FE" | "C7200-IO-FE" | "C7200-IO-GE-E" => platform == "c7200" && slot == 0,
        "PA-2FE-TX" | "PA-4E" | "PA-4T+" | "PA-8E" | "PA-8T" | "PA-A1" | "PA-FE-TX" | "PA-GE"
        | "PA-POS-OC3" => platform == "c7200" && (1..=6).contains(&slot),
        "C2600-MB-2FE" | "C2600-MB-1E" | "C2600-MB-2E" | "C2600-MB-1FE" => {
            platform == "c2600" && slot == 0
        }
        "C1700-MB-1FE" => platform == "c1700" && slot == 0,
        "C1700-MB-WIC1" => platform == "c1700" && slot == 1 && matches!(chassis, "1751" | "1760"),
        "GT96100-FE" => matches!(platform, "c2691" | "c3725" | "c3745") && slot == 0,
        "Leopard-2FE" => platform == "c3600" && chassis == "3660" && slot == 0,
        "NM-16ESW" | "NM-1E" | "NM-1FE-TX" | "NM-4E" | "NM-4T" => {
            matches!(platform, "c2600" | "c2691" | "c3600" | "c3725" | "c3745")
                && (slot > 0 || (platform == "c3600" && chassis != "3660"))
        }
        _ => false,
    }
}

pub(super) fn appliance_settings<'a>(
    appliance: &'a Value,
    runtime: &str,
    version_settings: Option<&str>,
) -> Option<&'a Value> {
    appliance
        .get("settings")
        .and_then(Value::as_array)
        .and_then(|settings| {
            settings
                .iter()
                .find(|setting| {
                    setting.get("name").and_then(Value::as_str) == version_settings
                        && setting.get("template_type").and_then(Value::as_str) == Some(runtime)
                })
                .or_else(|| {
                    settings.iter().find(|setting| {
                        setting.get("default").and_then(Value::as_bool) == Some(true)
                            && setting.get("template_type").and_then(Value::as_str) == Some(runtime)
                    })
                })
                .or_else(|| {
                    settings.iter().find(|setting| {
                        setting.get("template_type").and_then(Value::as_str) == Some(runtime)
                    })
                })
        })
        .and_then(|setting| setting.get("template_properties"))
        .or_else(|| appliance.get(runtime))
}

pub(super) fn appliance_properties<'a>(appliance: &'a Value, runtime: &str) -> &'a Value {
    static NULL_VALUE: Value = Value::Null;
    appliance_settings(appliance, runtime, None).unwrap_or(&NULL_VALUE)
}

pub(super) fn template_image_matches(template: &Value, runtime: &str, image_name: &str) -> bool {
    let candidate = if runtime == "qemu" {
        template["hda_disk_image"].as_str()
    } else {
        template["image"].as_str()
    };
    let Some(candidate) = candidate else {
        return false;
    };
    if runtime == "docker" {
        candidate == image_name
            || (!image_name.contains(':') && candidate.starts_with(&format!("{image_name}:")))
    } else {
        let candidate = Path::new(candidate)
            .file_name()
            .and_then(|name| name.to_str());
        let image_name = Path::new(image_name)
            .file_name()
            .and_then(|name| name.to_str());
        match (candidate, image_name) {
            (Some(candidate), Some(image_name)) => {
                candidate.eq_ignore_ascii_case(image_name)
                    || candidate
                        .to_ascii_lowercase()
                        .ends_with(&format!("-{}", image_name.to_ascii_lowercase()))
            }
            _ => false,
        }
    }
}
