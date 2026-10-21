use super::*;

pub(super) fn process_matches(pid_file: &Path, qmp_socket: &Path) -> bool {
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

pub(super) fn kvm_accessible() -> bool {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .is_ok()
}

pub(super) fn kvm_access_error() -> Option<String> {
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

