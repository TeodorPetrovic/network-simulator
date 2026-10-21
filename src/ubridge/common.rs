use super::START_TIMEOUT;
use crate::runtime::RuntimeError;
use std::{
    fs,
    net::{Ipv4Addr, SocketAddrV4},
    path::PathBuf,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UdpSocket, UnixStream},
    time::{sleep, timeout},
};

pub(super) fn is_final_reply(line: &str) -> bool {
    line.as_bytes().get(3) == Some(&b'-')
}

pub(super) async fn wait_for_socket(socket: &PathBuf) -> Result<(), RuntimeError> {
    let deadline = tokio::time::Instant::now() + START_TIMEOUT;
    loop {
        if UnixStream::connect(socket).await.is_ok() {
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(RuntimeError::Unavailable(format!(
                "uBridge did not create its control socket '{}'; mixed-runtime links need upstream uBridge and CAP_NET_ADMIN/CAP_NET_RAW",
                socket.display()
            )));
        }
        sleep(Duration::from_millis(40)).await;
    }
}

/// Stop a uBridge process left behind by a previous controller instance. The
/// old process has no child handle here, so rebuilding requires asking the
/// daemon itself to stop before launching a fresh process.
pub(super) async fn stop_existing_ubridge(socket: &PathBuf) -> Result<(), RuntimeError> {
    let mut stream = UnixStream::connect(socket).await.map_err(|error| {
        RuntimeError::Unavailable(format!(
            "Could not connect to the existing uBridge socket '{}': {error}",
            socket.display()
        ))
    })?;
    stream
        .write_all(b"hypervisor stop\n")
        .await
        .map_err(|error| {
            RuntimeError::Unavailable(format!(
                "Could not stop the existing uBridge daemon: {error}"
            ))
        })?;
    let _ = timeout(Duration::from_secs(2), async {
        let mut reader = BufReader::new(stream);
        let mut reply = String::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).await? == 0 {
                break;
            }
            reply.push_str(&line);
            if is_final_reply(&line) {
                break;
            }
        }
        Ok::<(), std::io::Error>(())
    })
    .await;

    let deadline = tokio::time::Instant::now() + START_TIMEOUT;
    loop {
        if UnixStream::connect(socket).await.is_err() {
            match fs::remove_file(socket) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(RuntimeError::Failed(format!(
                        "Could not remove stopped uBridge socket: {error}"
                    )));
                }
            }
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(RuntimeError::Unavailable(
                "The existing uBridge daemon did not stop during startup recovery".to_owned(),
            ));
        }
        sleep(Duration::from_millis(40)).await;
    }
}

pub(super) fn validate_interface_name(name: &str) -> Result<(), RuntimeError> {
    if name.is_empty()
        || name.len() > 15
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(RuntimeError::Failed(
            "uBridge interface names must contain 1–15 safe ASCII characters".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn validate_remote_host(host: &str) -> Result<(), RuntimeError> {
    if host.is_empty()
        || host.len() > 255
        || !host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b':' | b'%'))
    {
        return Err(RuntimeError::Failed(
            "Cloud UDP remote host must be an IP address or safe hostname".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn filter_value(value: &serde_json::Value) -> Result<String, RuntimeError> {
    match value {
        serde_json::Value::Number(number) => Ok(number.to_string()),
        serde_json::Value::String(text) if !text.chars().any(char::is_control) => {
            Ok(format!("\"{}\"", escape_ubridge_arg(text)))
        }
        _ => Err(RuntimeError::Failed(
            "Link filter values must be numbers or single-line text".to_owned(),
        )),
    }
}

pub(super) fn validate_filter_values(
    kind: &str,
    values: &[serde_json::Value],
) -> Result<(), RuntimeError> {
    let invalid = || RuntimeError::Failed(format!("Link filter '{kind}' has invalid values"));
    let integer = |index: usize, min: i64, max: i64| {
        values
            .get(index)
            .and_then(|value| match value {
                serde_json::Value::Number(number) => number.as_i64(),
                serde_json::Value::String(text) => text.trim().parse::<i64>().ok(),
                _ => None,
            })
            .filter(|value| (min..=max).contains(value))
    };
    match kind {
        "frequency_drop" if values.len() == 1 && integer(0, -1, 32767).is_some() => Ok(()),
        "packet_loss" | "corrupt" if values.len() == 1 && integer(0, 0, 100).is_some() => Ok(()),
        "delay"
            if (1..=3).contains(&values.len())
                && integer(0, 1, 32767).is_some()
                && (values.len() < 2 || integer(1, 0, 32767).is_some())
                && (values.len() < 3
                    || values[2].as_str().is_some_and(|distribution| {
                        matches!(
                            distribution.trim().to_ascii_lowercase().as_str(),
                            "" | "uniform" | "normal" | "pareto" | "paretonormal"
                        )
                    })) =>
        {
            Ok(())
        }
        "bpf"
            if values.len() == 1
                && values[0].as_str().is_some_and(|expression| {
                    expression.len() <= 4096 && !expression.chars().any(char::is_control)
                }) =>
        {
            Ok(())
        }
        _ => Err(invalid()),
    }
}

pub(super) fn escape_ubridge_arg(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_control() {
            escaped.push_str(&format!("%{:02X}", character as u32));
        } else {
            if matches!(character, '\\' | '"') {
                escaped.push('\\');
            }
            escaped.push(character);
        }
    }
    escaped
}

pub fn host_veth_name(endpoint_id: &str) -> Result<String, RuntimeError> {
    Ok(format!("nsv{}", compact_id(endpoint_id)?))
}

#[cfg(test)]
pub fn guest_veth_name(endpoint_id: &str) -> Result<String, RuntimeError> {
    Ok(format!("nsg{}", compact_id(endpoint_id)?))
}

pub(super) fn is_linux_bridge_interface(interface: &str) -> bool {
    cfg!(target_os = "linux")
        && std::path::Path::new("/sys/class/net")
            .join(interface)
            .join("bridge")
            .exists()
}

pub(super) fn endpoint_name(endpoint_id: &str) -> Result<String, RuntimeError> {
    Ok(format!("nse{}", compact_id(endpoint_id)?))
}

pub(super) fn compact_id(value: &str) -> Result<String, RuntimeError> {
    let compact = value
        .chars()
        .filter(|character| character.is_ascii_hexdigit())
        .collect::<String>();
    if compact.len() < 8 {
        return Err(RuntimeError::Failed(
            "Endpoint ID must contain at least eight hexadecimal characters".to_owned(),
        ));
    }
    Ok(compact[..12.min(compact.len())].to_owned())
}

pub(super) async fn available_udp_port() -> Result<u16, RuntimeError> {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not allocate a uBridge UDP port: {error}"))
        })?;
    socket
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not inspect the uBridge UDP port: {error}"))
        })
}
