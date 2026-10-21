use super::*;

pub(super) fn appliance_versions(
    appliance: &Value,
    required_files: &[ApplianceFile],
) -> Vec<ApplianceVersion> {
    let versions = appliance
        .get("versions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let versions = if versions.is_empty() {
        vec![Value::Null]
    } else {
        versions
    };
    versions
        .iter()
        .map(|version| {
            let version_name = version
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Default")
                .to_owned();
            let settings_name = version
                .get("settings")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let runtime = version
                .get("settings")
                .and_then(Value::as_str)
                .and_then(|settings_name| {
                    appliance
                        .get("settings")
                        .and_then(Value::as_array)
                        .and_then(|settings| {
                            settings.iter().find(|setting| {
                                setting.get("name").and_then(Value::as_str) == Some(settings_name)
                            })
                        })
                        .and_then(|setting| setting.get("template_type"))
                        .and_then(Value::as_str)
                })
                .or_else(|| appliance_runtime_type(appliance));
            let properties = runtime.and_then(|runtime| {
                appliance_settings(appliance, runtime, settings_name.as_deref())
            });
            let mut images = Vec::new();
            if let Some(configured_images) = version.get("images").and_then(Value::as_object) {
                for (key, configured) in configured_images {
                    let filename = configured
                        .as_str()
                        .or_else(|| configured.get("filename").and_then(Value::as_str));
                    let Some(filename) = filename else {
                        continue;
                    };
                    let requirement = required_files
                        .iter()
                        .find(|image| image.filename.eq_ignore_ascii_case(filename));
                    images.push(ApplianceImageRequirement {
                        key: key.clone(),
                        filename: filename.to_owned(),
                        assignable: matches!(
                            (runtime, key.as_str()),
                            (Some("qemu"), "hda_disk_image") | (Some("dynamips"), "image")
                        ),
                        filesize: requirement.and_then(|image| image.size),
                        md5sum: requirement.and_then(|image| image.md5sum.clone()),
                        assigned_image_id: None,
                        assigned_image_name: None,
                        suggested_image_id: None,
                        suggested_image_name: None,
                        image_path: None,
                        actual_size: None,
                        actual_md5sum: None,
                        actual_sha1sum: None,
                        status: "missing".to_owned(),
                        allow_checksum_mismatch: false,
                    });
                }
            } else if let (Some(runtime), Some(properties)) = (runtime, properties) {
                let role = match runtime {
                    "qemu" => "hda_disk_image",
                    "dynamips" => "image",
                    _ => "",
                };
                if !role.is_empty()
                    && let Some(filename) = properties.get(role).and_then(Value::as_str)
                {
                    let requirement = required_files
                        .iter()
                        .find(|image| image.filename.eq_ignore_ascii_case(filename));
                    images.push(ApplianceImageRequirement {
                        key: role.to_owned(),
                        filename: filename.to_owned(),
                        assignable: true,
                        filesize: requirement.and_then(|image| image.size),
                        md5sum: requirement.and_then(|image| image.md5sum.clone()),
                        assigned_image_id: None,
                        assigned_image_name: None,
                        suggested_image_id: None,
                        suggested_image_name: None,
                        image_path: None,
                        actual_size: None,
                        actual_md5sum: None,
                        actual_sha1sum: None,
                        status: "missing".to_owned(),
                        allow_checksum_mismatch: false,
                    });
                }
            }
            let platform = properties
                .and_then(|properties| properties.get("platform"))
                .and_then(Value::as_str)
                .or_else(|| {
                    appliance
                        .get("dynamips")
                        .and_then(|value| value["platform"].as_str())
                });
            let runtime_supported = match runtime {
                Some("qemu") => {
                    version["images"]["hda_disk_image"]
                        .as_str()
                        .or_else(|| {
                            properties.and_then(|properties| properties["hda_disk_image"].as_str())
                        })
                        .is_some_and(|value| !value.is_empty())
                        && images.iter().all(|image| image.assignable)
                }
                Some("docker") => properties
                    .and_then(|properties| properties.get("image"))
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.is_empty()),
                Some("dynamips") => properties
                    .map(dynamips_configuration_supported)
                    .unwrap_or_else(|| platform == Some("c7200")),
                Some("vpcs") => true,
                _ => false,
            };
            let docker_image = (runtime == Some("docker"))
                .then(|| properties.and_then(|properties| properties["image"].as_str()))
                .flatten()
                .map(str::to_owned);
            ApplianceVersion {
                name: version_name,
                settings: settings_name,
                template_type: runtime.map(str::to_owned),
                runtime_supported,
                docker_image,
                docker_image_id: None,
                docker_image_available: false,
                images,
            }
        })
        .collect()
}

pub(super) fn local_appliance_images(
    base_templates: &[Value],
) -> Result<Vec<LocalApplianceImage>, String> {
    let mut images = Vec::new();
    for template in base_templates {
        let Some(template_type) = template["template_type"].as_str() else {
            continue;
        };
        let path_field = match template_type {
            "qemu" => "hda_disk_image",
            "dynamips" => "image",
            _ => continue,
        };
        let Some(path) = template[path_field].as_str() else {
            continue;
        };
        let path = Path::new(path).canonicalize().map_err(|error| {
            format!("Could not resolve local {template_type} image '{path}': {error}")
        })?;
        let metadata = fs::metadata(&path).map_err(|error| {
            format!(
                "Could not inspect local image '{}': {error}",
                path.display()
            )
        })?;
        if !metadata.is_file() {
            continue;
        }
        let image_id = template["template_id"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("Local image")
            .to_owned();
        images.push(LocalApplianceImage {
            image_id,
            name,
            template_type: template_type.to_owned(),
            path: path.display().to_string(),
            size: metadata.len(),
            md5sum: None,
            sha1sum: None,
        });
    }
    images.sort_by(|left, right| {
        left.template_type
            .cmp(&right.template_type)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(images)
}

pub(super) fn verification_status(requirement: &ApplianceImageRequirement) -> &'static str {
    if requirement.md5sum.is_some() {
        "verified"
    } else if requirement.filesize.is_some() {
        "size-verified"
    } else {
        "available-unverified"
    }
}

pub(super) fn appliance_runtime_type(appliance: &Value) -> Option<&str> {
    if let Some(settings) = appliance.get("settings").and_then(Value::as_array) {
        let preferred = settings
            .iter()
            .find(|settings| settings.get("default").and_then(Value::as_bool) == Some(true))
            .or_else(|| settings.first());
        if let Some(runtime) = preferred?.get("template_type").and_then(Value::as_str) {
            return Some(runtime);
        }
    }
    ["qemu", "dynamips", "docker", "iou", "vpcs"]
        .into_iter()
        .find(|key| appliance.get(*key).is_some())
}
