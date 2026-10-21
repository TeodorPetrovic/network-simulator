use super::*;

impl DynamipsRuntime {
    pub fn prepare_upload_name(&self, file_name: &str) -> Result<(PathBuf, PathBuf), RuntimeError> {
        self.upload_paths(file_name)
    }

    pub(super) fn image_files(&self) -> Result<Vec<PathBuf>, RuntimeError> {
        let directory = match self.image_dir.canonicalize() {
            Ok(directory) => directory,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(RuntimeError::Failed(format!(
                    "Could not read Dynamips image directory '{}': {error}",
                    self.image_dir.display()
                )));
            }
        };
        let entries = fs::read_dir(&directory).map_err(|error| {
            RuntimeError::Failed(format!("Could not list Dynamips IOS images: {error}"))
        })?;
        let mut images = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                RuntimeError::Failed(format!("Could not read Dynamips image entry: {error}"))
            })?;
            let path = entry.path();
            if !path.is_file() || validate_ios_image(&path).is_err() {
                continue;
            }
            images.push(path.canonicalize().map_err(|error| {
                RuntimeError::Failed(format!("Could not resolve IOS image path: {error}"))
            })?);
        }
        images.sort();
        Ok(images)
    }

    pub(super) fn resolve_image(&self, image: &str) -> Result<PathBuf, RuntimeError> {
        let root = self.image_dir.canonicalize().map_err(|error| {
            RuntimeError::Unavailable(format!(
                "Dynamips IOS image directory '{}' is unavailable: {error}",
                self.image_dir.display()
            ))
        })?;
        let path = Path::new(image).canonicalize().map_err(|error| {
            RuntimeError::NotFound(format!("Cisco IOS image '{image}' is unavailable: {error}"))
        })?;
        if !path.starts_with(&root) {
            return Err(RuntimeError::Failed(
                "Cisco IOS images must be inside the configured Dynamips image directory"
                    .to_owned(),
            ));
        }
        validate_ios_image(&path)?;
        Ok(path)
    }

    pub(super) async fn detected_version(&self) -> Result<String, RuntimeError> {
        let mut command = Command::new(&self.binary);
        command
            .arg("-P")
            .arg("none")
            .stdin(Stdio::null())
            .kill_on_drop(true);
        // Dynamips names its log/VM files relative to the process cwd. Keep
        // this probe out of the controller's directory (the repository root in
        // development) even if a build writes a default log for it.
        if fs::create_dir_all(&self.state_dir).is_ok() {
            command.current_dir(&self.state_dir);
        }
        let output = timeout(COMMAND_TIMEOUT, command.output())
            .await
            .map_err(|_| RuntimeError::Unavailable("Dynamips version check timed out".to_owned()))?
            .map_err(|error| {
                RuntimeError::Unavailable(format!(
                    "Could not run Dynamips executable '{}': {error}",
                    self.binary.display()
                ))
            })?;
        // Some distro builds print their version after rejecting the `none`
        // platform used by GNS3's probe, and exit nonzero. Accept only a
        // version banner in that case; the real hypervisor is probed at launch.
        let output = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
            .lines()
            .find(|line| line.contains("Cisco Router Simulation Platform (version "))
            .and_then(|line| line.split("(version ").nth(1))
            .and_then(|value| value.split(')').next())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                RuntimeError::Unavailable(format!(
                    "Could not determine a supported Dynamips version from '{}': {}",
                    self.binary.display(),
                    output.trim()
                ))
            })
    }

    pub(super) async fn version(&self) -> Result<String, RuntimeError> {
        let version = self.detected_version().await?;
        dynamips_version_error(&version).map_or_else(
            || Ok(version.clone()),
            |message| Err(RuntimeError::Unavailable(message)),
        )
    }
}

pub(crate) fn validate_ios_image(path: &Path) -> Result<(), RuntimeError> {
    let mut file = fs::File::open(path).map_err(|error| {
        RuntimeError::NotFound(format!(
            "Cisco IOS image '{}' is unavailable: {error}",
            path.display()
        ))
    })?;
    let mut header = [0; IOS_ELF_HEADER.len()];
    use std::io::Read;
    file.read_exact(&mut header).map_err(|error| {
        RuntimeError::Failed(format!(
            "Cisco IOS image '{}' is too small: {error}",
            path.display()
        ))
    })?;
    if header != IOS_ELF_HEADER {
        return Err(RuntimeError::Failed(format!(
            "'{}' is not a supported Cisco IOS ELF image (Dynamips expects 32-bit big-endian ELF)",
            path.display()
        )));
    }
    Ok(())
}

pub(super) fn safe_file_name(file_name: &str) -> Result<String, RuntimeError> {
    if file_name.contains(['/', '\\']) {
        return Err(RuntimeError::Failed(
            "IOS upload filename must be a single file name".to_owned(),
        ));
    }
    let name = Path::new(file_name)
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| RuntimeError::Failed("IOS upload filename is invalid".to_owned()))?;
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.len() > 180
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(RuntimeError::Failed(
            "IOS filename may contain only letters, numbers, dot, underscore, and hyphen"
                .to_owned(),
        ));
    }
    Ok(name.to_owned())
}

pub(super) fn validate_router_name(name: &str) -> Result<(), RuntimeError> {
    if name.is_empty()
        || name.len() > 63
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(RuntimeError::Failed(
            "Dynamips router names may contain up to 63 letters, numbers, dot, underscore, and hyphen".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn runtime_manifest_contents(
    project_id: &Uuid,
    node_id: &Uuid,
    hypervisor_port: u16,
    console_port: u16,
    aux_port: Option<u16>,
) -> String {
    format!(
        "{project_id}\n{node_id}\n{hypervisor_port}\n{console_port}\n{}\n",
        aux_port.unwrap_or(0)
    )
}

pub(super) fn required_string<'a>(node: &'a Value, key: &str) -> Result<&'a str, RuntimeError> {
    node.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| RuntimeError::Failed(format!("Dynamips node is missing '{key}'")))
}

pub(super) fn parse_id(id: &str) -> Result<Uuid, RuntimeError> {
    Uuid::parse_str(id)
        .map_err(|_| RuntimeError::Failed("Dynamips IDs must be valid UUIDs".to_owned()))
}

pub(super) async fn available_tcp_port() -> Result<u16, RuntimeError> {
    let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not allocate a TCP port: {error}"))
        })?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not read allocated TCP port: {error}"))
        })
}

pub(super) async fn available_udp_port() -> Result<u16, RuntimeError> {
    let socket = tokio::net::UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not allocate a UDP port: {error}"))
        })?;
    socket
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not read allocated UDP port: {error}"))
        })
}
