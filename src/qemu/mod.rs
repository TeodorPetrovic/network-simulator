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

}

mod create;

impl QemuRuntime {
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

}

mod disks;


mod images;

impl QemuRuntime {
}

mod lifecycle;

impl QemuRuntime {
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

mod arguments;
use arguments::*;

fn tpm_socket_path(vm: &QemuVm) -> PathBuf {
    vm.pid_file.with_extension("swtpm.sock")
}

mod qmp;
use qmp::*;

mod config;
use config::*;

mod disk_helpers;
use disk_helpers::*;

mod network;
use network::*;

mod process;
use process::*;

#[derive(serde::Deserialize)]
struct ImageInfo {
    format: String,
}

#[cfg(test)]
mod tests;
