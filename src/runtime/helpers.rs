use super::*;

pub(super) fn parse_environment(environment: &str) -> Result<Vec<String>, RuntimeError> {
    let mut parsed = Vec::new();
    for line in environment
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let (key, value) = line.split_once('=').ok_or_else(|| {
            RuntimeError::Failed(format!(
                "Docker environment entry '{line}' must use KEY=VALUE"
            ))
        })?;
        if key.is_empty()
            || !key.bytes().enumerate().all(|(index, byte)| {
                if index == 0 {
                    byte.is_ascii_alphabetic() || byte == b'_'
                } else {
                    byte.is_ascii_alphanumeric() || byte == b'_'
                }
            })
            || line.contains('\0')
        {
            return Err(RuntimeError::Failed(format!(
                "Docker environment variable name '{key}' is invalid"
            )));
        }
        parsed.push(format!("{key}={value}"));
    }
    Ok(parsed)
}

pub(super) fn parse_extra_hosts(extra_hosts: &str) -> Result<Vec<String>, RuntimeError> {
    let mut parsed = Vec::new();
    for line in extra_hosts
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let (host, address) = line.split_once(':').ok_or_else(|| {
            RuntimeError::Failed(format!("Docker extra host '{line}' must use host:address"))
        })?;
        let host_valid = !host.is_empty()
            && host.len() <= 253
            && host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'));
        let address_valid = !address.is_empty()
            && address.len() <= 128
            && address.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b':' | b'-' | b'_')
            });
        if !host_valid || !address_valid {
            return Err(RuntimeError::Failed(format!(
                "Docker extra host '{line}' contains an invalid host or address"
            )));
        }
        parsed.push(line.to_owned());
    }
    Ok(parsed)
}

pub(super) fn validate_container_volume(path: &str) -> Result<PathBuf, RuntimeError> {
    let path = Path::new(path);
    if !path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
        || path.to_string_lossy().len() > 512
    {
        return Err(RuntimeError::Failed(
            "Docker persistent volumes must be safe absolute container paths".to_owned(),
        ));
    }
    Ok(path
        .strip_prefix("/")
        .map_err(|_| RuntimeError::Failed("Invalid Docker container path".to_owned()))?
        .to_path_buf())
}

pub(super) fn default_docker_console_type() -> String {
    "telnet".to_owned()
}

pub(super) fn docker_socket_path() -> PathBuf {
    std::env::var_os("DOCKER_HOST")
        .and_then(|host| host.into_string().ok())
        .and_then(|host| host.strip_prefix("unix://").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/var/run/docker.sock"))
}

pub(super) fn default_docker_console_resolution() -> String {
    "1024x768".to_owned()
}

pub(super) fn default_docker_aux_type() -> String {
    "none".to_owned()
}

pub(super) async fn available_tcp_port() -> Result<u16, RuntimeError> {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not allocate a TCP port: {error}"))
        })?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not inspect an allocated TCP port: {error}"))
        })
}

pub(super) fn runtime_program(variable: &str, name: &str) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(variable).map(PathBuf::from) {
        return path.is_file().then_some(path);
    }
    if name.contains('/') {
        let path = PathBuf::from(name);
        return path.is_file().then_some(path);
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|directory| directory.join(name))
        .find(|path| path.is_file())
}

pub(super) fn docker_aux_exec_command(
    binary: &Path,
    container_id: &str,
) -> Result<String, RuntimeError> {
    let binary = binary.to_string_lossy();
    let docker = shlex::try_quote(&binary)
        .map_err(|_| RuntimeError::Failed("Docker CLI path cannot be quoted safely".to_owned()))?;
    Ok(format!(
        "{docker} exec -i -t {container_id} /gns3/bin/busybox sh -c 'while true; do TERM=vt100 /gns3/bin/busybox sh; done'"
    ))
}

pub(super) async fn proxy_docker_http(
    client: TcpStream,
    docker: &Path,
    container_id: &str,
    target_port: u16,
    console_port: u16,
) -> Result<(), RuntimeError> {
    let target_port_text = target_port.to_string();
    let mut child = Command::new(docker)
        .args([
            "exec",
            "-i",
            container_id,
            "/gns3/bin/busybox",
            "nc",
            "127.0.0.1",
            target_port_text.as_str(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| {
            RuntimeError::Unavailable(format!(
                "Could not start the Docker web-console bridge: {error}"
            ))
        })?;
    let (Some(mut stdin), Some(mut stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return Err(RuntimeError::Failed(
            "Docker web-console bridge did not provide input and output streams".to_owned(),
        ));
    };
    let client_host = client
        .peer_addr()
        .map(|address| address.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_owned());
    let (mut client_reader, mut client_writer) = tokio::io::split(client);
    let input = tokio::io::copy(&mut client_reader, &mut stdin);
    let output = async {
        let host_replace = format!("://{client_host}").into_bytes();
        let port_replace = format!(":{target_port}").into_bytes();
        let public_port = format!(":{console_port}").into_bytes();
        let mut buffer = [0u8; 4096];
        loop {
            let length = stdout.read(&mut buffer).await?;
            if length == 0 {
                return Ok::<(), std::io::Error>(());
            }
            let mut chunk = buffer[..length].to_vec();
            replace_bytes(&mut chunk, b"://127.0.0.1", &host_replace);
            replace_bytes(&mut chunk, &port_replace, &public_port);
            client_writer.write_all(&chunk).await?;
        }
    };
    tokio::pin!(input, output);
    let bridge_result = tokio::select! {
        result = &mut output => result,
        input_result = &mut input => match input_result {
            Ok(_) => output.await,
            Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => output.await,
            Err(error) => return Err(RuntimeError::Unavailable(format!("Docker web-console request stream failed: {error}"))),
        },
    };
    bridge_result.map_err(|error| {
        RuntimeError::Unavailable(format!(
            "Docker web-console response stream failed: {error}"
        ))
    })?;
    let _ = child.kill().await;
    Ok(())
}

pub(super) fn replace_bytes(contents: &mut Vec<u8>, from: &[u8], to: &[u8]) {
    if from.is_empty() {
        return;
    }
    let mut output = Vec::with_capacity(contents.len());
    let mut index = 0;
    while index < contents.len() {
        if contents[index..].starts_with(from) {
            output.extend_from_slice(to);
            index += from.len();
        } else {
            output.push(contents[index]);
            index += 1;
        }
    }
    *contents = output;
}

pub(super) fn docker_vnc_tools_error(has_xvfb: bool, has_x11vnc: bool) -> Option<RuntimeError> {
    (!has_xvfb || !has_x11vnc).then(|| {
        RuntimeError::Unavailable(
            "Docker VNC console is unsupported: Xvfb and x11vnc must be installed".to_owned(),
        )
    })
}

pub(super) fn validate_docker_runtime_options(
    options: &DockerTemplateOptions,
) -> Result<(), RuntimeError> {
    if !matches!(
        options.console_type.as_str(),
        "none" | "telnet" | "vnc" | "http" | "https"
    ) {
        return Err(RuntimeError::Failed(
            "Docker console_type must be none, telnet, vnc, http, or https".to_owned(),
        ));
    }
    if !matches!(options.aux_type.as_str(), "none" | "telnet" | "ssh") {
        return Err(RuntimeError::Failed(
            "Docker aux_type must be none, telnet, or ssh".to_owned(),
        ));
    }
    let Some((width, height)) = options.console_resolution.split_once('x') else {
        return Err(RuntimeError::Failed(
            "Docker console_resolution must use WIDTHxHEIGHT".to_owned(),
        ));
    };
    if width.is_empty()
        || height.is_empty()
        || !width.bytes().all(|byte| byte.is_ascii_digit())
        || !height.bytes().all(|byte| byte.is_ascii_digit())
        || width
            .parse::<u16>()
            .ok()
            .filter(|value| *value > 0)
            .is_none()
        || height
            .parse::<u16>()
            .ok()
            .filter(|value| *value > 0)
            .is_none()
    {
        return Err(RuntimeError::Failed(
            "Docker console_resolution must use positive numeric WIDTHxHEIGHT values".to_owned(),
        ));
    }
    if options.console_http_port == Some(0) {
        return Err(RuntimeError::Failed(
            "Docker console_http_port must be between 1 and 65535".to_owned(),
        ));
    }
    if options.console_http_path.as_deref().is_some_and(|path| {
        !path.starts_with('/') || path.bytes().any(|byte| byte.is_ascii_control())
    }) {
        return Err(RuntimeError::Failed(
            "Docker console_http_path must be an absolute URL path".to_owned(),
        ));
    }
    validate_docker_extra_configs(
        &options.extra_configs,
        options.startup_config_content.as_deref(),
    )?;
    let mut adapter_numbers = std::collections::HashSet::new();
    for adapter in &options.custom_adapters {
        if adapter.adapter_number > 99 || !adapter_numbers.insert(adapter.adapter_number) {
            return Err(RuntimeError::Failed(
                "Docker custom adapter numbers must be unique values from 0 to 99".to_owned(),
            ));
        }
        if adapter
            .mac_address
            .as_deref()
            .is_some_and(|address| !valid_mac_address(address))
        {
            return Err(RuntimeError::Failed(format!(
                "Docker custom adapter {} has an invalid MAC address",
                adapter.adapter_number
            )));
        }
    }
    Ok(())
}

pub fn validate_docker_extra_configs(
    configs: &[DockerExtraConfig],
    startup_config_content: Option<&str>,
) -> Result<(), RuntimeError> {
    if configs.len() > 64 {
        return Err(RuntimeError::Failed(
            "Docker extra_configs may contain at most 64 files".to_owned(),
        ));
    }
    let mut targets = std::collections::HashSet::<PathBuf>::new();
    let mut total_bytes = 0_usize;
    for config in configs {
        let target = validate_extra_config_target(&config.target)?;
        if targets
            .iter()
            .any(|existing| existing.starts_with(&target) || target.starts_with(existing))
        {
            return Err(RuntimeError::Failed(format!(
                "Docker extra_configs contains overlapping target '{}'",
                config.target
            )));
        }
        targets.insert(target);
        if config.content.len() > 1024 * 1024 {
            return Err(RuntimeError::Failed(
                "Docker extra config content must be 1 MiB or smaller".to_owned(),
            ));
        }
        total_bytes = total_bytes.saturating_add(config.content.len());
    }
    if let Some(content) = startup_config_content {
        if content.len() > 1024 * 1024 {
            return Err(RuntimeError::Failed(
                "Docker startup_config_content must be 1 MiB or smaller".to_owned(),
            ));
        }
        if !content.is_empty() {
            let startup_target = PathBuf::from("startup-config");
            if targets.iter().any(|target| {
                target.starts_with(&startup_target) || startup_target.starts_with(target)
            }) {
                return Err(RuntimeError::Failed(
                    "Docker startup_config_content conflicts with extra_configs target /startup-config"
                        .to_owned(),
                ));
            }
            targets.insert(startup_target);
        }
        total_bytes = total_bytes.saturating_add(content.len());
    }
    if total_bytes > 4 * 1024 * 1024 {
        return Err(RuntimeError::Failed(
            "Docker injected config content must total 4 MiB or less".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn validate_extra_config_target(target: &str) -> Result<PathBuf, RuntimeError> {
    if !target.starts_with('/')
        || target.ends_with('/')
        || target.contains('\\')
        || target.bytes().any(|byte| byte.is_ascii_control())
        || target.len() > 1024
    {
        return Err(RuntimeError::Failed(format!(
            "Docker extra config target '{}' must be an absolute file path",
            target
        )));
    }
    let mut relative = PathBuf::new();
    for component in Path::new(target).components() {
        match component {
            std::path::Component::RootDir => {}
            std::path::Component::Normal(part) => relative.push(part),
            _ => {
                return Err(RuntimeError::Failed(format!(
                    "Docker extra config target '{}' contains an unsafe path component",
                    target
                )));
            }
        }
    }
    if relative.as_os_str().is_empty() {
        return Err(RuntimeError::Failed(
            "Docker extra config target must name a file".to_owned(),
        ));
    }
    Ok(relative)
}

pub(super) fn valid_container_id(container_id: &str) -> bool {
    (12..=64).contains(&container_id.len())
        && container_id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn validate_docker_topology_id(value: &str, kind: &str) -> Result<String, RuntimeError> {
    Uuid::parse_str(value)
        .map(|id| id.to_string())
        .map_err(|_| RuntimeError::Failed(format!("Docker {kind} ID must be a valid UUID")))
}

pub(super) fn valid_mac_address(value: &str) -> bool {
    let octets = value.split(':').collect::<Vec<_>>();
    octets.len() == 6
        && octets
            .iter()
            .all(|octet| octet.len() == 2 && octet.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

pub(super) fn validate_docker_image_reference(image: &str) -> Result<(), RuntimeError> {
    if image.trim().is_empty()
        || image.len() > 512
        || image.bytes().any(|byte| {
            byte.is_ascii_whitespace()
                || byte.is_ascii_control()
                || matches!(byte, b'\\' | b'`' | b'\'' | b'"')
        })
    {
        return Err(RuntimeError::Failed(
            "Docker image reference is invalid".to_owned(),
        ));
    }
    Ok(())
}
