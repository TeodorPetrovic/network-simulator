use super::*;

pub(super) fn parse_id(id: &str) -> Result<Uuid, RuntimeError> {
    Uuid::parse_str(id).map_err(|_| RuntimeError::Failed("QEMU IDs must be valid UUIDs".to_owned()))
}

pub(super) fn valid_disk_interface(interface: &str) -> bool {
    matches!(
        interface,
        "ide" | "sata" | "nvme" | "scsi" | "sd" | "mtd" | "floppy" | "pflash" | "virtio" | "none"
    )
}

pub(super) fn valid_console_type(console_type: &str) -> bool {
    matches!(
        console_type,
        "telnet" | "ssh" | "vnc" | "spice" | "spice+agent" | "none"
    )
}

pub(super) fn valid_adapter_type(adapter_type: &str) -> bool {
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

pub(super) async fn available_tcp_port() -> Result<u16, RuntimeError> {
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

pub(super) async fn available_vnc_port() -> Result<u16, RuntimeError> {
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

pub(super) fn validated_runtime_path(
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

pub(super) fn validated_optional_runtime_path(
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

pub(super) fn is_disk_image(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("qcow2" | "qcow" | "raw" | "img" | "vmdk" | "vdi")
    )
}

pub(super) fn safe_disk_image_name(file_name: &str) -> Result<String, RuntimeError> {
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

pub(super) fn safe_working_disk_name(file_name: &str) -> Result<String, RuntimeError> {
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

pub(super) fn valid_qemu_disk_size(size: &str) -> bool {
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

