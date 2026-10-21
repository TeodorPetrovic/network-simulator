use super::*;
use std::fs;
use std::process::Stdio;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    process::Command,
    time::timeout,
};

impl UbridgeRuntime {
    /// Release relays and stop the uBridge daemon owned by this controller.
    pub async fn shutdown(&self) {
        let endpoint_ids = self
            .link_bridges
            .lock()
            .await
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        if UnixStream::connect(&self.socket).await.is_ok() {
            for endpoint_id in endpoint_ids {
                if let Err(error) = self.delete_link_endpoint(&endpoint_id).await {
                    tracing::warn!(error = error.message(), %endpoint_id, "could not detach uBridge endpoint during shutdown");
                }
            }
        }
        self.link_bridges.lock().await.clear();
        self.host_bridge_taps.lock().await.clear();

        let mut daemon = self.daemon.lock().await;
        if let Some(mut child) = daemon.take() {
            if child.try_wait().ok().flatten().is_none() {
                if let Err(error) = child.start_kill() {
                    tracing::warn!(%error, "could not stop controller-owned uBridge daemon");
                }
                if let Err(error) = timeout(Duration::from_secs(3), child.wait()).await {
                    tracing::warn!(%error, "timed out waiting for controller-owned uBridge daemon");
                }
            }
            if let Err(error) = fs::remove_file(&self.socket)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(%error, socket = %self.socket.display(), "could not remove uBridge control socket");
            }
        }
    }

    pub(super) async fn command(&self, command: &str) -> Result<String, RuntimeError> {
        if command.chars().any(char::is_control) {
            return Err(RuntimeError::Failed(
                "uBridge commands cannot contain control characters".to_owned(),
            ));
        }
        self.ensure_started().await?;
        timeout(COMMAND_TIMEOUT, async {
            let stream = UnixStream::connect(&self.socket).await.map_err(|error| {
                RuntimeError::Unavailable(format!(
                    "Could not connect to the uBridge control socket '{}': {error}",
                    self.socket.display()
                ))
            })?;
            let mut stream = BufReader::new(stream);
            stream
                .get_mut()
                .write_all(format!("{command}\n").as_bytes())
                .await
                .map_err(|error| {
                    RuntimeError::Unavailable(format!("Could not send uBridge command: {error}"))
                })?;
            let mut reply = String::new();
            loop {
                let mut line = String::new();
                let read = stream.read_line(&mut line).await.map_err(|error| {
                    RuntimeError::Unavailable(format!("Could not read uBridge response: {error}"))
                })?;
                if read == 0 {
                    return Err(RuntimeError::Unavailable(
                        "uBridge closed its control connection without replying".to_owned(),
                    ));
                }
                if reply.len() + line.len() > MAX_REPLY_BYTES {
                    return Err(RuntimeError::Failed(
                        "uBridge response exceeded the controller limit".to_owned(),
                    ));
                }
                let final_reply = is_final_reply(&line);
                reply.push_str(&line);
                if final_reply {
                    break;
                }
            }
            if reply
                .lines()
                .last()
                .is_some_and(|line| !line.starts_with("100-"))
            {
                if reply.contains("Operation not permitted") || reply.contains("EPERM") {
                    return Err(RuntimeError::Unavailable(format!(
                        "uBridge needs CAP_NET_ADMIN and CAP_NET_RAW for kernel networking: {}",
                        reply.trim()
                    )));
                }
                return Err(RuntimeError::Failed(format!(
                    "uBridge command failed: {}",
                    reply.trim()
                )));
            }
            Ok(reply)
        })
        .await
        .map_err(|_| RuntimeError::Unavailable("uBridge command timed out".to_owned()))?
    }

    async fn ensure_started(&self) -> Result<(), RuntimeError> {
        let mut daemon = self.daemon.lock().await;
        let owned_daemon_running = match daemon.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(None) => true,
                Ok(Some(_)) => {
                    daemon.take();
                    false
                }
                Err(error) => {
                    return Err(RuntimeError::Unavailable(format!(
                        "Could not inspect the uBridge process: {error}"
                    )));
                }
            },
            None => false,
        };
        if owned_daemon_running {
            if UnixStream::connect(&self.socket).await.is_ok() {
                return Ok(());
            }
            wait_for_socket(&self.socket).await?;
            return Ok(());
        }

        if UnixStream::connect(&self.socket).await.is_ok() {
            stop_existing_ubridge(&self.socket).await?;
            self.link_bridges.lock().await.clear();
            self.host_bridge_taps.lock().await.clear();
        }
        if let Some(parent) = self.socket.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                RuntimeError::Failed(format!("Could not create uBridge state directory: {error}"))
            })?;
        }
        match fs::remove_file(&self.socket) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(RuntimeError::Failed(format!(
                    "Could not remove stale uBridge socket: {error}"
                )));
            }
        }
        let child = Command::new(&self.binary)
            .arg("-U")
            .arg(&self.socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| {
                RuntimeError::Unavailable(format!(
                    "Could not start upstream uBridge executable '{}': {error}",
                    self.binary.display()
                ))
            })?;
        *daemon = Some(child);
        wait_for_socket(&self.socket).await
    }
}
