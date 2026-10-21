use super::*;

impl QemuRuntime {
    pub(super) fn platform(&self) -> String {
        self.binary
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("qemu-system-"))
            .filter(|platform| !platform.is_empty())
            .unwrap_or("x86_64")
            .to_owned()
    }

    pub(super) fn image_files(&self) -> Result<Vec<PathBuf>, RuntimeError> {
        let directory = match self.image_dir.canonicalize() {
            Ok(directory) => directory,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(RuntimeError::Failed(format!(
                    "Could not read QEMU image directory '{}': {error}",
                    self.image_dir.display()
                )));
            }
        };
        let entries = fs::read_dir(&directory).map_err(|error| {
            RuntimeError::Failed(format!("Could not list QEMU images: {error}"))
        })?;
        let mut files = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                RuntimeError::Failed(format!("Could not read QEMU image entry: {error}"))
            })?;
            let path = entry.path();
            if !path.is_file() || !is_disk_image(&path) {
                continue;
            }
            let canonical = path.canonicalize().map_err(|error| {
                RuntimeError::Failed(format!("Could not resolve QEMU image path: {error}"))
            })?;
            if canonical.starts_with(&directory) {
                files.push(canonical);
            }
        }
        files.sort();
        Ok(files)
    }

    pub(super) fn resolve_image(&self, image: &str) -> Result<PathBuf, RuntimeError> {
        let root = self.image_dir.canonicalize().map_err(|error| {
            RuntimeError::Unavailable(format!(
                "QEMU image directory '{}' is unavailable: {error}",
                self.image_dir.display()
            ))
        })?;
        let path = Path::new(image).canonicalize().map_err(|error| {
            RuntimeError::NotFound(format!("QEMU disk image '{image}' is unavailable: {error}"))
        })?;
        if !path.starts_with(&root) || !path.is_file() || !is_disk_image(&path) {
            return Err(RuntimeError::Failed(
                "QEMU disk images must be supported image files inside the configured image directory".to_owned(),
            ));
        }
        Ok(path)
    }

    pub(super) fn resolve_node_image(&self, image: &str, vm_dir: &Path) -> Result<PathBuf, RuntimeError> {
        let candidate = Path::new(image);
        let candidate = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            let node_candidate = vm_dir.join(candidate);
            if node_candidate.exists() {
                node_candidate
            } else {
                self.image_dir.join(candidate)
            }
        };
        let path = candidate.canonicalize().map_err(|error| {
            RuntimeError::NotFound(format!("QEMU disk image '{image}' is unavailable: {error}"))
        })?;
        let image_root = self.image_dir.canonicalize().map_err(|error| {
            RuntimeError::Unavailable(format!(
                "QEMU image directory '{}' is unavailable: {error}",
                self.image_dir.display()
            ))
        })?;
        let node_root = vm_dir.canonicalize().map_err(|error| {
            RuntimeError::Unavailable(format!("QEMU node directory is unavailable: {error}"))
        })?;
        if !(path.is_file()
            && is_disk_image(&path)
            && (path.starts_with(&image_root) || path.starts_with(&node_root)))
        {
            return Err(RuntimeError::Failed(
                "QEMU disk images must be supported image files inside the configured image directory or node directory".to_owned(),
            ));
        }
        Ok(path)
    }

    pub(super) fn resolve_auxiliary_file(&self, file: &str) -> Result<PathBuf, RuntimeError> {
        let root = self.image_dir.canonicalize().map_err(|error| {
            RuntimeError::Unavailable(format!(
                "QEMU image directory '{}' is unavailable: {error}",
                self.image_dir.display()
            ))
        })?;
        let candidate = Path::new(file);
        let candidate = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            root.join(candidate)
        };
        let path = candidate.canonicalize().map_err(|error| {
            RuntimeError::NotFound(format!(
                "QEMU support file '{file}' is unavailable: {error}"
            ))
        })?;
        if !path.starts_with(&root) || !path.is_file() {
            return Err(RuntimeError::Failed(
                "QEMU support files must be regular files inside the configured image directory"
                    .to_owned(),
            ));
        }
        Ok(path)
    }

    pub(super) fn resolve_optional_auxiliary_file(
        &self,
        node: &Value,
        key: &str,
    ) -> Result<Option<PathBuf>, RuntimeError> {
        node[key]
            .as_str()
            .filter(|path| !path.trim().is_empty())
            .map(|path| self.resolve_auxiliary_file(path))
            .transpose()
    }

    pub(super) fn config_disk_image(&self) -> Result<PathBuf, RuntimeError> {
        let configured = std::env::var_os("NETWORK_SIMULATOR_QEMU_CONFIG_DISK")
            .map(PathBuf::from)
            .unwrap_or_else(|| self.image_dir.join("config.img"));
        let path = configured.canonicalize().map_err(|error| {
            RuntimeError::Unavailable(format!(
                "QEMU config disk is unavailable at '{}': {error}",
                configured.display()
            ))
        })?;
        if !path.is_file() {
            return Err(RuntimeError::Unavailable(format!(
                "QEMU config disk is not a regular file: '{}'",
                path.display()
            )));
        }
        Ok(path)
    }

    pub(super) async fn image_format(&self, image: &Path) -> Result<String, RuntimeError> {
        let output = self
            .command(
                &self.qemu_img,
                [
                    "info",
                    "--output=json",
                    image.to_str().ok_or_else(|| {
                        RuntimeError::Failed("QEMU image path is not valid UTF-8".to_owned())
                    })?,
                ],
                COMMAND_TIMEOUT,
            )
            .await?;
        let info: ImageInfo = serde_json::from_str(&output).map_err(|error| {
            RuntimeError::Failed(format!("qemu-img returned invalid image metadata: {error}"))
        })?;
        if info.format.is_empty() || !info.format.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
            return Err(RuntimeError::Failed(
                "qemu-img returned an invalid disk format".to_owned(),
            ));
        }
        Ok(info.format)
    }

    pub(super) async fn create_overlay(
        &self,
        image: &Path,
        source_format: &str,
        overlay: &Path,
    ) -> Result<(), RuntimeError> {
        if overlay.exists() {
            return Ok(());
        }
        self.command(
            &self.qemu_img,
            [
                "create",
                "-f",
                "qcow2",
                "-F",
                source_format,
                "-b",
                image.to_str().ok_or_else(|| {
                    RuntimeError::Failed("QEMU image path is not valid UTF-8".to_owned())
                })?,
                overlay.to_str().ok_or_else(|| {
                    RuntimeError::Failed("QEMU overlay path is not valid UTF-8".to_owned())
                })?,
            ],
            COMMAND_TIMEOUT,
        )
        .await?;
        Ok(())
    }

}
