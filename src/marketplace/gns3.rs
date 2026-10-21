use super::*;

impl MarketplaceCatalog {
    pub async fn search_gns3_appliances(&self, query: &str) -> Result<Vec<Value>, String> {
        let query = validate_query(query)?.to_lowercase();
        let entries = self.gns3_appliance_files().await?;
        let mut matches = entries
            .into_iter()
            .filter(|entry| {
                entry.kind == "file"
                    && entry.name.ends_with(".gns3a")
                    && (query.is_empty() || entry.name.to_lowercase().contains(&query))
            })
            .take(MAX_GNS3_APPLIANCE_RESULTS)
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
        let imported_names = self.imported_appliance_file_names();
        Ok(matches
            .into_iter()
            .map(|entry| {
                let slug = entry.name.trim_end_matches(".gns3a");
                let imported = imported_names.contains(&entry.name);
                json!({
                    "file_name": entry.name,
                    "name": title_from_slug(slug),
                    "registry_path": entry.path,
                    "source_url": entry.html_url,
                    "download_url": entry.download_url.unwrap_or_else(|| format!("{GNS3_RAW_APPLIANCES}/{}", entry.name)),
                    "imported": imported
                })
            })
            .collect())
    }

    pub async fn appliance_details(&self, file_name: &str) -> Result<ApplianceRecord, String> {
        let file_name = validate_appliance_file_name(file_name)?;
        let appliance = self.fetch_appliance(file_name).await?;
        let imported = self.imported_appliance_file_names().contains(file_name);
        let record = summarize_appliance(file_name, &appliance, imported)?;
        Ok(record)
    }

    pub async fn import_appliance(&self, file_name: &str) -> Result<ApplianceRecord, String> {
        let file_name = validate_appliance_file_name(file_name)?;
        let appliance = self.fetch_appliance(file_name).await?;
        let record = summarize_appliance(file_name, &appliance, true)?;
        fs::create_dir_all(&self.appliance_dir).map_err(|error| {
            format!(
                "Could not create appliance directory '{}': {error}",
                self.appliance_dir.display()
            )
        })?;
        let final_path = self.appliance_dir.join(file_name);
        if !final_path.exists() {
            let temporary_path =
                self.appliance_dir
                    .join(format!(".{}.{}.download", Uuid::new_v4(), file_name));
            let bytes = serde_json::to_vec_pretty(&appliance)
                .map_err(|error| format!("Could not encode appliance definition: {error}"))?;
            fs::write(&temporary_path, bytes)
                .map_err(|error| format!("Could not save appliance definition: {error}"))?;
            if let Err(error) = fs::hard_link(&temporary_path, &final_path) {
                let _ = fs::remove_file(&temporary_path);
                if error.kind() != ErrorKind::AlreadyExists {
                    return Err(format!("Could not install appliance definition: {error}"));
                }
            }
            let _ = fs::remove_file(temporary_path);
        }
        Ok(record)
    }

    pub fn import_appliance_file(
        &self,
        file_name: &str,
        bytes: &[u8],
    ) -> Result<ApplianceRecord, String> {
        let file_name = validate_appliance_file_name(file_name)?;
        let appliance: Value = serde_json::from_slice(bytes)
            .map_err(|error| format!("GNS3 appliance file is invalid JSON: {error}"))?;
        let mut record = summarize_appliance(file_name, &appliance, true)?;
        fs::create_dir_all(&self.appliance_dir).map_err(|error| {
            format!(
                "Could not create appliance directory '{}': {error}",
                self.appliance_dir.display()
            )
        })?;
        let final_path = self.appliance_dir.join(file_name);
        if final_path.exists() {
            let existing = fs::read(&final_path).map_err(|error| {
                format!("Could not inspect existing appliance definition: {error}")
            })?;
            let existing: Value = serde_json::from_slice(&existing).map_err(|error| {
                format!("Existing appliance definition is invalid JSON: {error}")
            })?;
            if existing.get("appliance_id").and_then(Value::as_str)
                != Some(record.appliance_id.as_str())
            {
                return Err(format!(
                    "A different appliance already uses the filename '{file_name}'"
                ));
            }
        }
        let temporary_path =
            self.appliance_dir
                .join(format!(".{}.{}.import", Uuid::new_v4(), file_name));
        fs::write(&temporary_path, bytes)
            .map_err(|error| format!("Could not save appliance definition: {error}"))?;
        fs::rename(&temporary_path, &final_path).map_err(|error| {
            let _ = fs::remove_file(&temporary_path);
            format!("Could not install appliance definition: {error}")
        })?;
        record.imported = true;
        Ok(record)
    }

    pub fn create_custom_appliance(
        &self,
        name: &str,
        source_template_id: &str,
        runtime_settings: &Value,
        image_options: &[Value],
    ) -> Result<ApplianceRecord, String> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 100 || name.chars().any(char::is_control) {
            return Err("Custom appliance name must contain 1 to 100 printable characters".into());
        }
        let source = image_options
            .iter()
            .find(|template| template["template_id"].as_str() == Some(source_template_id))
            .ok_or_else(|| "Select an image from the installed runtime inventory".to_owned())?;
        if source.get("appliance_id").is_some() {
            return Err("A custom appliance must be based on an installed runtime image".into());
        }
        let runtime = source["template_type"]
            .as_str()
            .filter(|runtime| matches!(*runtime, "qemu" | "dynamips" | "docker" | "vpcs"))
            .ok_or_else(|| {
                "The selected runtime cannot be used for a custom appliance".to_owned()
            })?;

        let properties = match runtime {
            "qemu" => {
                let disk = source["hda_disk_image"]
                    .as_str()
                    .ok_or_else(|| "The selected QEMU template has no disk image".to_owned())?;
                json!({
                    "hda_disk_image": image_file_name(disk),
                    "platform": source["platform"],
                    "ram": source["memory"].as_u64().unwrap_or(512),
                    "cpus": source["cpus"].as_u64().unwrap_or(1),
                    "adapters": source["adapters"].as_u64().unwrap_or(1),
                    "adapter_type": source["adapter_type"].as_str().unwrap_or("e1000"),
                    "hda_disk_interface": source["hda_disk_interface"].as_str().unwrap_or("ide"),
                    "boot_priority": source["boot_priority"].as_str().unwrap_or("c"),
                    "linked_clone": source["linked_clone"].as_bool().unwrap_or(true),
                    "maxcpus": source["maxcpus"].as_u64().unwrap_or(source["cpus"].as_u64().unwrap_or(1)),
                    "hdb_disk_image": source["hdb_disk_image"].as_str().unwrap_or(""),
                    "hdb_disk_interface": source["hdb_disk_interface"].as_str().unwrap_or("none"),
                    "hdc_disk_image": source["hdc_disk_image"].as_str().unwrap_or(""),
                    "hdc_disk_interface": source["hdc_disk_interface"].as_str().unwrap_or("none"),
                    "hdd_disk_image": source["hdd_disk_image"].as_str().unwrap_or(""),
                    "hdd_disk_interface": source["hdd_disk_interface"].as_str().unwrap_or("none"),
                    "cdrom_image": source["cdrom_image"].as_str().unwrap_or(""),
                    "bios_image": source["bios_image"].as_str().unwrap_or(""),
                    "kernel_image": source["kernel_image"].as_str().unwrap_or(""),
                    "initrd": source["initrd"].as_str().unwrap_or(""),
                    "kernel_command_line": source["kernel_command_line"].as_str().unwrap_or(""),
                    "console_type": source["console_type"].as_str().unwrap_or("telnet"),
                    "aux_type": source["aux_type"].as_str().unwrap_or("none"),
                    "on_close": source["on_close"].as_str().unwrap_or("power_off"),
                    "process_priority": source["process_priority"].as_str().unwrap_or("normal"),
                    "options": source["options"].as_str().unwrap_or(""),
                    "tpm": source["tpm"].as_bool().unwrap_or(false),
                    "uefi": source["uefi"].as_bool().unwrap_or(false),
                    "replicate_network_connection_state": source["replicate_network_connection_state"].as_bool().unwrap_or(true),
                    "create_config_disk": source["create_config_disk"].as_bool().unwrap_or(false),
                    "cpu_throttling": source["cpu_throttling"].as_u64().unwrap_or(0)
                })
            }
            "dynamips" => {
                let image = source["image"]
                    .as_str()
                    .ok_or_else(|| "The selected Dynamips template has no IOS image".to_owned())?;
                let platform = runtime_settings["platform"]
                    .as_str()
                    .or_else(|| source["platform"].as_str())
                    .unwrap_or("c7200");
                let chassis = runtime_settings["chassis"]
                    .as_str()
                    .or_else(|| source["chassis"].as_str())
                    .unwrap_or(match platform {
                        "c3600" => "3640",
                        "c2600" => "2610",
                        "c1700" => "1760",
                        _ => "",
                    });
                let source_slot0 = (source["platform"].as_str() == Some(platform))
                    .then(|| source["slot0"].as_str())
                    .flatten();
                let slot0 = runtime_settings["slot0"]
                    .as_str()
                    .or(source_slot0)
                    .unwrap_or_else(|| default_dynamips_slot0(platform, chassis));
                let mut settings = json!({
                    "image": image_file_name(image),
                    "platform": platform,
                    "ram": source["memory"].as_u64().unwrap_or(256),
                    "nvram": source["nvram"].as_u64().unwrap_or(match platform {
                        "c7200" => 512,
                        "c2691" | "c3725" | "c3745" => 256,
                        _ => 128,
                    }),
                    "slot0": slot0,
                    "idlepc": source["idlepc"].as_str().unwrap_or(""),
                    "adapters": source["adapters"].as_u64().unwrap_or(1),
                    "mmap": runtime_settings["mmap"].as_bool().unwrap_or(source["mmap"].as_bool().unwrap_or(true)),
                    "sparsemem": runtime_settings["sparsemem"].as_bool().unwrap_or(source["sparsemem"].as_bool().unwrap_or(!matches!(platform, "c1700" | "c2600"))),
                    "clock_divisor": runtime_settings["clock_divisor"].as_u64().unwrap_or(source["clock_divisor"].as_u64().unwrap_or(8)),
                    "idlemax": runtime_settings["idlemax"].as_u64().unwrap_or(source["idlemax"].as_u64().unwrap_or(500)),
                    "idlesleep": runtime_settings["idlesleep"].as_u64().unwrap_or(source["idlesleep"].as_u64().unwrap_or(30)),
                    "exec_area": runtime_settings["exec_area"].as_u64().unwrap_or(source["exec_area"].as_u64().unwrap_or(64)),
                    "disk0": runtime_settings["disk0"].as_u64().unwrap_or(source["disk0"].as_u64().unwrap_or(0)),
                    "disk1": runtime_settings["disk1"].as_u64().unwrap_or(source["disk1"].as_u64().unwrap_or(0)),
                    "auto_delete_disks": runtime_settings["auto_delete_disks"].as_bool().unwrap_or(source["auto_delete_disks"].as_bool().unwrap_or(false)),
                    "private_config": runtime_settings["private_config"].as_str().unwrap_or(source["private_config"].as_str().unwrap_or("")),
                    "mac_addr": runtime_settings["mac_addr"].as_str().unwrap_or(source["mac_addr"].as_str().unwrap_or("")),
                    "system_id": runtime_settings["system_id"].as_str().unwrap_or(source["system_id"].as_str().unwrap_or("FTX0945W0MY")),
                    "npe": runtime_settings["npe"].as_str().unwrap_or(source["npe"].as_str().unwrap_or("npe-400")),
                    "midplane": runtime_settings["midplane"].as_str().unwrap_or(source["midplane"].as_str().unwrap_or("vxr")),
                    "iomem": runtime_settings["iomem"].as_u64().unwrap_or(source["iomem"].as_u64().unwrap_or(match platform { "c1700" | "c2600" => 15, _ => 5 })),
                    "ghost_ios": runtime_settings["ghost_ios"].as_bool().unwrap_or(source["ghost_ios"].as_bool().unwrap_or(false)),
                    "console_type": runtime_settings["console_type"].as_str().unwrap_or(source["console_type"].as_str().unwrap_or("telnet")),
                    "aux_type": runtime_settings["aux_type"].as_str().unwrap_or(source["aux_type"].as_str().unwrap_or("none")),
                    "slot1": runtime_settings["slot1"].as_str().or(source["slot1"].as_str()),
                    "slot2": runtime_settings["slot2"].as_str().or(source["slot2"].as_str()),
                    "slot3": runtime_settings["slot3"].as_str().or(source["slot3"].as_str()),
                    "slot4": runtime_settings["slot4"].as_str().or(source["slot4"].as_str()),
                    "slot5": runtime_settings["slot5"].as_str().or(source["slot5"].as_str()),
                    "slot6": runtime_settings["slot6"].as_str().or(source["slot6"].as_str()),
                    "wic0": runtime_settings["wic0"].as_str().or(source["wic0"].as_str()),
                    "wic1": runtime_settings["wic1"].as_str().or(source["wic1"].as_str()),
                    "wic2": runtime_settings["wic2"].as_str().or(source["wic2"].as_str())
                });
                if matches!(platform, "c1700" | "c2600" | "c3600") {
                    settings["chassis"] = json!(chassis);
                }
                if !dynamips_configuration_supported(&settings) {
                    return Err(
                        "Choose a supported Dynamips platform and chassis configuration".into(),
                    );
                }
                settings
            }
            "docker" => {
                let image = source["image"].as_str().ok_or_else(|| {
                    "The selected Docker template has no image reference".to_owned()
                })?;
                json!({
                    "image": image,
                    "ram": source["memory"].as_u64().unwrap_or(0),
                    "cpus": source["cpus"].as_f64().unwrap_or(0.0),
                    "adapters": source["adapters"].as_u64().unwrap_or(1),
                    "start_command": source["start_command"].as_str().unwrap_or(""),
                    "environment": source["environment"].as_str().unwrap_or(""),
                    "extra_hosts": source["extra_hosts"].as_str().unwrap_or(""),
                    "extra_volumes": source["extra_volumes"].as_array().cloned().unwrap_or_default(),
                    "console_type": source["console_type"].as_str().unwrap_or("telnet"),
                    "console_resolution": source["console_resolution"].as_str().unwrap_or("1024x768"),
                    "console_http_port": source["console_http_port"].as_u64().unwrap_or(80),
                    "console_http_path": source["console_http_path"].as_str().unwrap_or("/"),
                    "aux_type": source["aux_type"].as_str().unwrap_or("none"),
                    "custom_adapters": source["custom_adapters"].as_array().cloned().unwrap_or_default()
                })
            }
            "vpcs" => json!({
                "startup_script": source["startup_script"].as_str().unwrap_or(""),
                "base_script_file": source["base_script_file"].as_str().unwrap_or("vpcs_base_config.txt"),
                "console_type": source["console_type"].as_str().unwrap_or("telnet"),
                "console_auto_start": source["console_auto_start"].as_bool().unwrap_or(false)
            }),
            _ => unreachable!(),
        };

        let mut required_images = Vec::new();
        let mut version_images = serde_json::Map::new();
        let image_slot = match runtime {
            "qemu" => Some("hda_disk_image"),
            "dynamips" => Some("image"),
            _ => None,
        };
        if let Some(image_slot) = image_slot {
            let filename = properties[image_slot]
                .as_str()
                .ok_or_else(|| "The selected image has no filename".to_owned())?;
            let path_key = if runtime == "qemu" {
                "hda_disk_image"
            } else {
                "image"
            };
            let image_path = source[path_key]
                .as_str()
                .ok_or_else(|| "The selected image path is unavailable".to_owned())?;
            let filesize = fs::metadata(image_path)
                .map_err(|error| format!("Could not inspect selected image: {error}"))?
                .len();
            required_images.push(json!({ "filename": filename, "filesize": filesize }));
            version_images.insert(image_slot.to_owned(), json!(filename));
        }
        let appliance_id = Uuid::new_v4();
        let file_name = format!("custom-{appliance_id}.gns3a");
        let mut definition = json!({
            "appliance_id": appliance_id.to_string(),
            "name": name,
            "description": "Custom appliance created in the appliance library.",
            "category": match runtime { "qemu" => "guest", "docker" | "vpcs" => "guest", _ => "router" },
            "registry_version": 8,
            "network_simulator": { "origin": "custom" },
            "settings": [{ "name": "default", "default": true, "template_type": runtime, "template_properties": properties }],
            "versions": [{ "name": "Default", "settings": "default", "images": version_images }],
            "images": required_images
        });
        if runtime == "vpcs" {
            definition["versions"][0]
                .as_object_mut()
                .unwrap()
                .remove("images");
        }
        let bytes = serde_json::to_vec_pretty(&definition)
            .map_err(|error| format!("Could not encode custom appliance: {error}"))?;
        self.import_appliance_file(&file_name, &bytes)
    }

    pub fn imported_appliances(&self) -> Result<Vec<ApplianceRecord>, String> {
        let entries = match fs::read_dir(&self.appliance_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(format!("Could not list imported appliances: {error}")),
        };
        let mut records = Vec::new();
        for entry in entries {
            let entry =
                entry.map_err(|error| format!("Could not read appliance entry: {error}"))?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("gns3a") {
                continue;
            }
            let content = fs::read(&path)
                .map_err(|error| format!("Could not read imported appliance: {error}"))?;
            let appliance: Value = serde_json::from_slice(&content)
                .map_err(|error| format!("Imported appliance JSON is invalid: {error}"))?;
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            records.push(summarize_appliance(file_name, &appliance, true)?);
        }
        records.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
        Ok(records)
    }

    pub async fn appliance_manager(
        &self,
        image_options: &[Value],
    ) -> Result<ApplianceManagerState, String> {
        let mut local_images = local_appliance_images(image_options)?;
        let mut appliances = self.imported_appliances()?;
        for appliance in &mut appliances {
            let bindings = self.read_image_bindings(&appliance.appliance_id)?;
            for version in &mut appliance.versions {
                if version.template_type.as_deref() == Some("docker")
                    && let Some(image_ref) = version.docker_image.as_deref()
                    && let Some(template) = image_options.iter().find(|template| {
                        template["template_type"] == "docker"
                            && template["image"].as_str().is_some_and(|candidate| {
                                candidate == image_ref
                                    || (!image_ref.contains(':')
                                        && candidate.starts_with(&format!("{image_ref}:")))
                            })
                    })
                {
                    version.docker_image_available = true;
                    version.docker_image_id = template["image_digest"].as_str().map(str::to_owned);
                }
                for requirement in &mut version.images {
                    if !requirement.assignable {
                        requirement.status = "runtime-slot-unsupported".to_owned();
                        continue;
                    }
                    let binding = bindings.iter().find(|binding| {
                        binding.version == version.name && binding.image_key == requirement.key
                    });
                    let preferred = binding.and_then(|binding| {
                        local_images.iter().position(|image| {
                            image.image_id == binding.image_id && image.path == binding.image_path
                        })
                    });
                    let expected = requirement
                        .md5sum
                        .as_deref()
                        .map(|value| value.to_ascii_lowercase());
                    let mut candidate_indices = Vec::new();
                    if let Some(index) = preferred {
                        candidate_indices.push(index);
                    } else {
                        candidate_indices.extend(local_images.iter().enumerate().filter_map(
                            |(index, image)| {
                                (image.template_type
                                    == version.template_type.as_deref().unwrap_or_default()
                                    && image.name.eq_ignore_ascii_case(&requirement.filename))
                                .then_some(index)
                            },
                        ));
                        if expected.is_some() {
                            let already_checked = candidate_indices.clone();
                            candidate_indices.extend(local_images.iter().enumerate().filter_map(
                                |(index, image)| {
                                    (image.template_type
                                        == version.template_type.as_deref().unwrap_or_default()
                                        && requirement
                                            .filesize
                                            .is_none_or(|size| size == image.size)
                                        && !already_checked.contains(&index))
                                    .then_some(index)
                                },
                            ));
                        }
                    }
                    let mut selected = None;
                    let mut first_candidate = None;
                    for index in candidate_indices {
                        let checksum = self
                            .image_checksum(Path::new(&local_images[index].path))
                            .await?;
                        local_images[index].size = checksum.size;
                        local_images[index].md5sum = Some(checksum.md5sum.clone());
                        local_images[index].sha1sum = Some(checksum.sha1sum.clone());
                        if first_candidate.is_none() {
                            first_candidate = Some(index);
                        }
                        let matches = requirement
                            .filesize
                            .is_none_or(|size| size == checksum.size)
                            && expected.as_deref().is_none_or(|md5| md5 == checksum.md5sum);
                        if matches {
                            selected = Some(index);
                            break;
                        }
                    }
                    let current = preferred
                        .filter(|_| binding.is_some())
                        .and_then(|index| Some((index, binding?)));
                    let chosen = current.map(|(index, _)| index).or(selected);
                    let observed = chosen.or(first_candidate);
                    if let Some(index) = observed {
                        requirement.suggested_image_id = Some(local_images[index].image_id.clone());
                        requirement.suggested_image_name = Some(local_images[index].name.clone());
                        requirement.actual_size = Some(local_images[index].size);
                        requirement.actual_md5sum = local_images[index].md5sum.clone();
                        requirement.actual_sha1sum = local_images[index].sha1sum.clone();
                    }
                    if let Some((index, binding)) = current {
                        let image = &local_images[index];
                        requirement.assigned_image_id = Some(image.image_id.clone());
                        requirement.assigned_image_name = Some(image.name.clone());
                        requirement.image_path = Some(image.path.clone());
                        requirement.allow_checksum_mismatch = binding.allow_checksum_mismatch;
                        let hash_matches = requirement
                            .md5sum
                            .as_deref()
                            .is_none_or(|md5| image.md5sum.as_deref() == Some(md5));
                        let size_matches =
                            requirement.filesize.is_none_or(|size| image.size == size);
                        requirement.status = if hash_matches && size_matches {
                            verification_status(requirement).to_owned()
                        } else if binding.allow_checksum_mismatch {
                            "override".to_owned()
                        } else {
                            "mismatch".to_owned()
                        };
                    } else if let Some(index) = selected {
                        let image = &local_images[index];
                        requirement.suggested_image_id = Some(image.image_id.clone());
                        requirement.suggested_image_name = Some(image.name.clone());
                        requirement.actual_size = Some(image.size);
                        requirement.actual_md5sum = image.md5sum.clone();
                        requirement.actual_sha1sum = image.sha1sum.clone();
                        let status = verification_status(requirement);
                        let binding = PersistedImageBinding {
                            version: version.name.clone(),
                            image_key: requirement.key.clone(),
                            image_id: image.image_id.clone(),
                            image_path: image.path.clone(),
                            allow_checksum_mismatch: false,
                        };
                        self.write_image_binding(&appliance.appliance_id, binding)
                            .await?;
                        requirement.assigned_image_id = Some(image.image_id.clone());
                        requirement.assigned_image_name = Some(image.name.clone());
                        requirement.image_path = Some(image.path.clone());
                        requirement.status = status.to_owned();
                    } else if first_candidate.is_some() {
                        requirement.status = "mismatch".to_owned();
                    } else if binding.is_some() {
                        requirement.status = "assigned-image-missing".to_owned();
                        requirement.assigned_image_name = binding
                            .map(|binding| Path::new(&binding.image_path).file_name())
                            .flatten()
                            .and_then(|name| name.to_str())
                            .map(str::to_owned);
                        requirement.image_path = binding.map(|binding| binding.image_path.clone());
                    } else {
                        requirement.status = "missing".to_owned();
                    }
                }
            }
        }
        Ok(ApplianceManagerState {
            appliances,
            local_images,
            custom_image_options: image_options.to_vec(),
        })
    }
}
