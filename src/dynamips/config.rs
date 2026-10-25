use super::*;

fn resolve_working_dir(path: &Path) -> std::io::Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    fs::create_dir_all(&absolute)?;
    fs::canonicalize(absolute)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct DynamipsOptions {
    pub dynamips_id: u32,
    pub clock_divisor: u32,
    pub mmap: bool,
    pub sparsemem: bool,
    pub idlemax: u32,
    pub idlesleep: u32,
    pub exec_area: u32,
    pub disk0: u32,
    pub disk1: u32,
    pub auto_delete_disks: bool,
    #[serde(alias = "private_config_content")]
    pub private_config: String,
    pub mac_addr: Option<String>,
    pub system_id: Option<String>,
    pub slots: Vec<Option<String>>,
    pub wics: Vec<Option<String>>,
    pub npe: Option<String>,
    pub midplane: Option<String>,
    pub iomem: Option<u8>,
    pub ghost_ios: bool,
    pub console_type: String,
    pub aux_type: String,
}

impl Default for DynamipsOptions {
    fn default() -> Self {
        Self {
            dynamips_id: 1,
            clock_divisor: 8,
            mmap: true,
            sparsemem: true,
            idlemax: 500,
            idlesleep: 30,
            exec_area: 64,
            disk0: 0,
            disk1: 0,
            auto_delete_disks: false,
            private_config: String::new(),
            mac_addr: None,
            system_id: Some("FTX0945W0MY".to_owned()),
            slots: vec![None; 7],
            wics: vec![None; 3],
            npe: Some("npe-400".to_owned()),
            midplane: Some("vxr".to_owned()),
            iomem: None,
            ghost_ios: false,
            console_type: "telnet".to_owned(),
            aux_type: "none".to_owned(),
        }
    }
}

impl DynamipsRuntime {
    pub async fn templates(&self) -> Result<Vec<Value>, RuntimeError> {
        self.version().await?;
        let mut templates = Vec::new();
        for image in self.image_files()? {
            let name = image
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("Cisco IOS image");
            let template_id =
                Uuid::new_v5(&Uuid::NAMESPACE_URL, image.to_string_lossy().as_bytes());
            templates.push(json!({
                "template_id": template_id,
                "name": format!("{name} · c7200"),
                "template_type": "dynamips",
                "compute_id": "local",
                "category": "router",
                "symbol": "router",
                "builtin": false,
                "platform": "c7200",
                "image": image,
                "adapters": 1,
                "memory": 256,
                "nvram": 128,
                "idlepc": "",
                "mmap": true,
                "sparsemem": true,
                "clock_divisor": 8,
                "idlemax": 500,
                "idlesleep": 30,
                "exec_area": 64,
                "disk0": 0,
                "disk1": 0,
                "auto_delete_disks": false,
                "private_config": "",
                "mac_addr": "",
                "system_id": "FTX0945W0MY",
                "slot0": "C7200-IO-FE",
                "slot1": null,
                "slot2": null,
                "slot3": null,
                "slot4": null,
                "slot5": null,
                "slot6": null,
                "wic0": null,
                "wic1": null,
                "wic2": null,
                "npe": "npe-400",
                "midplane": "vxr",
                "iomem": null,
                "ghost_ios": false,
                "console_type": "telnet",
                "aux_type": "none"
            }));
        }
        templates.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
        Ok(templates)
    }

    pub fn upload_paths(&self, file_name: &str) -> Result<(PathBuf, PathBuf), RuntimeError> {
        let safe_name = safe_file_name(file_name)?;
        fs::create_dir_all(&self.image_dir).map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not create Dynamips image directory '{}': {error}",
                self.image_dir.display()
            ))
        })?;
        let directory = self.image_dir.canonicalize().map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not resolve Dynamips image directory: {error}"
            ))
        })?;
        let final_path = directory.join(&safe_name);
        if final_path.exists() {
            return Err(RuntimeError::Failed(format!(
                "A Dynamips image named '{}' already exists",
                final_path.file_name().unwrap_or_default().to_string_lossy()
            )));
        }
        let temporary_path = directory.join(format!(".{}.{}.upload", Uuid::new_v4(), safe_name));
        Ok((temporary_path, final_path))
    }

    pub fn finalize_uploaded_image(
        &self,
        temporary_path: &Path,
        final_path: &Path,
    ) -> Result<PathBuf, RuntimeError> {
        validate_ios_image(temporary_path)?;
        fs::hard_link(temporary_path, final_path).map_err(|error| {
            RuntimeError::Failed(format!("Could not install Cisco IOS image: {error}"))
        })?;
        fs::remove_file(temporary_path).map_err(|error| {
            let _ = fs::remove_file(final_path);
            RuntimeError::Failed(format!(
                "Could not finalize Cisco IOS image upload: {error}"
            ))
        })?;
        final_path.canonicalize().map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not resolve installed Cisco IOS image: {error}"
            ))
        })
    }

    pub async fn create_router(
        &self,
        project_id: &str,
        node_id: &str,
        name: &str,
        image: &str,
        platform: &str,
        chassis: Option<&str>,
        slot0: Option<&str>,
        ram_mb: u64,
        nvram: u64,
        idle_pc: Option<&str>,
        startup_config: &str,
    ) -> Result<DynamipsVm, RuntimeError> {
        self.create_router_with_options(
            project_id,
            node_id,
            name,
            image,
            platform,
            chassis,
            slot0,
            ram_mb,
            nvram,
            idle_pc,
            startup_config,
            &DynamipsOptions::default(),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_router_with_options(
        &self,
        project_id: &str,
        node_id: &str,
        name: &str,
        image: &str,
        platform: &str,
        chassis: Option<&str>,
        slot0: Option<&str>,
        ram_mb: u64,
        nvram: u64,
        idle_pc: Option<&str>,
        startup_config: &str,
        options: &DynamipsOptions,
    ) -> Result<DynamipsVm, RuntimeError> {
        let _guard = self.operation_lock.lock().await;
        let project_id = parse_id(project_id)?;
        let node_id = parse_id(node_id)?;
        validate_router_name(name)?;
        validate_platform_settings(platform, chassis, slot0)?;
        validate_dynamips_options(platform, chassis, slot0, options)?;
        if !(128..=2048).contains(&ram_mb) {
            return Err(RuntimeError::Failed(
                "Dynamips router RAM must be between 128 and 2048 MB".to_owned(),
            ));
        }
        if !(64..=2048).contains(&nvram) {
            return Err(RuntimeError::Failed(
                "Dynamips router NVRAM must be between 64 and 2048 KB".to_owned(),
            ));
        }
        let idle_pc = idle_pc
            .filter(|value| !value.trim().is_empty())
            .map(|value| {
                if is_hex_pointer(value) {
                    Ok(value.to_owned())
                } else {
                    Err(RuntimeError::Failed(
                        "Dynamips idle-PC must be a hexadecimal address".to_owned(),
                    ))
                }
            })
            .transpose()?;
        let image = self.resolve_image(image)?;
        let working_dir = self
            .state_dir
            .join(project_id.to_string())
            .join(node_id.to_string());
        let working_dir = resolve_working_dir(&working_dir).map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not create Dynamips router directory: {error}"
            ))
        })?;
        fs::create_dir_all(working_dir.join("configs")).map_err(|error| {
            RuntimeError::Failed(format!("Could not create Dynamips configs directory: {error}"))
        })?;
        fs::write(
            working_dir.join(format!(
                "configs/i{}_startup-config.cfg",
                options.dynamips_id
            )),
            startup_config,
        )
        .map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not write Dynamips startup configuration: {error}"
            ))
        })?;
        if !options.private_config.is_empty() {
            fs::write(
                working_dir.join(format!(
                    "configs/i{}_private-config.cfg",
                    options.dynamips_id
                )),
                &options.private_config,
            )
            .map_err(|error| {
                RuntimeError::Failed(format!(
                    "Could not write Dynamips private configuration: {error}"
                ))
            })?;
        }

        let console_port = available_tcp_port().await?;
        let aux_port = if options.aux_type != "none" {
            Some(available_tcp_port().await?)
        } else {
            None
        };
        let ghost_file = if options.ghost_ios && options.mmap {
            let file_name = image
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("ios");
            Some(
                self.state_dir
                    .join("ghosts")
                    .join(format!("{file_name}-{}.ghost", ram_mb)),
            )
        } else {
            None
        };
        let vm = DynamipsVm {
            node_id: node_id.to_string(),
            name: name.to_owned(),
            platform: platform.to_owned(),
            chassis: chassis.map(str::to_owned),
            slot0: options
                .slots
                .first()
                .and_then(Option::as_deref)
                .or(slot0)
                .unwrap_or_else(|| default_slot0(platform, chassis).expect("validated platform"))
                .to_owned(),
            image,
            ram_mb,
            nvram,
            idle_pc,
            startup_config: startup_config.to_owned(),
            log_file: working_dir.join("dynamips_stdout.log"),
            working_dir,
            hypervisor_port: available_tcp_port().await?,
            console_port,
            aux_port,
            options: options.clone(),
            ghost_file,
        };
        self.write_runtime_manifest(&vm, &project_id, &node_id)?;
        if let Err(error) = self.initialize_router(&vm).await {
            let _ = fs::remove_file(vm.working_dir.join(RUNTIME_MANIFEST_FILE));
            return Err(error);
        }
        Ok(vm)
    }

    pub fn vm_from_node(&self, node: &Value) -> Result<DynamipsVm, RuntimeError> {
        let project_id = parse_id(node["project_id"].as_str().unwrap_or_default())?;
        let node_id = parse_id(node["node_id"].as_str().unwrap_or_default())?;
        let working_dir = self
            .state_dir
            .join(project_id.to_string())
            .join(node_id.to_string());
        let supplied_working_dir = PathBuf::from(required_string(node, "runtime_working_dir")?);
        if supplied_working_dir != working_dir {
            return Err(RuntimeError::Failed(
                "Dynamips runtime_working_dir must stay inside the controller runtime directory"
                    .to_owned(),
            ));
        }
        let expected_log_file = working_dir.join("dynamips_stdout.log");
        let supplied_log_file = PathBuf::from(required_string(node, "runtime_log_file")?);
        if supplied_log_file != expected_log_file {
            return Err(RuntimeError::Failed(
                "Dynamips runtime_log_file must stay inside the controller runtime directory"
                    .to_owned(),
            ));
        }
        let platform = required_string(node, "platform")?.to_owned();
        let chassis = node["chassis"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| (platform == "c3600").then(|| "3620".to_owned()));
        let slot0 = node["slot0"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| {
                default_slot0(&platform, chassis.as_deref())
                    .unwrap_or("C7200-IO-FE")
                    .to_owned()
            });
        validate_platform_settings(&platform, chassis.as_deref(), Some(&slot0))?;
        let name = required_string(node, "name")?;
        validate_router_name(name)?;
        let mut options: DynamipsOptions =
            serde_json::from_value(node.clone()).map_err(|error| {
                RuntimeError::Failed(format!("Dynamips node settings are invalid: {error}"))
            })?;
        options.slots = (0..7)
            .map(|index| {
                node.get(format!("slot{index}"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect();
        options.wics = (0..3)
            .map(|index| {
                node.get(format!("wic{index}"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect();
        validate_dynamips_options(&platform, chassis.as_deref(), Some(&slot0), &options)?;
        let image = self.resolve_image(required_string(node, "image")?)?;
        let idle_pc = node["idlepc"]
            .as_str()
            .filter(|value| !value.trim().is_empty())
            .map(|value| {
                if is_hex_pointer(value) {
                    Ok(value.to_owned())
                } else {
                    Err(RuntimeError::Failed(
                        "Dynamips idle-PC must be a hexadecimal address".to_owned(),
                    ))
                }
            })
            .transpose()?;
        let hypervisor_port = node["runtime_hypervisor_port"]
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .filter(|port| *port != 0)
            .ok_or_else(|| {
                RuntimeError::Failed("Dynamips node has no hypervisor port".to_owned())
            })?;
        let ram_mb = node["ram"].as_u64().unwrap_or(256);
        if !(128..=2048).contains(&ram_mb) {
            return Err(RuntimeError::Failed(
                "Dynamips router RAM must be between 128 and 2048 MB".to_owned(),
            ));
        }
        let nvram = node["nvram"].as_u64().unwrap_or(128);
        if !(64..=2048).contains(&nvram) {
            return Err(RuntimeError::Failed(
                "Dynamips router NVRAM must be between 64 and 2048 KB".to_owned(),
            ));
        }
        let console_port = node["console"]
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .unwrap_or(0);
        let aux_port = node["aux"]
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .filter(|port| *port != 0);
        self.validate_runtime_manifest(
            &working_dir,
            &project_id,
            &node_id,
            hypervisor_port,
            console_port,
            aux_port,
        )?;
        let ghost_file = if options.ghost_ios && options.mmap {
            let file_name = image
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("ios");
            let expected = self
                .state_dir
                .join("ghosts")
                .join(format!("{file_name}-{ram_mb}.ghost"));
            if let Some(supplied) = node["runtime_ghost_file"].as_str()
                && Path::new(supplied) != expected
            {
                return Err(RuntimeError::Failed(
                    "Dynamips runtime_ghost_file must stay inside the controller runtime directory"
                        .to_owned(),
                ));
            }
            Some(expected)
        } else {
            if node
                .get("runtime_ghost_file")
                .is_some_and(|value| !value.is_null())
            {
                return Err(RuntimeError::Failed(
                    "Dynamips runtime_ghost_file is not valid when Ghost IOS is disabled"
                        .to_owned(),
                ));
            }
            None
        };
        Ok(DynamipsVm {
            node_id: node_id.to_string(),
            name: name.to_owned(),
            platform,
            chassis,
            slot0,
            image,
            ram_mb,
            nvram,
            idle_pc,
            startup_config: node["startup_config"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            working_dir,
            hypervisor_port,
            console_port,
            aux_port,
            log_file: expected_log_file,
            options,
            ghost_file,
        })
    }

    pub(super) fn write_runtime_manifest(
        &self,
        vm: &DynamipsVm,
        project_id: &Uuid,
        node_id: &Uuid,
    ) -> Result<(), RuntimeError> {
        let contents = runtime_manifest_contents(
            project_id,
            node_id,
            vm.hypervisor_port,
            vm.console_port,
            vm.aux_port,
        );
        fs::write(vm.working_dir.join(RUNTIME_MANIFEST_FILE), contents).map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not record Dynamips runtime ownership: {error}"
            ))
        })
    }

    pub(super) fn validate_runtime_manifest(
        &self,
        working_dir: &Path,
        project_id: &Uuid,
        node_id: &Uuid,
        hypervisor_port: u16,
        console_port: u16,
        aux_port: Option<u16>,
    ) -> Result<(), RuntimeError> {
        let path = working_dir.join(RUNTIME_MANIFEST_FILE);
        let metadata = fs::symlink_metadata(&path).map_err(|_| {
            RuntimeError::Failed(
                "Dynamips runtime state is not owned by this controller".to_owned(),
            )
        })?;
        if !metadata.file_type().is_file() {
            return Err(RuntimeError::Failed(
                "Dynamips runtime ownership marker is invalid".to_owned(),
            ));
        }
        let stored = fs::read_to_string(path).map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not read Dynamips runtime ownership: {error}"
            ))
        })?;
        let expected =
            runtime_manifest_contents(project_id, node_id, hypervisor_port, console_port, aux_port);
        if stored != expected {
            return Err(RuntimeError::Failed(
                "Dynamips runtime fields do not match controller-owned runtime state".to_owned(),
            ));
        }
        Ok(())
    }

    pub fn runtime_fields(&self, vm: &DynamipsVm) -> Value {
        json!({
            "platform": vm.platform,
            "chassis": vm.chassis,
            "slot0": vm.slot0,
            "image": vm.image,
            "ram": vm.ram_mb,
            "nvram": vm.nvram,
            "idlepc": vm.idle_pc,
            "console": vm.console_port,
            "runtime_hypervisor_port": vm.hypervisor_port,
            "runtime_working_dir": vm.working_dir,
            "runtime_log_file": vm.log_file,
            "adapter_types": [vm.slot0],
            "dynamips_id": vm.options.dynamips_id,
            "clock_divisor": vm.options.clock_divisor,
            "mmap": vm.options.mmap,
            "sparsemem": vm.options.sparsemem,
            "idlemax": vm.options.idlemax,
            "idlesleep": vm.options.idlesleep,
            "exec_area": vm.options.exec_area,
            "disk0": vm.options.disk0,
            "disk1": vm.options.disk1,
            "auto_delete_disks": vm.options.auto_delete_disks,
            "private_config": vm.options.private_config,
            "mac_addr": vm.options.mac_addr,
            "system_id": vm.options.system_id,
            "slot0": vm.options.slots.first().cloned().flatten().unwrap_or_else(|| vm.slot0.clone()),
            "slot1": vm.options.slots.get(1).cloned().flatten(),
            "slot2": vm.options.slots.get(2).cloned().flatten(),
            "slot3": vm.options.slots.get(3).cloned().flatten(),
            "slot4": vm.options.slots.get(4).cloned().flatten(),
            "slot5": vm.options.slots.get(5).cloned().flatten(),
            "slot6": vm.options.slots.get(6).cloned().flatten(),
            "wic0": vm.options.wics.first().cloned().flatten(),
            "wic1": vm.options.wics.get(1).cloned().flatten(),
            "wic2": vm.options.wics.get(2).cloned().flatten(),
            "npe": vm.options.npe,
            "midplane": vm.options.midplane,
            "iomem": vm.options.iomem.or_else(|| default_iomem(&vm.platform)),
            "ghost_ios": vm.options.ghost_ios,
            "console_type": vm.options.console_type,
            "aux_type": vm.options.aux_type,
            "aux": vm.aux_port,
            "runtime_ghost_file": vm.ghost_file
        })
    }
}
