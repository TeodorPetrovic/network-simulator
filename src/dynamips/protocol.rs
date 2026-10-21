use super::*;

impl DynamipsRuntime {
    pub(super) async fn send(&self, port: u16, command: &str) -> Result<Vec<String>, RuntimeError> {
        self.send_with_timeout(port, command, COMMAND_TIMEOUT).await
    }

    pub(super) async fn send_with_timeout(
        &self,
        port: u16,
        command: &str,
        command_timeout: Duration,
    ) -> Result<Vec<String>, RuntimeError> {
        if command.chars().any(char::is_control) {
            return Err(RuntimeError::Failed(
                "Dynamips protocol commands cannot contain control characters".to_owned(),
            ));
        }
        timeout(command_timeout, async {
            let mut stream = TcpStream::connect(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))
                .await
                .map_err(|error| {
                    RuntimeError::Unavailable(format!(
                        "Could not connect to Dynamips hypervisor on 127.0.0.1:{port}: {error}"
                    ))
                })?;
            stream
                .write_all(format!("{command}\n").as_bytes())
                .await
                .map_err(|error| {
                    RuntimeError::Unavailable(format!("Could not send Dynamips command: {error}"))
                })?;
            stream.shutdown().await.ok();
            let mut reader = BufReader::new(stream);
            let mut response = Vec::new();
            let mut total_bytes = 0;
            for _ in 0..MAX_HYPERVISOR_LINES {
                let mut line = String::new();
                let length = reader.read_line(&mut line).await.map_err(|error| {
                    RuntimeError::Unavailable(format!(
                        "Could not read Dynamips response for '{command}': {error}"
                    ))
                })?;
                if length == 0 {
                    return Err(RuntimeError::Unavailable(format!(
                        "Dynamips closed the connection before completing '{command}'"
                    )));
                }
                total_bytes += length;
                if total_bytes > MAX_HYPERVISOR_RESPONSE {
                    return Err(RuntimeError::Failed(
                        "Dynamips hypervisor response exceeded the 256 KiB limit".to_owned(),
                    ));
                }
                let line = line.trim_end_matches(['\r', '\n']);
                if line.starts_with("2") && line.as_bytes().get(3) == Some(&b'-') {
                    return Err(RuntimeError::Failed(format!(
                        "Dynamips rejected '{command}': {}",
                        line.get(4..).unwrap_or(line)
                    )));
                }
                if line.starts_with("100-") {
                    let message = line.get(4..).unwrap_or_default();
                    if message != "OK" && !message.is_empty() {
                        response.push(message.to_owned());
                    }
                    return Ok(response);
                }
                if line.as_bytes().get(3) == Some(&b' ') {
                    response.push(line.get(4..).unwrap_or_default().to_owned());
                } else if !line.is_empty() {
                    response.push(line.to_owned());
                }
            }
            Err(RuntimeError::Failed(format!(
                "Dynamips returned too many lines for '{command}'"
            )))
        })
        .await
        .map_err(|_| RuntimeError::Unavailable(format!("Dynamips command timed out: {command}")))?
    }
}

pub(super) fn quote(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_control() {
            let codepoint = character as u32;
            if codepoint <= 0xff {
                escaped.push_str(&format!("\\x{codepoint:02x}"));
            } else {
                escaped.push_str(&format!("\\u{{{codepoint:x}}}"));
            }
        } else if character == '\\' || character == '"' {
            escaped.push('\\');
            escaped.push(character);
        } else {
            escaped.push(character);
        }
    }
    format!("\"{escaped}\"")
}
