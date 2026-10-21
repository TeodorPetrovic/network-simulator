use super::*;

impl DockerRuntime {
    pub fn new() -> Self {
        Self::with_binary(
            std::env::var_os("NETWORK_SIMULATOR_DOCKER")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("docker")),
        )
    }

    pub fn with_binary(binary: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            state_dir: std::env::var_os("NETWORK_SIMULATOR_DOCKER_STATE")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("data/docker")),
            socket: docker_socket_path(),
            console_hosts: std::sync::Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    #[cfg(test)]
    pub fn with_paths(binary: impl Into<PathBuf>, state_dir: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            state_dir: state_dir.into(),
            socket: PathBuf::from("/var/run/docker.sock"),
            console_hosts: std::sync::Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    #[cfg(test)]
    pub fn with_paths_and_socket(
        binary: impl Into<PathBuf>,
        state_dir: impl Into<PathBuf>,
        socket: impl Into<PathBuf>,
    ) -> Self {
        Self {
            binary: binary.into(),
            state_dir: state_dir.into(),
            socket: socket.into(),
            console_hosts: std::sync::Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    pub(super) fn managed_container_marker(
        &self,
        container_id: &str,
    ) -> Result<PathBuf, RuntimeError> {
        if !valid_container_id(container_id) {
            return Err(RuntimeError::Failed(
                "Docker returned an invalid container ID".to_owned(),
            ));
        }
        Ok(self
            .state_dir
            .join("managed-containers")
            .join(container_id.to_ascii_lowercase()))
    }

    pub(super) fn mark_container_managed(
        &self,
        project_id: &str,
        node_id: &str,
        container_id: &str,
    ) -> Result<(), RuntimeError> {
        let marker = self.managed_container_marker(container_id)?;
        if marker.exists() {
            return self.ensure_managed_container(container_id);
        }
        let directory = marker.parent().expect("managed marker has a parent");
        fs::create_dir_all(directory).map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not create Docker runtime state directory: {error}"
            ))
        })?;
        let contents = format!("{project_id}\n{node_id}\n");
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker)
            .and_then(|mut file| std::io::Write::write_all(&mut file, contents.as_bytes()))
        {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                self.ensure_managed_container(container_id)
            }
            Err(error) => Err(RuntimeError::Failed(format!(
                "Could not record Docker runtime ownership: {error}"
            ))),
        }
    }

    pub(super) fn ensure_managed_container(&self, container_id: &str) -> Result<(), RuntimeError> {
        let marker = self.managed_container_marker(container_id)?;
        let metadata = fs::symlink_metadata(&marker).map_err(|_| {
            RuntimeError::Failed(
                "Docker container ID is not owned by this controller runtime".to_owned(),
            )
        })?;
        if !metadata.file_type().is_file() {
            return Err(RuntimeError::Failed(
                "Docker runtime ownership marker is invalid".to_owned(),
            ));
        }
        let contents = fs::read_to_string(marker).map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not read Docker runtime ownership marker: {error}"
            ))
        })?;
        let mut lines = contents.lines();
        let project = lines.next().unwrap_or_default();
        let node = lines.next().unwrap_or_default();
        if Uuid::parse_str(project).is_err()
            || Uuid::parse_str(node).is_err()
            || lines.next().is_some()
        {
            return Err(RuntimeError::Failed(
                "Docker runtime ownership marker is invalid".to_owned(),
            ));
        }
        Ok(())
    }

    pub(super) fn forget_managed_container(&self, container_id: &str) {
        // Docker IDs are immutable and never recycled. Keeping this small
        // record allows the controller to recognize stale IDs after a restart.
        let _ = container_id;
    }

    pub(super) async fn command<I, S>(
        &self,
        args: I,
        duration: Duration,
    ) -> Result<String, RuntimeError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let mut command = Command::new(&self.binary);
        command.args(args).stdin(Stdio::null()).kill_on_drop(true);
        let output = timeout(duration, command.output())
            .await
            .map_err(|_| RuntimeError::Unavailable("Docker command timed out".to_owned()))?
            .map_err(|error| {
                RuntimeError::Unavailable(format!(
                    "Could not run Docker CLI '{}': {error}",
                    self.binary.display()
                ))
            })?;
        if output.status.success() {
            return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if stderr.contains("Cannot connect to the Docker daemon")
            || stderr.contains("Is the docker daemon running")
            || stderr.contains("permission denied while trying to connect")
        {
            return Err(RuntimeError::Unavailable(stderr));
        }
        Err(RuntimeError::Failed(if stderr.is_empty() {
            format!("Docker command exited with {}", output.status)
        } else {
            stderr
        }))
    }
}
