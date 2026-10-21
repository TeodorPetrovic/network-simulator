use super::*;

pub(super) async fn qmp_execute_inner(
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

pub(super) async fn send_qmp_command(
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

