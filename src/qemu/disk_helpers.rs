use super::*;

pub(super) fn remove_disk_and_checksum(path: &Path) -> Result<(), RuntimeError> {
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

pub(super) fn remove_checksum_sidecar(path: &Path) -> Result<(), RuntimeError> {
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

pub(super) fn cached_md5(path: &Path, cache_dir: &Path) -> Option<String> {
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

pub(super) fn image_format_matches(path: &Path, format: &str) -> bool {
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

