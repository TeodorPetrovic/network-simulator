use super::*;

impl MarketplaceCatalog {
    pub async fn set_image_binding(
        &self,
        file_name: &str,
        version_name: &str,
        image_key: &str,
        image_id: &str,
        allow_checksum_mismatch: bool,
        base_templates: &[Value],
    ) -> Result<ApplianceRecord, String> {
        let file_name = validate_appliance_file_name(file_name)?;
        let mut record = self
            .imported_appliances()?
            .into_iter()
            .find(|record| record.file_name == file_name)
            .ok_or_else(|| format!("Appliance '{file_name}' is not imported"))?;
        let version = record
            .versions
            .iter_mut()
            .find(|version| version.name == version_name)
            .ok_or_else(|| format!("Appliance configuration '{version_name}' was not found"))?;
        let requirement = version
            .images
            .iter_mut()
            .find(|image| image.key == image_key)
            .ok_or_else(|| {
                format!("Image role '{image_key}' was not found in configuration '{version_name}'")
            })?;
        if !requirement.assignable {
            return Err(format!(
                "Image role '{image_key}' is listed in the appliance, but the current runtime cannot attach this image slot"
            ));
        }
        let local_images = local_appliance_images(base_templates)?;
        let image = local_images
            .iter()
            .find(|image| image.image_id == image_id)
            .ok_or_else(|| {
                "The selected image is no longer in the local runtime library".to_owned()
            })?;
        let expected_runtime = version.template_type.as_deref().unwrap_or_default();
        if image.template_type != expected_runtime {
            return Err(format!(
                "This configuration requires a {expected_runtime} image; '{}' is a {} image",
                image.name, image.template_type
            ));
        }
        let checksum = self.image_checksum(Path::new(&image.path)).await?;
        let hash_matches = requirement
            .md5sum
            .as_deref()
            .is_none_or(|expected| expected.eq_ignore_ascii_case(&checksum.md5sum));
        let size_matches = requirement
            .filesize
            .is_none_or(|expected| expected == checksum.size);
        if (!hash_matches || !size_matches) && !allow_checksum_mismatch {
            return Err(format!(
                "Selected image does not match the appliance definition. Expected MD5: {}; actual MD5: {}. Expected size: {}; actual size: {} bytes. Enable the explicit checksum override to use this image anyway.",
                requirement.md5sum.as_deref().unwrap_or("not provided"),
                checksum.md5sum,
                requirement
                    .filesize
                    .map(|size| size.to_string())
                    .unwrap_or_else(|| "not provided".to_owned()),
                checksum.size
            ));
        }
        let binding = PersistedImageBinding {
            version: version_name.to_owned(),
            image_key: image_key.to_owned(),
            image_id: image.image_id.clone(),
            image_path: image.path.clone(),
            allow_checksum_mismatch,
        };
        self.write_image_binding(&record.appliance_id, binding)
            .await?;
        requirement.assigned_image_id = Some(image.image_id.clone());
        requirement.assigned_image_name = Some(image.name.clone());
        requirement.image_path = Some(image.path.clone());
        requirement.actual_size = Some(checksum.size);
        requirement.actual_md5sum = Some(checksum.md5sum);
        requirement.actual_sha1sum = Some(checksum.sha1sum);
        requirement.allow_checksum_mismatch = allow_checksum_mismatch;
        requirement.status = if hash_matches && size_matches {
            verification_status(requirement).to_owned()
        } else {
            "override".to_owned()
        };
        Ok(record)
    }

    pub async fn clear_image_binding(
        &self,
        file_name: &str,
        version_name: &str,
        image_key: &str,
    ) -> Result<ApplianceRecord, String> {
        let file_name = validate_appliance_file_name(file_name)?;
        let record = self
            .imported_appliances()?
            .into_iter()
            .find(|record| record.file_name == file_name)
            .ok_or_else(|| format!("Appliance '{file_name}' is not imported"))?;
        let _guard = self.binding_lock.lock().await;
        let mut bindings = self.read_image_bindings(&record.appliance_id)?;
        let previous_length = bindings.len();
        bindings
            .retain(|binding| binding.version != version_name || binding.image_key != image_key);
        if bindings.len() != previous_length {
            self.save_image_bindings(&record.appliance_id, &bindings)?;
        }
        Ok(record)
    }

    pub(super) async fn image_checksum(&self, path: &Path) -> Result<CachedImageHash, String> {
        let canonical = path
            .canonicalize()
            .map_err(|error| format!("Could not resolve image '{}': {error}", path.display()))?;
        let metadata = fs::metadata(&canonical)
            .map_err(|error| format!("Could not read image '{}': {error}", canonical.display()))?;
        let size = metadata.len();
        let modified_ns = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        if let Some(cached) = self.image_hashes.lock().await.get(&canonical).cloned()
            && cached.size == size
            && cached.modified_ns == modified_ns
        {
            return Ok(cached);
        }
        let hash_path = canonical.clone();
        let (md5sum, sha1sum) = tokio::task::spawn_blocking(move || {
            let mut file = fs::File::open(&hash_path).map_err(|error| error.to_string())?;
            let mut md5 = Md5::new();
            let mut sha1 = Sha1::new();
            let mut buffer = vec![0_u8; 1024 * 1024];
            loop {
                let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
                if count == 0 {
                    break;
                }
                md5.update(&buffer[..count]);
                sha1.update(&buffer[..count]);
            }
            Ok::<_, String>((
                format!("{:x}", md5.finalize()),
                format!("{:x}", sha1.finalize()),
            ))
        })
        .await
        .map_err(|error| format!("Image checksum worker failed: {error}"))??;
        let after = fs::metadata(&canonical).map_err(|error| {
            format!("Could not recheck image '{}': {error}", canonical.display())
        })?;
        let after_modified_ns = after
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        if after.len() != size || after_modified_ns != modified_ns {
            self.image_hashes.lock().await.remove(&canonical);
            return Err(format!(
                "Image '{}' changed while its checksums were being calculated; refresh and retry",
                canonical.display()
            ));
        }
        let checksum = CachedImageHash {
            size,
            modified_ns,
            md5sum,
            sha1sum,
        };
        self.image_hashes
            .lock()
            .await
            .insert(canonical, checksum.clone());
        Ok(checksum)
    }

    pub(super) fn bindings_path(&self, appliance_id: &str) -> Result<PathBuf, String> {
        let id = Uuid::parse_str(appliance_id).map_err(|_| "Appliance ID is invalid".to_owned())?;
        Ok(self.appliance_dir.join(format!("{id}.image-bindings.json")))
    }

    pub(super) fn read_image_bindings(
        &self,
        appliance_id: &str,
    ) -> Result<Vec<PersistedImageBinding>, String> {
        let path = self.bindings_path(appliance_id)?;
        match fs::read(&path) {
            Ok(content) => serde_json::from_slice(&content)
                .map_err(|error| format!("Appliance image bindings are invalid: {error}")),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(format!("Could not read appliance image bindings: {error}")),
        }
    }

    pub(super) async fn write_image_binding(
        &self,
        appliance_id: &str,
        binding: PersistedImageBinding,
    ) -> Result<(), String> {
        let _guard = self.binding_lock.lock().await;
        let mut bindings = self.read_image_bindings(appliance_id)?;
        if let Some(existing) = bindings.iter_mut().find(|existing| {
            existing.version == binding.version && existing.image_key == binding.image_key
        }) {
            *existing = binding;
        } else {
            bindings.push(binding);
        }
        self.save_image_bindings(appliance_id, &bindings)
    }

    pub(super) fn save_image_bindings(
        &self,
        appliance_id: &str,
        bindings: &[PersistedImageBinding],
    ) -> Result<(), String> {
        fs::create_dir_all(&self.appliance_dir).map_err(|error| {
            format!("Could not create appliance configuration directory: {error}")
        })?;
        let path = self.bindings_path(appliance_id)?;
        if bindings.is_empty() {
            match fs::remove_file(&path) {
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
                Err(error) => {
                    return Err(format!(
                        "Could not remove appliance image bindings: {error}"
                    ));
                }
            }
        }
        let temporary_path = path.with_extension(format!("json.{}.tmp", Uuid::new_v4()));
        let bytes = serde_json::to_vec_pretty(bindings)
            .map_err(|error| format!("Could not encode appliance image bindings: {error}"))?;
        fs::write(&temporary_path, bytes)
            .map_err(|error| format!("Could not write appliance image bindings: {error}"))?;
        fs::rename(&temporary_path, &path).map_err(|error| {
            let _ = fs::remove_file(&temporary_path);
            format!("Could not install appliance image bindings: {error}")
        })
    }
}
