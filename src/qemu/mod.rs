use std::{
    fs,
    io::ErrorKind,
    net::Ipv4Addr,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use md5::{Digest, Md5};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    process::{Child, Command},
    sync::Mutex,
    time::{sleep, timeout},
};
use uuid::Uuid;

use crate::runtime::{NodeAction, RuntimeError};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);
const QMP_TIMEOUT: Duration = Duration::from_secs(5);
const START_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone)]
pub struct QemuRuntime {
    binary: PathBuf,
    qemu_img: PathBuf,
    image_dir: PathBuf,
    state_dir: PathBuf,
    qmp_dir: PathBuf,
    tpm_processes: std::sync::Arc<Mutex<std::collections::HashMap<String, Child>>>,
    cpu_limit_processes: std::sync::Arc<Mutex<std::collections::HashMap<String, Child>>>,
}

#[derive(Clone, Debug)]
pub struct QemuVm {
    pub node_id: String,
    pub image: PathBuf,
    pub platform: String,
    pub overlay: PathBuf,
    pub qmp_socket: PathBuf,
    pub serial_socket: PathBuf,
    pub pid_file: PathBuf,
    pub log_file: PathBuf,
    pub memory_mb: u64,
    pub cpus: u32,
    pub maxcpus: u32,
    pub adapters: u32,
    pub adapter_type: String,
    pub disk_interface: String,
    pub boot_priority: String,
    pub disks: Vec<QemuDisk>,
    pub cdrom_image: Option<PathBuf>,
    pub bios_image: Option<PathBuf>,
    pub kernel_image: Option<PathBuf>,
    pub initrd: Option<PathBuf>,
    pub kernel_command_line: String,
    pub linked_clone: bool,
    pub console_type: String,
    pub console_port: Option<u16>,
    pub aux_type: String,
    pub aux_port: Option<u16>,
    pub on_close: String,
    pub process_priority: String,
    pub extra_options: String,
    pub tpm: bool,
    pub uefi: bool,
    pub replicate_network_connection_state: bool,
    pub create_config_disk: bool,
    pub cpu_throttling: u32,
}

#[derive(Clone, Debug)]
pub struct QemuDisk {
    pub slot: String,
    pub image: PathBuf,
    pub active_file: PathBuf,
    pub interface: String,
    pub format: String,
}

#[derive(Clone, Debug, Default)]
pub struct QemuTemplateOptions {
    pub platform: Option<String>,
    pub adapter_type: Option<String>,
    pub hda_disk_interface: Option<String>,
    pub boot_priority: Option<String>,
    pub linked_clone: Option<bool>,
    pub maxcpus: Option<u32>,
    pub hdb_disk_image: Option<String>,
    pub hdb_disk_interface: Option<String>,
    pub hdc_disk_image: Option<String>,
    pub hdc_disk_interface: Option<String>,
    pub hdd_disk_image: Option<String>,
    pub hdd_disk_interface: Option<String>,
    pub hda_disk_image_backing_file: Option<String>,
    pub hdb_disk_image_backing_file: Option<String>,
    pub hdc_disk_image_backing_file: Option<String>,
    pub hdd_disk_image_backing_file: Option<String>,
    pub disk_images_to_reset: Vec<String>,
    pub cdrom_image: Option<String>,
    pub bios_image: Option<String>,
    pub kernel_image: Option<String>,
    pub initrd: Option<String>,
    pub kernel_command_line: Option<String>,
    pub console_type: Option<String>,
    pub aux_type: Option<String>,
    pub on_close: Option<String>,
    pub process_priority: Option<String>,
    pub options: Option<String>,
    pub tpm: Option<bool>,
    pub uefi: Option<bool>,
    pub replicate_network_connection_state: Option<bool>,
    pub create_config_disk: Option<bool>,
    pub cpu_throttling: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct QemuLink {
    pub link_id: String,
    pub adapter_number: u32,
    pub adapter_port: u16,
    pub runtime_port: u16,
}

#[derive(Serialize)]
pub struct QemuRuntimeInfo {
    pub available: bool,
    pub version: Option<String>,
    pub qemu_img_available: bool,
    pub image_directory: String,
    pub image_count: usize,
    pub acceleration: String,
    pub kvm_accessible: bool,
    pub acceleration_error: Option<String>,
    pub error: Option<String>,
}

impl QemuRuntime {
    pub fn new() -> Self {
        let state_dir = std::env::var_os("NETWORK_SIMULATOR_QEMU_STATE")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("data/qemu"));
        Self {
            binary: std::env::var_os("NETWORK_SIMULATOR_QEMU")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("qemu-system-x86_64")),
            qemu_img: std::env::var_os("NETWORK_SIMULATOR_QEMU_IMG")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("qemu-img")),
            image_dir: crate::image_paths::qemu_images(),
            qmp_dir: std::env::var_os("NETWORK_SIMULATOR_QEMU_SOCKETS")
                .map(PathBuf::from)
                .unwrap_or_else(|| std::env::temp_dir().join("network-simulator-qemu")),
            state_dir,
            tpm_processes: std::sync::Arc::new(Mutex::new(std::collections::HashMap::new())),
            cpu_limit_processes: std::sync::Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }

    #[cfg(test)]
    pub fn with_paths(
        binary: impl Into<PathBuf>,
        qemu_img: impl Into<PathBuf>,
        image_dir: impl Into<PathBuf>,
        state_dir: impl Into<PathBuf>,
        qmp_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            binary: binary.into(),
            qemu_img: qemu_img.into(),
            image_dir: image_dir.into(),
            state_dir: state_dir.into(),
            qmp_dir: qmp_dir.into(),
            tpm_processes: std::sync::Arc::new(Mutex::new(std::collections::HashMap::new())),
            cpu_limit_processes: std::sync::Arc::new(Mutex::new(std::collections::HashMap::new())),
        }
    }

    pub async fn info(&self) -> QemuRuntimeInfo {
        let images = self.image_files().unwrap_or_default();
        let image_directory = self
            .image_dir
            .canonicalize()
            .unwrap_or_else(|_| self.image_dir.clone())
            .display()
            .to_string();
        let qemu_img_available = self
            .command(&self.qemu_img, ["--version"], COMMAND_TIMEOUT)
            .await
            .is_ok();
        let version = match self
            .command(&self.binary, ["-version"], COMMAND_TIMEOUT)
            .await
        {
            Ok(output) => output.lines().next().map(str::to_owned),
            Err(_) => None,
        };
        let error = if version.is_none() {
            Some(format!(
                "QEMU binary '{}' is unavailable",
                self.binary.display()
            ))
        } else if !qemu_img_available {
            Some(format!(
                "qemu-img binary '{}' is unavailable",
                self.qemu_img.display()
            ))
        } else {
            None
        };
        let acceleration_error = kvm_access_error();
        let kvm_accessible = acceleration_error.is_none();
        QemuRuntimeInfo {
            available: error.is_none(),
            version,
            qemu_img_available,
            image_directory,
            image_count: images.len(),
            acceleration: if kvm_accessible { "KVM" } else { "TCG" }.to_owned(),
            kvm_accessible,
            acceleration_error,
            error,
        }
    }

    pub async fn templates(&self) -> Result<Vec<Value>, RuntimeError> {
        self.command(&self.binary, ["-version"], COMMAND_TIMEOUT)
            .await?;
        self.command(&self.qemu_img, ["--version"], COMMAND_TIMEOUT)
            .await?;
        let platform = self.platform();
        let mut templates = Vec::new();
        for path in self.image_files()? {
            let canonical = path.canonicalize().map_err(|error| {
                RuntimeError::Failed(format!("Could not resolve QEMU image path: {error}"))
            })?;
            let template_id =
                Uuid::new_v5(&Uuid::NAMESPACE_URL, canonical.to_string_lossy().as_bytes());
            let name = path
                .file_name()
                .and_then(|file| file.to_str())
                .unwrap_or("QEMU disk image");
            templates.push(json!({
                "template_id": template_id,
                "name": name,
                "template_type": "qemu",
                "compute_id": "local",
                "category": "guest",
                "symbol": "qemu_guest",
                "builtin": false,
                "hda_disk_image": canonical,
                "adapters": 1,
                "memory": 512,
                "cpus": 1,
                "platform": platform,
                "adapter_type": "e1000",
                "hda_disk_interface": "ide",
                "boot_priority": "c",
                "linked_clone": true,
                "maxcpus": 1,
                "hdb_disk_image": "",
                "hdb_disk_interface": "none",
                "hdc_disk_image": "",
                "hdc_disk_interface": "none",
                "hdd_disk_image": "",
                "hdd_disk_interface": "none",
                "cdrom_image": "",
                "bios_image": "",
                "kernel_image": "",
                "initrd": "",
                "kernel_command_line": "",
                "console_type": "telnet",
                "aux_type": "none",
                "on_close": "power_off",
                "process_priority": "normal",
                "options": "",
                "tpm": false,
                "uefi": false,
                "replicate_network_connection_state": true,
                "create_config_disk": false,
                "cpu_throttling": 0
            }));
        }
        templates.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
        Ok(templates)
    }

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

    pub async fn lifecycle(
        &self,
        vm: &QemuVm,
        action: NodeAction,
        links: &[QemuLink],
    ) -> Result<String, RuntimeError> {
        match action {
            NodeAction::Start => self.start(vm, links).await,
            NodeAction::Stop => self.stop(vm).await,
            NodeAction::Suspend => {
                self.qmp_execute(&vm.qmp_socket, "stop", None).await?;
                self.status(vm).await
            }
            NodeAction::Reload => {
                self.qmp_execute(&vm.qmp_socket, "system_reset", None)
                    .await?;
                self.status(vm).await
            }
        }
    }

    pub async fn attach_nio(&self, vm: &QemuVm, link: &QemuLink) -> Result<(), RuntimeError> {
        if link.adapter_number >= vm.adapters {
            return Err(RuntimeError::Failed(format!(
                "Invalid QEMU adapter {} for hot-added link",
                link.adapter_number
            )));
        }
        let id = qemu_nio_id(&link.link_id);
        let netdev = format!("ns_{id}");
        let device = format!("nsdev_{id}");
        if let Err(error) = self
            .qmp_execute(
                &vm.qmp_socket,
                "netdev_add",
                Some(json!({
                    "type": "socket",
                    "id": netdev,
                    "udp": format!("127.0.0.1:{}", link.adapter_port),
                    "localaddr": format!("127.0.0.1:{}", link.runtime_port)
                })),
            )
            .await
        {
            if qmp_reports_duplicate_id(&error) {
                return Ok(());
            }
            return Err(error);
        }
        if let Err(error) = self
            .qmp_execute(
                &vm.qmp_socket,
                "device_add",
                Some(json!({
                    "driver": vm.adapter_type,
                    "id": device,
                    "netdev": netdev,
                    "mac": mac_address(&vm.node_id, link.adapter_number)
                })),
            )
            .await
        {
            let _ = self
                .qmp_execute(&vm.qmp_socket, "netdev_del", Some(json!({ "id": netdev })))
                .await;
            return Err(error);
        }
        Ok(())
    }

    pub async fn detach_nio(&self, vm: &QemuVm, link: &QemuLink) -> Result<(), RuntimeError> {
        let id = qemu_nio_id(&link.link_id);
        if let Err(error) = self
            .qmp_execute(
                &vm.qmp_socket,
                "device_del",
                Some(json!({ "id": format!("nsdev_{id}") })),
            )
            .await
        {
            if !qmp_reports_missing_id(&error) {
                return Err(error);
            }
        }
        if let Err(error) = self
            .qmp_execute(
                &vm.qmp_socket,
                "netdev_del",
                Some(json!({ "id": format!("ns_{id}") })),
            )
            .await
        {
            if !qmp_reports_missing_id(&error) {
                return Err(error);
            }
        }
        Ok(())
    }

    pub async fn status(&self, vm: &QemuVm) -> Result<String, RuntimeError> {
        if !vm.qmp_socket.exists() {
            return Ok("stopped".to_owned());
        }
        let status = self
            .qmp_execute(&vm.qmp_socket, "query-status", None)
            .await?;
        let status = status
            .get("status")
            .and_then(Value::as_str)
            .ok_or_else(|| RuntimeError::Failed("QMP query-status omitted VM status".to_owned()))?;
        Ok(match status {
            "running" | "inmigrate" | "postmigrate" => "started",
            "paused" | "prelaunch" | "suspended" => "suspended",
            _ => "stopped",
        }
        .to_owned())
    }

    pub async fn remove_vm(&self, vm: &QemuVm) -> Result<(), RuntimeError> {
        self.stop(vm).await?;
        let mut runtime_files = vm
            .disks
            .iter()
            .filter(|disk| disk.active_file != disk.image)
            .map(|disk| &disk.active_file)
            .collect::<Vec<_>>();
        runtime_files.extend([
            &vm.qmp_socket,
            &vm.serial_socket,
            &vm.pid_file,
            &vm.log_file,
        ]);
        runtime_files.sort();
        runtime_files.dedup();
        for path in runtime_files {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(RuntimeError::Failed(format!(
                        "Could not remove QEMU runtime file '{}': {error}",
                        path.display()
                    )));
                }
            }
            let checksum = path.with_file_name(format!(
                "{}.md5sum",
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
            match fs::remove_file(&checksum) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(RuntimeError::Failed(format!(
                        "Could not remove QEMU checksum '{}': {error}",
                        checksum.display()
                    )));
                }
            }
        }
        Ok(())
    }

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

    pub fn prepare_upload_name(&self, file_name: &str) -> Result<(PathBuf, PathBuf), RuntimeError> {
        let safe_name = safe_disk_image_name(file_name)?;
        fs::create_dir_all(&self.image_dir).map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not create QEMU image directory '{}': {error}",
                self.image_dir.display()
            ))
        })?;
        let directory = self.image_dir.canonicalize().map_err(|error| {
            RuntimeError::Failed(format!("Could not resolve QEMU image directory: {error}"))
        })?;
        let final_path = directory.join(&safe_name);
        if final_path.exists() {
            return Err(RuntimeError::Failed(format!(
                "A QEMU image named '{safe_name}' already exists"
            )));
        }
        let temporary_path = directory.join(format!(".{}.{}.upload", Uuid::new_v4(), safe_name));
        Ok((temporary_path, final_path))
    }

    pub async fn finalize_uploaded_image(
        &self,
        temporary_path: &Path,
        final_path: &Path,
    ) -> Result<PathBuf, RuntimeError> {
        let metadata = fs::metadata(temporary_path).map_err(|error| {
            RuntimeError::Failed(format!("Could not read uploaded QEMU image: {error}"))
        })?;
        if metadata.len() == 0 {
            return Err(RuntimeError::Failed(
                "QEMU image upload is empty".to_owned(),
            ));
        }
        let output = self
            .command(
                &self.qemu_img,
                [
                    "info",
                    "--output=json",
                    temporary_path.to_str().ok_or_else(|| {
                        RuntimeError::Failed("QEMU image path is not valid UTF-8".to_owned())
                    })?,
                ],
                COMMAND_TIMEOUT,
            )
            .await?;
        let info: Value = serde_json::from_str(&output).map_err(|error| {
            RuntimeError::Failed(format!("qemu-img returned invalid image metadata: {error}"))
        })?;
        let format = info["format"].as_str().unwrap_or_default();
        if !image_format_matches(final_path, format) {
            return Err(RuntimeError::Failed(format!(
                "QEMU image extension does not match detected format '{format}'"
            )));
        }
        fs::hard_link(temporary_path, final_path).map_err(|error| {
            RuntimeError::Failed(format!("Could not install QEMU image: {error}"))
        })?;
        fs::remove_file(temporary_path).map_err(|error| {
            RuntimeError::Failed(format!("Could not finalize QEMU image upload: {error}"))
        })?;
        final_path.canonicalize().map_err(|error| {
            RuntimeError::Failed(format!("Could not resolve installed QEMU image: {error}"))
        })
    }

    pub async fn create_disk_image(
        &self,
        project_id: &str,
        node_id: &str,
        file_name: &str,
        format: &str,
        size_mb: u64,
        image_options: &std::collections::BTreeMap<String, String>,
    ) -> Result<PathBuf, RuntimeError> {
        self.create_disk_image_with_size(
            project_id,
            node_id,
            file_name,
            format,
            &format!("{size_mb}M"),
            image_options,
        )
        .await
    }

    pub async fn create_disk_image_with_size(
        &self,
        project_id: &str,
        node_id: &str,
        file_name: &str,
        format: &str,
        size: &str,
        image_options: &std::collections::BTreeMap<String, String>,
    ) -> Result<PathBuf, RuntimeError> {
        let project_id = parse_id(project_id)?;
        let node_id = parse_id(node_id)?;
        let file_name = safe_working_disk_name(file_name)?;
        if !matches!(format, "qcow2" | "qcow" | "raw" | "vmdk" | "vdi") {
            return Err(RuntimeError::Failed(
                "Unsupported QEMU disk format".to_owned(),
            ));
        }
        if !valid_qemu_disk_size(size) {
            return Err(RuntimeError::Failed("Invalid QEMU disk size".to_owned()));
        }
        let directory = self
            .state_dir
            .join(project_id.to_string())
            .join(node_id.to_string());
        fs::create_dir_all(&directory).map_err(|error| {
            RuntimeError::Failed(format!("Could not create QEMU node directory: {error}"))
        })?;
        if process_matches(
            &directory.join(format!("{node_id}.pid")),
            &self.qmp_dir.join(format!("{node_id}.sock")),
        ) {
            return Err(RuntimeError::Failed(
                "Cannot create a QEMU disk while its VM is running".to_owned(),
            ));
        }
        let target = directory.join(file_name);
        if target.exists() {
            return Err(RuntimeError::Failed(format!(
                "QEMU disk image '{}' already exists",
                target.file_name().unwrap_or_default().to_string_lossy()
            )));
        }
        let mut args = vec!["create".to_owned(), "-f".to_owned(), format.to_owned()];
        if !image_options.is_empty() {
            let options = image_options
                .iter()
                .map(|(key, value)| {
                    if key.is_empty()
                        || !key
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                        || value
                            .bytes()
                            .any(|byte| byte.is_ascii_control() || byte == b',')
                    {
                        return Err(RuntimeError::Failed(
                            "QEMU disk image options contain an invalid key or value".to_owned(),
                        ));
                    }
                    Ok(format!("{key}={value}"))
                })
                .collect::<Result<Vec<_>, _>>()?
                .join(",");
            args.extend(["-o".to_owned(), options]);
        }
        args.extend([
            target
                .to_str()
                .ok_or_else(|| {
                    RuntimeError::Failed("QEMU disk path is not valid UTF-8".to_owned())
                })?
                .to_owned(),
            size.to_owned(),
        ]);
        self.command(&self.qemu_img, args, COMMAND_TIMEOUT).await?;
        Ok(target)
    }

    pub async fn resize_disk_image(
        &self,
        project_id: &str,
        node_id: &str,
        file_name: &str,
        extend_mb: u64,
    ) -> Result<PathBuf, RuntimeError> {
        let project_id = parse_id(project_id)?;
        let node_id = parse_id(node_id)?;
        let file_name = safe_working_disk_name(file_name)?;
        if extend_mb == 0 || extend_mb > 1_000_000 {
            return Err(RuntimeError::Failed(
                "QEMU disk resize must add from 1 to 1000000 MB".to_owned(),
            ));
        }
        let directory = self
            .state_dir
            .join(project_id.to_string())
            .join(node_id.to_string());
        let target = directory.join(file_name);
        let root = directory.canonicalize().map_err(|error| {
            RuntimeError::NotFound(format!("QEMU node directory is unavailable: {error}"))
        })?;
        let canonical = target.canonicalize().map_err(|error| {
            RuntimeError::NotFound(format!("QEMU disk image is unavailable: {error}"))
        })?;
        if !canonical.starts_with(&root) || !canonical.is_file() {
            return Err(RuntimeError::Failed(
                "QEMU disk image must be inside the node directory".to_owned(),
            ));
        }
        let pid_file = directory.join(format!("{node_id}.pid"));
        let qmp_socket = self.qmp_dir.join(format!("{node_id}.sock"));
        if process_matches(&pid_file, &qmp_socket) {
            return Err(RuntimeError::Failed(
                "Cannot resize a QEMU disk while its VM is running".to_owned(),
            ));
        }
        self.command(
            &self.qemu_img,
            [
                "resize",
                canonical.to_str().ok_or_else(|| {
                    RuntimeError::Failed("QEMU disk path is not valid UTF-8".to_owned())
                })?,
                &format!("+{extend_mb}M"),
            ],
            COMMAND_TIMEOUT,
        )
        .await?;
        remove_checksum_sidecar(&canonical)?;
        Ok(canonical)
    }

    pub async fn resize_disk_image_to_size(
        &self,
        project_id: &str,
        node_id: &str,
        file_name: &str,
        size: &str,
        allow_shrink: bool,
    ) -> Result<PathBuf, RuntimeError> {
        let project_id = parse_id(project_id)?;
        let node_id = parse_id(node_id)?;
        let file_name = safe_working_disk_name(file_name)?;
        if !valid_qemu_disk_size(size) {
            return Err(RuntimeError::Failed("Invalid QEMU disk size".to_owned()));
        }
        let directory = self
            .state_dir
            .join(project_id.to_string())
            .join(node_id.to_string());
        let target = directory.join(file_name);
        let root = directory.canonicalize().map_err(|error| {
            RuntimeError::NotFound(format!("QEMU node directory is unavailable: {error}"))
        })?;
        let canonical = target.canonicalize().map_err(|error| {
            RuntimeError::NotFound(format!("QEMU disk image is unavailable: {error}"))
        })?;
        if !canonical.starts_with(&root) || !canonical.is_file() {
            return Err(RuntimeError::Failed(
                "QEMU disk image must be inside the node directory".to_owned(),
            ));
        }
        if process_matches(
            &directory.join(format!("{node_id}.pid")),
            &self.qmp_dir.join(format!("{node_id}.sock")),
        ) {
            return Err(RuntimeError::Failed(
                "Cannot resize a QEMU disk while its VM is running".to_owned(),
            ));
        }
        let mut args = vec!["resize".to_owned()];
        if allow_shrink {
            args.push("--shrink".to_owned());
        }
        args.push(canonical.to_string_lossy().into_owned());
        args.push(size.to_owned());
        self.command(&self.qemu_img, args, COMMAND_TIMEOUT).await?;
        remove_checksum_sidecar(&canonical)?;
        Ok(canonical)
    }

    pub fn delete_disk_image(
        &self,
        project_id: &str,
        node_id: &str,
        file_name: &str,
    ) -> Result<PathBuf, RuntimeError> {
        let project_id = parse_id(project_id)?;
        let node_id = parse_id(node_id)?;
        let file_name = safe_working_disk_name(file_name)?;
        let directory = self
            .state_dir
            .join(project_id.to_string())
            .join(node_id.to_string());
        let root = directory.canonicalize().map_err(|error| {
            RuntimeError::NotFound(format!("QEMU node directory is unavailable: {error}"))
        })?;
        let target = directory.join(file_name);
        let canonical = target.canonicalize().map_err(|error| {
            RuntimeError::NotFound(format!("QEMU disk image is unavailable: {error}"))
        })?;
        if !canonical.starts_with(&root) || !canonical.is_file() {
            return Err(RuntimeError::Failed(
                "QEMU disk image must be inside the node directory".to_owned(),
            ));
        }
        if process_matches(
            &directory.join(format!("{node_id}.pid")),
            &self.qmp_dir.join(format!("{node_id}.sock")),
        ) {
            return Err(RuntimeError::Failed(
                "Cannot delete a QEMU disk while its VM is running".to_owned(),
            ));
        }
        remove_disk_and_checksum(&canonical)?;
        Ok(canonical)
    }

    fn platform(&self) -> String {
        self.binary
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("qemu-system-"))
            .filter(|platform| !platform.is_empty())
            .unwrap_or("x86_64")
            .to_owned()
    }

    fn image_files(&self) -> Result<Vec<PathBuf>, RuntimeError> {
        let directory = match self.image_dir.canonicalize() {
            Ok(directory) => directory,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(RuntimeError::Failed(format!(
                    "Could not read QEMU image directory '{}': {error}",
                    self.image_dir.display()
                )));
            }
        };
        let entries = fs::read_dir(&directory).map_err(|error| {
            RuntimeError::Failed(format!("Could not list QEMU images: {error}"))
        })?;
        let mut files = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                RuntimeError::Failed(format!("Could not read QEMU image entry: {error}"))
            })?;
            let path = entry.path();
            if !path.is_file() || !is_disk_image(&path) {
                continue;
            }
            let canonical = path.canonicalize().map_err(|error| {
                RuntimeError::Failed(format!("Could not resolve QEMU image path: {error}"))
            })?;
            if canonical.starts_with(&directory) {
                files.push(canonical);
            }
        }
        files.sort();
        Ok(files)
    }

    fn resolve_image(&self, image: &str) -> Result<PathBuf, RuntimeError> {
        let root = self.image_dir.canonicalize().map_err(|error| {
            RuntimeError::Unavailable(format!(
                "QEMU image directory '{}' is unavailable: {error}",
                self.image_dir.display()
            ))
        })?;
        let path = Path::new(image).canonicalize().map_err(|error| {
            RuntimeError::NotFound(format!("QEMU disk image '{image}' is unavailable: {error}"))
        })?;
        if !path.starts_with(&root) || !path.is_file() || !is_disk_image(&path) {
            return Err(RuntimeError::Failed(
                "QEMU disk images must be supported image files inside the configured image directory".to_owned(),
            ));
        }
        Ok(path)
    }

    fn resolve_node_image(&self, image: &str, vm_dir: &Path) -> Result<PathBuf, RuntimeError> {
        let candidate = Path::new(image);
        let candidate = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            let node_candidate = vm_dir.join(candidate);
            if node_candidate.exists() {
                node_candidate
            } else {
                self.image_dir.join(candidate)
            }
        };
        let path = candidate.canonicalize().map_err(|error| {
            RuntimeError::NotFound(format!("QEMU disk image '{image}' is unavailable: {error}"))
        })?;
        let image_root = self.image_dir.canonicalize().map_err(|error| {
            RuntimeError::Unavailable(format!(
                "QEMU image directory '{}' is unavailable: {error}",
                self.image_dir.display()
            ))
        })?;
        let node_root = vm_dir.canonicalize().map_err(|error| {
            RuntimeError::Unavailable(format!("QEMU node directory is unavailable: {error}"))
        })?;
        if !(path.is_file()
            && is_disk_image(&path)
            && (path.starts_with(&image_root) || path.starts_with(&node_root)))
        {
            return Err(RuntimeError::Failed(
                "QEMU disk images must be supported image files inside the configured image directory or node directory".to_owned(),
            ));
        }
        Ok(path)
    }

    fn resolve_auxiliary_file(&self, file: &str) -> Result<PathBuf, RuntimeError> {
        let root = self.image_dir.canonicalize().map_err(|error| {
            RuntimeError::Unavailable(format!(
                "QEMU image directory '{}' is unavailable: {error}",
                self.image_dir.display()
            ))
        })?;
        let candidate = Path::new(file);
        let candidate = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            root.join(candidate)
        };
        let path = candidate.canonicalize().map_err(|error| {
            RuntimeError::NotFound(format!(
                "QEMU support file '{file}' is unavailable: {error}"
            ))
        })?;
        if !path.starts_with(&root) || !path.is_file() {
            return Err(RuntimeError::Failed(
                "QEMU support files must be regular files inside the configured image directory"
                    .to_owned(),
            ));
        }
        Ok(path)
    }

    fn resolve_optional_auxiliary_file(
        &self,
        node: &Value,
        key: &str,
    ) -> Result<Option<PathBuf>, RuntimeError> {
        node[key]
            .as_str()
            .filter(|path| !path.trim().is_empty())
            .map(|path| self.resolve_auxiliary_file(path))
            .transpose()
    }

    fn config_disk_image(&self) -> Result<PathBuf, RuntimeError> {
        let configured = std::env::var_os("NETWORK_SIMULATOR_QEMU_CONFIG_DISK")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.image_dir.join("config.img"));
        let path = configured.canonicalize().map_err(|error| {
            RuntimeError::Unavailable(format!(
                "QEMU config disk is unavailable at '{}': {error}",
                configured.display()
            ))
        })?;
        if !path.is_file() {
            return Err(RuntimeError::Unavailable(format!(
                "QEMU config disk is not a regular file: '{}'",
                path.display()
            )));
        }
        Ok(path)
    }

    async fn image_format(&self, image: &Path) -> Result<String, RuntimeError> {
        let output = self
            .command(
                &self.qemu_img,
                [
                    "info",
                    "--output=json",
                    image.to_str().ok_or_else(|| {
                        RuntimeError::Failed("QEMU image path is not valid UTF-8".to_owned())
                    })?,
                ],
                COMMAND_TIMEOUT,
            )
            .await?;
        let info: ImageInfo = serde_json::from_str(&output).map_err(|error| {
            RuntimeError::Failed(format!("qemu-img returned invalid image metadata: {error}"))
        })?;
        if info.format.is_empty() || !info.format.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Err(RuntimeError::Failed(
                "qemu-img returned an invalid disk format".to_owned(),
            ));
        }
        Ok(info.format)
    }

    async fn create_overlay(
        &self,
        image: &Path,
        source_format: &str,
        overlay: &Path,
    ) -> Result<(), RuntimeError> {
        if overlay.exists() {
            return Ok(());
        }
        self.command(
            &self.qemu_img,
            [
                "create",
                "-f",
                "qcow2",
                "-F",
                source_format,
                "-b",
                image.to_str().ok_or_else(|| {
                    RuntimeError::Failed("QEMU image path is not valid UTF-8".to_owned())
                })?,
                overlay.to_str().ok_or_else(|| {
                    RuntimeError::Failed("QEMU overlay path is not valid UTF-8".to_owned())
                })?,
            ],
            COMMAND_TIMEOUT,
        )
        .await?;
        Ok(())
    }

    async fn start(&self, vm: &QemuVm, links: &[QemuLink]) -> Result<String, RuntimeError> {
        if !vm.overlay.is_file() {
            return Err(RuntimeError::NotFound(format!(
                "QEMU node disk overlay '{}' is missing",
                vm.overlay.display()
            )));
        }
        if process_matches(&vm.pid_file, &vm.qmp_socket) {
            if !vm.qmp_socket.exists() {
                return Err(RuntimeError::Unavailable(
                    "QEMU is running but its QMP monitor socket is missing".to_owned(),
                ));
            }
            match self.status(vm).await {
                Ok(status) if status == "started" => {
                    self.start_cpu_limiter(vm).await?;
                    return Ok("started".to_owned());
                }
                Ok(status) if status == "suspended" => {
                    self.qmp_execute(&vm.qmp_socket, "cont", None).await?;
                    self.start_cpu_limiter(vm).await?;
                    return self.status(vm).await;
                }
                Ok(status) if status == "stopped" => {
                    self.qmp_execute(&vm.qmp_socket, "system_reset", None)
                        .await?;
                    return self.status(vm).await;
                }
                Ok(_) => {
                    return Err(RuntimeError::Failed(
                        "QEMU is running in a state that cannot be resumed".to_owned(),
                    ));
                }
                Err(error) => {
                    return Err(RuntimeError::Unavailable(format!(
                        "QEMU is running but its QMP monitor is not responding: {}",
                        error.message()
                    )));
                }
            }
        }
        let _ = fs::remove_file(&vm.qmp_socket);
        let _ = fs::remove_file(&vm.serial_socket);
        let _ = fs::remove_file(&vm.pid_file);
        let _ = fs::remove_file(&vm.log_file);
        let runtime_arguments = qemu_runtime_arguments(vm)?;
        let mut command = Command::new(&self.binary);
        command
            .arg("-daemonize")
            .arg("-pidfile")
            .arg(&vm.pid_file)
            .arg("-D")
            .arg(&vm.log_file)
            .arg("-qmp")
            .arg(format!(
                "unix:{},server=on,wait=off",
                vm.qmp_socket.display()
            ));
        command.arg("-monitor").arg("none");
        if matches!(vm.console_type.as_str(), "telnet" | "ssh") {
            command
                .arg("-chardev")
                .arg(format!(
                    "socket,id=serial0,path={},server=on,wait=off",
                    qemu_keyval_escape(&vm.serial_socket.display().to_string())
                ))
                .arg("-serial")
                .arg("chardev:serial0");
        }
        // QEMU treats QMP system_reset as a shutdown when -no-reboot is set.
        command.args(runtime_arguments);
        // TODO(RUNTIME-NET QEMU custom adapter hook): materialize adapter
        // counts without links, custom_adapters, configured MAC addresses, and
        // carrier replication through NET's NIC work.
        if links.is_empty() {
            command.arg("-nic").arg("none");
        } else {
            let mut links = links.to_vec();
            links.sort_by_key(|link| link.adapter_number);
            for adapter_number in 0..vm.adapters {
                if let Some(link) = links
                    .iter()
                    .find(|link| link.adapter_number == adapter_number)
                {
                    let id = qemu_nio_id(&link.link_id);
                    let net_id = format!("ns_{id}");
                    command
                        .arg("-netdev")
                        .arg(qemu_netdev_argument(link, &net_id))
                        .arg("-device")
                        .arg(format!(
                            "{},netdev={net_id},mac={}",
                            vm.adapter_type,
                            mac_address(&vm.node_id, adapter_number)
                        ));
                } else {
                    let net_id = format!("ns_unused_{adapter_number}");
                    command
                        .arg("-netdev")
                        .arg(format!("null,id={net_id}"))
                        .arg("-device")
                        .arg(format!(
                            "{},netdev={net_id},mac={}",
                            vm.adapter_type,
                            mac_address(&vm.node_id, adapter_number)
                        ));
                }
            }
        }
        if vm.tpm {
            self.start_tpm(vm).await?;
        }
        set_child_process_priority(&mut command, &vm.process_priority)?;
        let output = timeout(COMMAND_TIMEOUT, command.stdin(Stdio::null()).output())
            .await
            .map_err(|_| RuntimeError::Unavailable("QEMU startup timed out".to_owned()))
            .and_then(|output| {
                output.map_err(|error| {
                    RuntimeError::Unavailable(format!(
                        "Could not run QEMU binary '{}': {error}",
                        self.binary.display()
                    ))
                })
            });
        let output = match output {
            Ok(output) => output,
            Err(error) => {
                self.stop_tpm(vm).await;
                return Err(error);
            }
        };
        if !output.status.success() {
            self.stop_tpm(vm).await;
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(RuntimeError::Failed(if stderr.is_empty() {
                format!("QEMU exited with {}", output.status)
            } else {
                stderr
            }));
        }
        let deadline = tokio::time::Instant::now() + START_TIMEOUT;
        loop {
            if vm.qmp_socket.exists() {
                match self.status(vm).await {
                    Ok(status) if status == "started" || status == "suspended" => {
                        if status == "started"
                            && let Err(error) = self.start_cpu_limiter(vm).await
                        {
                            let _ = self.stop(vm).await;
                            return Err(error);
                        }
                        return Ok(status);
                    }
                    Ok(_) => {}
                    Err(error) => {
                        if tokio::time::Instant::now() >= deadline {
                            self.stop_tpm(vm).await;
                            let log = fs::read_to_string(&vm.log_file).unwrap_or_default();
                            return Err(RuntimeError::Failed(format!(
                                "QEMU started without a usable QMP monitor: {}{}",
                                error.message(),
                                if log.is_empty() {
                                    String::new()
                                } else {
                                    format!("; {log}")
                                }
                            )));
                        }
                    }
                }
            }
            if tokio::time::Instant::now() >= deadline {
                self.stop_tpm(vm).await;
                let log = fs::read_to_string(&vm.log_file).unwrap_or_default();
                return Err(RuntimeError::Failed(format!(
                    "QEMU did not create its QMP monitor{}",
                    if log.is_empty() {
                        String::new()
                    } else {
                        format!(": {log}")
                    }
                )));
            }
            sleep(Duration::from_millis(100)).await;
        }
    }

    async fn stop(&self, vm: &QemuVm) -> Result<String, RuntimeError> {
        let pid_matches = process_matches(&vm.pid_file, &vm.qmp_socket);
        if !vm.qmp_socket.exists() && pid_matches {
            return Err(RuntimeError::Unavailable(
                "QEMU is running but its QMP monitor socket is missing".to_owned(),
            ));
        }
        if !vm.qmp_socket.exists() {
            let _ = fs::remove_file(&vm.qmp_socket);
            let _ = fs::remove_file(&vm.serial_socket);
            let _ = fs::remove_file(&vm.pid_file);
            self.stop_tpm(vm).await;
            self.stop_cpu_limiter(vm).await;
            return Ok("stopped".to_owned());
        }
        if vm.on_close == "shutdown_signal" {
            let _ = self
                .qmp_execute(&vm.qmp_socket, "system_powerdown", None)
                .await;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
            while process_matches(&vm.pid_file, &vm.qmp_socket)
                && tokio::time::Instant::now() < deadline
            {
                sleep(Duration::from_millis(200)).await;
            }
        } else if vm.on_close == "save_vm_state" {
            self.qmp_execute(&vm.qmp_socket, "stop", None).await?;
            self.qmp_execute(
                &vm.qmp_socket,
                "human-monitor-command",
                Some(json!({ "command-line": "savevm GNS3_SAVED_STATE" })),
            )
            .await?;
        }
        if process_matches(&vm.pid_file, &vm.qmp_socket)
            && let Err(error) = self.qmp_execute(&vm.qmp_socket, "quit", None).await
        {
            if pid_matches {
                return Err(RuntimeError::Unavailable(format!(
                    "Could not stop QEMU because its QMP monitor is unavailable: {}",
                    error.message()
                )));
            }
            let _ = fs::remove_file(&vm.qmp_socket);
            let _ = fs::remove_file(&vm.serial_socket);
            let _ = fs::remove_file(&vm.pid_file);
            self.stop_tpm(vm).await;
            self.stop_cpu_limiter(vm).await;
            return Ok("stopped".to_owned());
        }
        let deadline = tokio::time::Instant::now() + START_TIMEOUT;
        while process_matches(&vm.pid_file, &vm.qmp_socket) {
            if tokio::time::Instant::now() >= deadline {
                return Err(RuntimeError::Failed(
                    "QEMU did not exit after the QMP quit request".to_owned(),
                ));
            }
            sleep(Duration::from_millis(100)).await;
        }
        let _ = fs::remove_file(&vm.qmp_socket);
        let _ = fs::remove_file(&vm.serial_socket);
        let _ = fs::remove_file(&vm.pid_file);
        self.stop_tpm(vm).await;
        self.stop_cpu_limiter(vm).await;
        Ok("stopped".to_owned())
    }

    async fn start_cpu_limiter(&self, vm: &QemuVm) -> Result<(), RuntimeError> {
        if vm.cpu_throttling == 0
            || self
                .cpu_limit_processes
                .lock()
                .await
                .contains_key(&vm.node_id)
        {
            return Ok(());
        }
        let pid = fs::read_to_string(&vm.pid_file)
            .map_err(|error| {
                RuntimeError::Failed(format!(
                    "Could not read QEMU pid file for CPU throttling: {error}"
                ))
            })?
            .trim()
            .parse::<u32>()
            .map_err(|_| {
                RuntimeError::Failed("QEMU pid file contains an invalid process ID".to_owned())
            })?;
        let binary = std::env::var_os("NETWORK_SIMULATOR_CPULIMIT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("cpulimit"));
        let mut child = Command::new(&binary)
            .args(cpu_limiter_arguments(pid, vm.cpu_throttling))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| RuntimeError::Unavailable(format!(
                "QEMU CPU throttling is enabled but cpulimit could not be started with '{}': {error}",
                binary.display()
            )))?;
        if let Some(status) = child.try_wait().map_err(|error| {
            RuntimeError::Unavailable(format!("Could not inspect cpulimit startup: {error}"))
        })? {
            return Err(RuntimeError::Failed(format!(
                "cpulimit exited during startup ({status})"
            )));
        }
        self.cpu_limit_processes
            .lock()
            .await
            .insert(vm.node_id.clone(), child);
        Ok(())
    }

    async fn stop_cpu_limiter(&self, vm: &QemuVm) {
        let Some(mut child) = self.cpu_limit_processes.lock().await.remove(&vm.node_id) else {
            return;
        };
        if child.try_wait().ok().flatten().is_none() {
            let _ = child.start_kill();
            let _ = timeout(Duration::from_secs(2), child.wait()).await;
        }
    }

    async fn start_tpm(&self, vm: &QemuVm) -> Result<(), RuntimeError> {
        if self.tpm_processes.lock().await.contains_key(&vm.node_id) {
            return Ok(());
        }
        let binary = std::env::var_os("NETWORK_SIMULATOR_SWTPM")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("swtpm"));
        let working_dir = vm
            .pid_file
            .parent()
            .ok_or_else(|| RuntimeError::Failed("QEMU working directory is missing".to_owned()))?;
        let state_dir = working_dir.join("tpm");
        fs::create_dir_all(&state_dir).map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not create QEMU TPM state directory: {error}"
            ))
        })?;
        let socket = tpm_socket_path(vm);
        let _ = fs::remove_file(&socket);
        let state = format!("dir={}", state_dir.display());
        let server = format!("type=unixio,path={},mode=0600", socket.display());
        let mut child = Command::new(&binary)
            .args([
                "socket",
                "--tpm2",
                "--tpmstate",
                &state,
                "--server",
                &server,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                RuntimeError::Unavailable(format!(
                    "TPM is enabled but swtpm could not be started with '{}': {error}",
                    binary.display()
                ))
            })?;
        let deadline = tokio::time::Instant::now() + START_TIMEOUT;
        while !socket.exists() {
            if let Some(status) = child.try_wait().map_err(|error| {
                RuntimeError::Unavailable(format!("Could not inspect swtpm startup: {error}"))
            })? {
                return Err(RuntimeError::Failed(format!(
                    "swtpm exited during startup ({status})"
                )));
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(RuntimeError::Unavailable(
                    "swtpm did not create its Unix socket".to_owned(),
                ));
            }
            sleep(Duration::from_millis(50)).await;
        }
        self.tpm_processes
            .lock()
            .await
            .insert(vm.node_id.clone(), child);
        Ok(())
    }

    async fn stop_tpm(&self, vm: &QemuVm) {
        let Some(mut process) = self.tpm_processes.lock().await.remove(&vm.node_id) else {
            return;
        };
        if process.try_wait().ok().flatten().is_none() {
            let _ = process.start_kill();
            let _ = timeout(Duration::from_secs(2), process.wait()).await;
        }
        let _ = fs::remove_file(tpm_socket_path(vm));
    }

    async fn qmp_execute(
        &self,
        socket: &Path,
        execute: &str,
        arguments: Option<Value>,
    ) -> Result<Value, RuntimeError> {
        timeout(QMP_TIMEOUT, qmp_execute_inner(socket, execute, arguments))
            .await
            .map_err(|_| RuntimeError::Unavailable("QEMU QMP request timed out".to_owned()))?
    }

    async fn command<I, S>(
        &self,
        binary: &Path,
        args: I,
        duration: Duration,
    ) -> Result<String, RuntimeError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let mut command = Command::new(binary);
        command.args(args).stdin(Stdio::null()).kill_on_drop(true);
        let output = timeout(duration, command.output())
            .await
            .map_err(|_| RuntimeError::Unavailable("QEMU utility command timed out".to_owned()))?
            .map_err(|error| {
                RuntimeError::Unavailable(format!(
                    "Could not run QEMU utility '{}': {error}",
                    binary.display()
                ))
            })?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(RuntimeError::Failed(if stderr.is_empty() {
            format!("QEMU utility exited with {}", output.status)
        } else {
            stderr
        }))
    }
}

impl Default for QemuRuntime {
    fn default() -> Self {
        Self::new()
    }
}

fn qemu_runtime_arguments(vm: &QemuVm) -> Result<Vec<String>, RuntimeError> {
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

fn validate_extra_options(options: &str) -> Result<(), RuntimeError> {
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

fn qemu_disk_arguments(disk: &QemuDisk) -> Result<Vec<String>, RuntimeError> {
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

fn uefi_arguments(vm: &QemuVm) -> Result<Vec<String>, RuntimeError> {
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

fn locate_ovmf_file(name: &str, image: &Path) -> Result<PathBuf, RuntimeError> {
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

fn set_child_process_priority(command: &mut Command, priority: &str) -> Result<(), RuntimeError> {
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

fn cpu_limiter_arguments(pid: u32, percentage: u32) -> [String; 3] {
    [
        "--lazy".to_owned(),
        format!("--pid={pid}"),
        format!("--limit={percentage}"),
    ]
}

fn tpm_socket_path(vm: &QemuVm) -> PathBuf {
    vm.pid_file.with_extension("swtpm.sock")
}

async fn qmp_execute_inner(
    socket: &Path,
    execute: &str,
    arguments: Option<Value>,
) -> Result<Value, RuntimeError> {
    let stream = UnixStream::connect(socket).await.map_err(|error| {
        RuntimeError::Unavailable(format!("Could not connect to QEMU QMP socket: {error}"))
    })?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).await.map_err(|error| {
        RuntimeError::Unavailable(format!("Could not read the QEMU QMP greeting: {error}"))
    })?;
    let greeting: Value = serde_json::from_str(&line).map_err(|error| {
        RuntimeError::Failed(format!("QEMU returned an invalid QMP greeting: {error}"))
    })?;
    if greeting.get("QMP").is_none() {
        return Err(RuntimeError::Failed(
            "QEMU QMP socket did not send a protocol greeting".to_owned(),
        ));
    }
    send_qmp_command(&mut reader, "qmp_capabilities", None, "capabilities").await?;
    send_qmp_command(&mut reader, execute, arguments, "request").await
}

async fn send_qmp_command(
    reader: &mut BufReader<UnixStream>,
    execute: &str,
    arguments: Option<Value>,
    id: &str,
) -> Result<Value, RuntimeError> {
    let mut request = json!({ "execute": execute, "id": id });
    if let Some(arguments) = arguments {
        request["arguments"] = arguments;
    }
    let mut serialized = serde_json::to_vec(&request)
        .map_err(|error| RuntimeError::Failed(format!("Could not encode QMP request: {error}")))?;
    serialized.push(b'\n');
    reader
        .get_mut()
        .write_all(&serialized)
        .await
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not write a QEMU QMP request: {error}"))
        })?;
    loop {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).await.map_err(|error| {
            RuntimeError::Unavailable(format!("Could not read a QEMU QMP response: {error}"))
        })?;
        if bytes == 0 {
            return Err(RuntimeError::Unavailable(
                "QEMU closed the QMP connection before replying".to_owned(),
            ));
        }
        let response: Value = serde_json::from_str(&line).map_err(|error| {
            RuntimeError::Failed(format!("QEMU returned an invalid QMP response: {error}"))
        })?;
        if response.get("event").is_some() {
            continue;
        }
        if response.get("id").and_then(Value::as_str) != Some(id) {
            continue;
        }
        if let Some(error) = response.get("error") {
            return Err(RuntimeError::Failed(format!(
                "QEMU QMP {execute} failed: {error}"
            )));
        }
        return Ok(response.get("return").cloned().unwrap_or(Value::Null));
    }
}

fn parse_id(id: &str) -> Result<Uuid, RuntimeError> {
    Uuid::parse_str(id).map_err(|_| RuntimeError::Failed("QEMU IDs must be valid UUIDs".to_owned()))
}

fn valid_disk_interface(interface: &str) -> bool {
    matches!(
        interface,
        "ide" | "sata" | "nvme" | "scsi" | "sd" | "mtd" | "floppy" | "pflash" | "virtio" | "none"
    )
}

fn valid_console_type(console_type: &str) -> bool {
    matches!(
        console_type,
        "telnet" | "ssh" | "vnc" | "spice" | "spice+agent" | "none"
    )
}

fn valid_adapter_type(adapter_type: &str) -> bool {
    matches!(
        adapter_type,
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
}

async fn available_tcp_port() -> Result<u16, RuntimeError> {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not allocate a QEMU TCP port: {error}"))
        })?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not read QEMU TCP port: {error}"))
        })
}

async fn available_vnc_port() -> Result<u16, RuntimeError> {
    for port in 5900..6900 {
        if tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .await
            .is_ok()
        {
            return Ok(port);
        }
    }
    Err(RuntimeError::Unavailable(
        "Could not allocate a free QEMU VNC display port".to_owned(),
    ))
}

fn validated_runtime_path(
    node: &Value,
    key: &str,
    expected: &Path,
) -> Result<PathBuf, RuntimeError> {
    let supplied = node
        .get(key)
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .ok_or_else(|| {
            RuntimeError::Failed(format!("QEMU node is missing runtime field '{key}'"))
        })?;
    if supplied != expected {
        return Err(RuntimeError::Failed(format!(
            "QEMU runtime field '{key}' must stay inside the controller runtime directory"
        )));
    }
    Ok(expected.to_path_buf())
}

fn validated_optional_runtime_path(
    node: &Value,
    key: &str,
    expected: &Path,
) -> Result<PathBuf, RuntimeError> {
    match node.get(key).and_then(Value::as_str) {
        Some(supplied) if Path::new(supplied) != expected => Err(RuntimeError::Failed(format!(
            "QEMU runtime field '{key}' must stay inside the controller runtime directory"
        ))),
        _ => Ok(expected.to_path_buf()),
    }
}

fn is_disk_image(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("qcow2" | "qcow" | "raw" | "img" | "vmdk" | "vdi")
    )
}

fn safe_disk_image_name(file_name: &str) -> Result<String, RuntimeError> {
    if file_name.is_empty()
        || file_name.len() > 180
        || file_name.starts_with('.')
        || !file_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+'))
        || !is_disk_image(Path::new(file_name))
    {
        return Err(RuntimeError::Failed(
            "QEMU image name must be a single supported disk-image filename".to_owned(),
        ));
    }
    Ok(file_name.to_owned())
}

fn safe_working_disk_name(file_name: &str) -> Result<String, RuntimeError> {
    if file_name.is_empty()
        || file_name.len() > 180
        || file_name.starts_with('.')
        || !file_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+'))
        || !is_disk_image(Path::new(file_name))
    {
        return Err(RuntimeError::Failed(
            "QEMU disk name must be a single supported disk-image filename".to_owned(),
        ));
    }
    Ok(file_name.to_owned())
}

fn valid_qemu_disk_size(size: &str) -> bool {
    if size.is_empty() || matches!(size.as_bytes().first(), Some(b'+' | b'-')) || !size.is_ascii() {
        return false;
    }
    let digits = size.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0
        || size[..digits]
            .parse::<u64>()
            .ok()
            .is_none_or(|value| value == 0)
    {
        return false;
    }
    matches!(
        &size[digits..].to_ascii_lowercase()[..],
        "" | "b"
            | "k"
            | "kb"
            | "kib"
            | "m"
            | "mb"
            | "mib"
            | "g"
            | "gb"
            | "gib"
            | "t"
            | "tb"
            | "tib"
    )
}

fn remove_disk_and_checksum(path: &Path) -> Result<(), RuntimeError> {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => {
            return Err(RuntimeError::Failed(format!(
                "Could not remove QEMU disk file '{}': {error}",
                path.display()
            )));
        }
    }
    remove_checksum_sidecar(path)
}

fn remove_checksum_sidecar(path: &Path) -> Result<(), RuntimeError> {
    let target = path.with_file_name(format!(
        "{}.md5sum",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    match fs::remove_file(&target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(RuntimeError::Failed(format!(
            "Could not remove QEMU checksum '{}': {error}",
            target.display()
        ))),
    }
}

fn cached_md5(path: &Path, cache_dir: &Path) -> Option<String> {
    if !path.is_file() {
        return None;
    }
    let cache_name = format!("{}.md5sum", path.file_name()?.to_string_lossy());
    let cache_path = cache_dir.join(cache_name);
    if let Ok(cached) = fs::read_to_string(&cache_path) {
        let cached = cached.trim();
        if cached.len() == 32 && cached.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Some(cached.to_ascii_lowercase());
        }
    }
    let mut file = fs::File::open(path).ok()?;
    let mut digest = Md5::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        use std::io::Read;
        let length = file.read(&mut buffer).ok()?;
        if length == 0 {
            break;
        }
        digest.update(&buffer[..length]);
    }
    let digest = format!("{:x}", digest.finalize());
    let _ = fs::write(cache_path, &digest);
    Some(digest)
}

fn image_format_matches(path: &Path, format: &str) -> bool {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("qcow2") => format == "qcow2",
        Some("qcow") => format == "qcow",
        Some("raw" | "img") => format == "raw",
        Some("vmdk") => format == "vmdk",
        Some("vdi") => format == "vdi",
        _ => false,
    }
}

fn qemu_keyval_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace(',', "\\,")
}

fn mac_address(node_id: &str, adapter_number: u32) -> String {
    let bytes = Uuid::parse_str(node_id)
        .map(|id| *id.as_bytes())
        .unwrap_or([0; 16]);
    format!(
        "52:54:00:{:02x}:{:02x}:{:02x}",
        bytes[13] ^ ((adapter_number >> 16) as u8),
        bytes[14] ^ ((adapter_number >> 8) as u8),
        bytes[15] ^ (adapter_number as u8)
    )
}

fn qemu_netdev_argument(link: &QemuLink, net_id: &str) -> String {
    format!(
        "socket,id={net_id},udp=127.0.0.1:{},localaddr=127.0.0.1:{}",
        link.adapter_port, link.runtime_port
    )
}

fn qemu_nio_id(link_id: &str) -> String {
    link_id.chars().filter(char::is_ascii_hexdigit).collect()
}

fn qmp_reports_duplicate_id(error: &RuntimeError) -> bool {
    let message = error.message().to_ascii_lowercase();
    message.contains("duplicate") || message.contains("already exists")
}

fn qmp_reports_missing_id(error: &RuntimeError) -> bool {
    let message = error.message().to_ascii_lowercase();
    message.contains("not found")
        || message.contains("does not exist")
        || message.contains("not a valid")
}

fn process_matches(pid_file: &Path, qmp_socket: &Path) -> bool {
    let Ok(pid) = fs::read_to_string(pid_file) else {
        return false;
    };
    let pid = pid.trim();
    if !pid.bytes().all(|byte| byte.is_ascii_digit()) || pid.is_empty() {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        let command_line = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
        let command_line = String::from_utf8_lossy(&command_line);
        let socket_path = qmp_socket.display().to_string();
        if command_line.contains(&socket_path) {
            return true;
        }
        let environment = fs::read(format!("/proc/{pid}/environ")).unwrap_or_default();
        environment
            .windows(socket_path.len())
            .any(|value| value == socket_path.as_bytes())
    }
    #[cfg(not(target_os = "linux"))]
    {
        qmp_socket.exists()
    }
}

fn kvm_accessible() -> bool {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .is_ok()
}

fn kvm_access_error() -> Option<String> {
    match fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
    {
        Ok(_) => None,
        Err(error) => Some(format!(
            "KVM is unavailable to this service account: {error}"
        )),
    }
}

#[derive(serde::Deserialize)]
struct ImageInfo {
    format: String,
}

#[cfg(test)]
mod tests {
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
}
