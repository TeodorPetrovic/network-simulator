use super::*;

impl QemuRuntime {
    pub fn prepare_upload_name(&self, file_name: &str) -> Result<(PathBuf, PathBuf), RuntimeError> {
        let safe_name = safe_disk_image_name(file_name)?;
        fs::create_dir_all(&self.image_dir).map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not create QEMU image directory '{}': {error}",
                self.image_dir.display()
            ))
        })?;
        let directory = self.image_dir.canonicalize().map_err(|error| {
            RuntimeError::Failed(format!("Could not resolve QEMU image directory: {error}"))
        })?;
        let final_path = directory.join(&safe_name);
        if final_path.exists() {
            return Err(RuntimeError::Failed(format!(
                "A QEMU image named '{safe_name}' already exists"
            )));
        }
        let temporary_path = directory.join(format!(".{}.{}.upload", Uuid::new_v4(), safe_name));
        Ok((temporary_path, final_path))
    }

    pub async fn finalize_uploaded_image(
        &self,
        temporary_path: &Path,
        final_path: &Path,
    ) -> Result<PathBuf, RuntimeError> {
        let metadata = fs::metadata(temporary_path).map_err(|error| {
            RuntimeError::Failed(format!("Could not read uploaded QEMU image: {error}"))
        })?;
        if metadata.len() == 0 {
            return Err(RuntimeError::Failed(
                "QEMU image upload is empty".to_owned(),
            ));
        }
        let output = self
            .command(
                &self.qemu_img,
                [
                    "info",
                    "--output=json",
                    temporary_path.to_str().ok_or_else(|| {
                        RuntimeError::Failed("QEMU image path is not valid UTF-8".to_owned())
                    })?,
                ],
                COMMAND_TIMEOUT,
            )
            .await?;
        let info: Value = serde_json::from_str(&output).map_err(|error| {
            RuntimeError::Failed(format!("qemu-img returned invalid image metadata: {error}"))
        })?;
        let format = info["format"].as_str().unwrap_or_default();
        if !image_format_matches(final_path, format) {
            return Err(RuntimeError::Failed(format!(
                "QEMU image extension does not match detected format '{format}'"
            )));
        }
        fs::hard_link(temporary_path, final_path).map_err(|error| {
            RuntimeError::Failed(format!("Could not install QEMU image: {error}"))
        })?;
        fs::remove_file(temporary_path).map_err(|error| {
            RuntimeError::Failed(format!("Could not finalize QEMU image upload: {error}"))
        })?;
        final_path.canonicalize().map_err(|error| {
            RuntimeError::Failed(format!("Could not resolve installed QEMU image: {error}"))
        })
    }

    pub async fn create_disk_image(
        &self,
        project_id: &str,
        node_id: &str,
        file_name: &str,
        format: &str,
        size_mb: u64,
        image_options: &std::collections::BTreeMap<String, String>,
    ) -> Result<PathBuf, RuntimeError> {
        self.create_disk_image_with_size(
            project_id,
            node_id,
            file_name,
            format,
            &format!("{size_mb}M"),
            image_options,
        )
        .await
    }

    pub async fn create_disk_image_with_size(
        &self,
        project_id: &str,
        node_id: &str,
        file_name: &str,
        format: &str,
        size: &str,
        image_options: &std::collections::BTreeMap<String, String>,
    ) -> Result<PathBuf, RuntimeError> {
        let project_id = parse_id(project_id)?;
        let node_id = parse_id(node_id)?;
        let file_name = safe_working_disk_name(file_name)?;
        if !matches!(format, "qcow2" | "qcow" | "raw" | "vmdk" | "vdi") {
            return Err(RuntimeError::Failed(
                "Unsupported QEMU disk format".to_owned(),
            ));
        }
        if !valid_qemu_disk_size(size) {
            return Err(RuntimeError::Failed("Invalid QEMU disk size".to_owned()));
        }
        let directory = self
            .state_dir
            .join(project_id.to_string())
            .join(node_id.to_string());
        fs::create_dir_all(&directory).map_err(|error| {
            RuntimeError::Failed(format!("Could not create QEMU node directory: {error}"))
        })?;
        if process_matches(
            &directory.join(format!("{node_id}.pid")),
            &self.qmp_dir.join(format!("{node_id}.sock")),
        ) {
            return Err(RuntimeError::Failed(
                "Cannot create a QEMU disk while its VM is running".to_owned(),
            ));
        }
        let target = directory.join(file_name);
        if target.exists() {
            return Err(RuntimeError::Failed(format!(
                "QEMU disk image '{}' already exists",
                target.file_name().unwrap_or_default().to_string_lossy()
            )));
        }
        let mut args = vec!["create".to_owned(), "-f".to_owned(), format.to_owned()];
        if !image_options.is_empty() {
            let options = image_options
                .iter()
                .map(|(key, value)| {
                    if key.is_empty()
                        || !key
                            .bytes()
                            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                        || value
                            .bytes()
                            .any(|byte| byte.is_ascii_control() || byte == b',')
                    {
                        return Err(RuntimeError::Failed(
                            "QEMU disk image options contain an invalid key or value".to_owned(),
                        ));
                    }
                    Ok(format!("{key}={value}"))
                })
                .collect::<Result<Vec<_>, _>>()?
                .join(",");
            args.extend(["-o".to_owned(), options]);
        }
        args.extend([
            target
                .to_str()
                .ok_or_else(|| {
                    RuntimeError::Failed("QEMU disk path is not valid UTF-8".to_owned())
                })?
                .to_owned(),
            size.to_owned(),
        ]);
        self.command(&self.qemu_img, args, COMMAND_TIMEOUT).await?;
        Ok(target)
    }

    pub async fn resize_disk_image(
        &self,
        project_id: &str,
        node_id: &str,
        file_name: &str,
        extend_mb: u64,
    ) -> Result<PathBuf, RuntimeError> {
        let project_id = parse_id(project_id)?;
        let node_id = parse_id(node_id)?;
        let file_name = safe_working_disk_name(file_name)?;
        if extend_mb == 0 || extend_mb > 1_000_000 {
            return Err(RuntimeError::Failed(
                "QEMU disk resize must add from 1 to 1000000 MB".to_owned(),
            ));
        }
        let directory = self
            .state_dir
            .join(project_id.to_string())
            .join(node_id.to_string());
        let target = directory.join(file_name);
        let root = directory.canonicalize().map_err(|error| {
            RuntimeError::NotFound(format!("QEMU node directory is unavailable: {error}"))
        })?;
        let canonical = target.canonicalize().map_err(|error| {
            RuntimeError::NotFound(format!("QEMU disk image is unavailable: {error}"))
        })?;
        if !canonical.starts_with(&root) || !canonical.is_file() {
            return Err(RuntimeError::Failed(
                "QEMU disk image must be inside the node directory".to_owned(),
            ));
        }
        let pid_file = directory.join(format!("{node_id}.pid"));
        let qmp_socket = self.qmp_dir.join(format!("{node_id}.sock"));
        if process_matches(&pid_file, &qmp_socket) {
            return Err(RuntimeError::Failed(
                "Cannot resize a QEMU disk while its VM is running".to_owned(),
            ));
        }
        self.command(
            &self.qemu_img,
            [
                "resize",
                canonical.to_str().ok_or_else(|| {
                    RuntimeError::Failed("QEMU disk path is not valid UTF-8".to_owned())
                })?,
                &format!("+{extend_mb}M"),
            ],
            COMMAND_TIMEOUT,
        )
        .await?;
        remove_checksum_sidecar(&canonical)?;
        Ok(canonical)
    }

    pub async fn resize_disk_image_to_size(
        &self,
        project_id: &str,
        node_id: &str,
        file_name: &str,
        size: &str,
        allow_shrink: bool,
    ) -> Result<PathBuf, RuntimeError> {
        let project_id = parse_id(project_id)?;
        let node_id = parse_id(node_id)?;
        let file_name = safe_working_disk_name(file_name)?;
        if !valid_qemu_disk_size(size) {
            return Err(RuntimeError::Failed("Invalid QEMU disk size".to_owned()));
        }
        let directory = self
            .state_dir
            .join(project_id.to_string())
            .join(node_id.to_string());
        let target = directory.join(file_name);
        let root = directory.canonicalize().map_err(|error| {
            RuntimeError::NotFound(format!("QEMU node directory is unavailable: {error}"))
        })?;
        let canonical = target.canonicalize().map_err(|error| {
            RuntimeError::NotFound(format!("QEMU disk image is unavailable: {error}"))
        })?;
        if !canonical.starts_with(&root) || !canonical.is_file() {
            return Err(RuntimeError::Failed(
                "QEMU disk image must be inside the node directory".to_owned(),
            ));
        }
        if process_matches(
            &directory.join(format!("{node_id}.pid")),
            &self.qmp_dir.join(format!("{node_id}.sock")),
        ) {
            return Err(RuntimeError::Failed(
                "Cannot resize a QEMU disk while its VM is running".to_owned(),
            ));
        }
        let mut args = vec!["resize".to_owned()];
        if allow_shrink {
            args.push("--shrink".to_owned());
        }
        args.push(canonical.to_string_lossy().into_owned());
        args.push(size.to_owned());
        self.command(&self.qemu_img, args, COMMAND_TIMEOUT).await?;
        remove_checksum_sidecar(&canonical)?;
        Ok(canonical)
    }

    pub fn delete_disk_image(
        &self,
        project_id: &str,
        node_id: &str,
        file_name: &str,
    ) -> Result<PathBuf, RuntimeError> {
        let project_id = parse_id(project_id)?;
        let node_id = parse_id(node_id)?;
        let file_name = safe_working_disk_name(file_name)?;
        let directory = self
            .state_dir
            .join(project_id.to_string())
            .join(node_id.to_string());
        let root = directory.canonicalize().map_err(|error| {
            RuntimeError::NotFound(format!("QEMU node directory is unavailable: {error}"))
        })?;
        let target = directory.join(file_name);
        let canonical = target.canonicalize().map_err(|error| {
            RuntimeError::NotFound(format!("QEMU disk image is unavailable: {error}"))
        })?;
        if !canonical.starts_with(&root) || !canonical.is_file() {
            return Err(RuntimeError::Failed(
                "QEMU disk image must be inside the node directory".to_owned(),
            ));
        }
        if process_matches(
            &directory.join(format!("{node_id}.pid")),
            &self.qmp_dir.join(format!("{node_id}.sock")),
        ) {
            return Err(RuntimeError::Failed(
                "Cannot delete a QEMU disk while its VM is running".to_owned(),
            ));
        }
        remove_disk_and_checksum(&canonical)?;
        Ok(canonical)
    }
}
