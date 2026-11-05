use super::*;

/// Runs a configuration export command on a QEMU device's serial console (for example RouterOS
/// `/export`) and returns what it printed, so devices without a config file can still be exported.
pub(crate) async fn export_node_config(
    State(state): State<AppState>,
    Path((project_id, node_id)): Path<(Uuid, Uuid)>,
    Json(request): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let project_id = project_id.to_string();
    let node_id = node_id.to_string();
    let node = get_topology_object(state.store.pool(), &project_id, "node", &node_id)
        .await?
        .0;
    if node["node_type"].as_str() != Some("qemu") {
        return Err(ApiError::conflict(
            "Serial config export is available for QEMU nodes only",
        ));
    }
    let hint = format!(
        "{} {} {}",
        node["name"].as_str().unwrap_or_default(),
        node["template_name"].as_str().unwrap_or_default(),
        node["hda_disk_image"].as_str().unwrap_or_default()
    )
    .to_lowercase();
    let command = request["command"]
        .as_str()
        .or_else(|| node["config_export_command"].as_str())
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            ["mikrotik", "routeros", "chr"]
                .iter()
                .any(|marker| hint.contains(marker))
                .then(|| "/export".to_owned())
        })
        .ok_or_else(|| {
            ApiError::bad_request(
                "No export command is set. Open Configure device and fill in 'Export config command'.",
            )
        })?;
    let user = node["config_export_username"].as_str().filter(|v| !v.is_empty());
    let password = node["config_export_password"].as_str().unwrap_or_default();
    // MikroTik RouterOS CHR ships with user "admin" and an empty password.
    let credentials = match user {
        Some(user) => Some((user, password)),
        None if hint.contains("mikrotik") || hint.contains("routeros") || hint.contains("chr") => Some(("admin", password)),
        None => None,
    };
    let vm = state.qemu.vm_from_node(&node).map_err(runtime_api_error)?;
    if state.qemu.status(&vm).await.map_err(runtime_api_error)? != "started" {
        return Err(ApiError::conflict("Start the device before exporting its configuration"));
    }
    let unavailable = |error: std::io::Error| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            format!("Could not reach the serial console (close any open console for this device first): {error}"),
        )
    };
    let raw = if vm.console_type == "telnet" {
        let port = vm
            .console_port
            .ok_or_else(|| ApiError::conflict("QEMU telnet console has no port"))?;
        let stream = TcpStream::connect(("127.0.0.1", port)).await.map_err(unavailable)?;
        serial_command_output(stream, &command, credentials).await
    } else {
        let stream = UnixStream::connect(&vm.serial_socket).await.map_err(unavailable)?;
        serial_command_output(stream, &command, credentials).await
    }
    .map_err(|error| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, error))?;
    Ok(Json(json!({"command": command, "config": clean_serial_output(&raw, &command)})))
}

pub(crate) async fn serial_command_output<S>(
    mut stream: S,
    command: &str,
    credentials: Option<(&str, &str)>,
) -> Result<Vec<u8>, String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::time::{Duration, Instant, timeout};
    let mut buffer = [0u8; 4096];
    // Get to a usable prompt: answer login / password / "[Y/n]" questions as they appear.
    let mut seen = Vec::new();
    stream.write_all(b"\r").await.map_err(|e| e.to_string())?;
    for _ in 0..8 {
        seen.clear();
        while let Ok(Ok(read)) = timeout(Duration::from_millis(900), stream.read(&mut buffer)).await {
            if read == 0 {
                break;
            }
            seen.extend_from_slice(&buffer[..read]);
        }
        let tail = strip_terminal_codes(&seen).to_lowercase();
        let tail = tail.trim_end();
        let answer: Option<String> = if tail.ends_with("login:") {
            credentials.map(|(user, _)| format!("{user}\r"))
        } else if tail.ends_with("password:") {
            credentials.map(|(_, password)| format!("{password}\r"))
        } else if tail.ends_with("[y/n]") || tail.ends_with("[y/n]:") || tail.ends_with("(y/n)") {
            Some("n\r".to_owned())
        } else if tail.ends_with('>') || tail.ends_with('#') || tail.ends_with('$') {
            break;
        } else {
            Some("\r".to_owned())
        };
        if let Some(answer) = answer {
            stream.write_all(answer.as_bytes()).await.map_err(|e| e.to_string())?;
        }
    }
    stream
        .write_all(format!("{command}\r").as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    let started = Instant::now();
    let mut output = Vec::new();
    loop {
        let idle = if output.is_empty() { Duration::from_secs(6) } else { Duration::from_millis(2500) };
        match timeout(idle, stream.read(&mut buffer)).await {
            Ok(Ok(0)) | Err(_) => break,
            Ok(Ok(read)) => output.extend_from_slice(&buffer[..read]),
            Ok(Err(error)) => return Err(error.to_string()),
        }
        if started.elapsed() > Duration::from_secs(60) || output.len() > 4 * 1024 * 1024 {
            break;
        }
    }
    if output.is_empty() {
        return Err("The device did not answer on its console. Is it booted and logged in?".into());
    }
    Ok(output)
}

pub(crate) fn strip_terminal_codes(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let mut plain = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => {
                if chars.peek() == Some(&'[') {
                    chars.next();
                    while let Some(n) = chars.next() {
                        if n.is_ascii_alphabetic() || n == '~' {
                            break;
                        }
                    }
                } else {
                    chars.next();
                }
            }
            '\r' => {}
            c => plain.push(c),
        }
    }
    plain
}

pub(crate) fn clean_serial_output(raw: &[u8], command: &str) -> String {
    let plain = strip_terminal_codes(raw);
    let mut lines: Vec<&str> = plain.lines().collect();
    if let Some(index) = lines.iter().position(|line| line.contains(command)) {
        lines.drain(..=index);
    }
    // The last unterminated line is the prompt printed after the output.
    if !plain.ends_with('\n') {
        lines.pop();
    }
    lines.join("\n").trim_end().to_owned() + "\n"
}

#[cfg(test)]
mod serial_export_tests {
    use super::clean_serial_output;

    #[test]
    fn strips_echo_escapes_and_prompt() {
        let raw = b"\x1b[9999B[admin@MikroTik] > /export\r\n# jan/01 by RouterOS 7\r\n/ip address\r\nadd address=10.0.0.1/24\r\n[admin@MikroTik] > ";
        assert_eq!(
            clean_serial_output(raw, "/export"),
            "# jan/01 by RouterOS 7\n/ip address\nadd address=10.0.0.1/24\n"
        );
    }
}
