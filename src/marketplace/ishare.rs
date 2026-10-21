use super::*;

impl MarketplaceCatalog {
    pub fn new() -> Self {
        Self::with_directory(crate::image_paths::appliance_definitions())
    }

    pub(crate) fn with_directory(appliance_dir: impl Into<PathBuf>) -> Self {
        Self::build(appliance_dir.into())
    }

    pub(super) fn build(appliance_dir: PathBuf) -> Self {
        let client = Client::builder()
            .user_agent(concat!("network-simulator/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .build()
            .expect("the static marketplace HTTP client configuration is valid");
        let download_client = Client::builder()
            .user_agent(concat!("network-simulator/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("the static image download HTTP client configuration is valid");
        Self {
            client,
            download_client,
            appliance_dir,
            cache: Arc::new(Mutex::new(CatalogCache::default())),
            image_hashes: Arc::new(Mutex::new(HashMap::new())),
            binding_lock: Arc::new(Mutex::new(())),
        }
    }

    pub async fn search_ishare2(
        &self,
        query: &str,
        image_type: &str,
    ) -> Result<Vec<Value>, String> {
        let query = validate_query(query)?;
        let image_type = image_type.to_ascii_lowercase();
        if !matches!(image_type.as_str(), "all" | "qemu" | "dynamips" | "iol") {
            return Err("iShare2 image type must be all, qemu, dynamips, or iol".to_owned());
        }
        let index = self.ishare_index().await?;
        let categories: &[&str] = match image_type.as_str() {
            "all" => &["QEMU", "DYNAMIPS", "IOL"],
            "qemu" => &["QEMU"],
            "dynamips" => &["DYNAMIPS"],
            "iol" => &["IOL"],
            _ => unreachable!("image type was validated"),
        };
        let needle = query.to_lowercase();
        let mut images = Vec::new();
        for category in categories {
            let Some(rows) = index.get(*category).and_then(Value::as_array) else {
                continue;
            };
            for row in rows {
                let Some(name) = row.get("name").and_then(Value::as_str) else {
                    continue;
                };
                if !ishare_row_matches_query(row, &needle) {
                    continue;
                }
                let metadata = &row["metadata"];
                images.push(
                    serde_json::to_value(IshareImage {
                        id: row["id"].as_u64().unwrap_or_default(),
                        name: name.to_owned(),
                        image_type: row["type"]
                            .as_str()
                            .unwrap_or(category)
                            .to_ascii_lowercase(),
                        size: metadata["total_human_size"]
                            .as_str()
                            .unwrap_or("size unknown")
                            .to_owned(),
                        total_size: metadata["total_size"].as_u64().unwrap_or_default(),
                        file_count: row["files"].as_array().map_or(0, Vec::len),
                        files: row["files"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|file| file["filename"].as_str().map(str::to_owned))
                            .collect(),
                        catalog: "iShare2 / LabHub",
                    })
                    .map_err(|error| format!("Could not encode iShare2 result: {error}"))?,
                );
                if images.len() >= MAX_SEARCH_RESULTS {
                    return Ok(images);
                }
            }
        }
        Ok(images)
    }

    pub async fn download_ishare2(
        &self,
        image_type: &str,
        image_id: u64,
    ) -> Result<IshareDownloadResult, String> {
        let image_type = image_type.to_ascii_lowercase();
        let category = match image_type.as_str() {
            "qemu" => "QEMU",
            "dynamips" => "DYNAMIPS",
            "iol" => {
                return Err("IOL images are not supported by an installed runtime".to_owned());
            }
            _ => return Err("Image type must be qemu or dynamips".to_owned()),
        };
        let index = self.ishare_index().await?;
        let row = index
            .get(category)
            .and_then(Value::as_array)
            .and_then(|images| {
                images
                    .iter()
                    .find(|image| image["id"].as_u64() == Some(image_id))
            })
            .ok_or_else(|| format!("iShare2 {image_type} image {image_id} was not found"))?;
        let name = row["name"]
            .as_str()
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| "The iShare2 image has no name".to_owned())?;
        let files = row["files"]
            .as_array()
            .filter(|files| !files.is_empty())
            .ok_or_else(|| "The iShare2 image has no downloadable files".to_owned())?;
        let url_properties = index
            .get("url_properties")
            .ok_or_else(|| "The iShare2 index has no mirror configuration".to_owned())?;
        let protocol = url_properties["protocol"]
            .as_str()
            .filter(|protocol| *protocol == "https")
            .ok_or_else(|| "The iShare2 index does not provide HTTPS downloads".to_owned())?;
        let safe_name = safe_catalog_component(name)?;
        let image_directory = match image_type.as_str() {
            "qemu" => crate::image_paths::qemu_images(),
            "dynamips" => crate::image_paths::dynamips_images(),
            _ => unreachable!("image type was validated"),
        };
        fs::create_dir_all(&image_directory).map_err(|error| {
            format!(
                "Could not create global {image_type} image directory '{}': {error}",
                image_directory.display()
            )
        })?;

        let mut last_error = None;
        for mirror in ["drive", "main"] {
            let Some(host) = url_properties["hostnames"][mirror].as_str() else {
                continue;
            };
            let Some(prefix) = url_properties["prefixes"][mirror].as_str() else {
                continue;
            };
            if !valid_ishare_host(host) || !valid_ishare_prefix(prefix) {
                continue;
            }
            match self
                .download_ishare_attempt(
                    files,
                    protocol,
                    host,
                    prefix,
                    &image_type,
                    image_id,
                    &safe_name,
                    &image_directory,
                )
                .await
            {
                Ok(installed_files) => {
                    return Ok(IshareDownloadResult {
                        id: image_id,
                        name: name.to_owned(),
                        image_type,
                        files: installed_files.files,
                        source_files: installed_files.source_files,
                        image_directory: image_directory.display().to_string(),
                    });
                }
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| {
            "The iShare2 index has no supported HTTPS mirrors for this image".to_owned()
        }))
    }

    pub(super) async fn download_ishare_attempt(
        &self,
        files: &[Value],
        protocol: &str,
        host: &str,
        prefix: &str,
        image_type: &str,
        image_id: u64,
        safe_name: &str,
        image_directory: &Path,
    ) -> Result<InstalledIshareFiles, String> {
        let staging = image_directory.join(format!(".ishare-{image_id}-{}", Uuid::new_v4()));
        fs::create_dir(&staging).map_err(|error| {
            format!("Could not create image download staging directory: {error}")
        })?;
        let download = async {
            let mut staged_files = Vec::with_capacity(files.len());
            for file in files {
                let filename = file["filename"]
                    .as_str()
                    .filter(|filename| safe_download_filename(filename))
                    .ok_or_else(|| "The iShare2 manifest contains an unsafe filename".to_owned())?;
                let path = file["path"]
                    .as_str()
                    .filter(|path| safe_ishare_path(path))
                    .ok_or_else(|| "The iShare2 manifest contains an unsafe download path".to_owned())?;
                let url = format!("{protocol}://{host}{prefix}{path}");
                let target = staging.join(filename);
                let downloaded = self
                    .download_verified_file(&url, &target, file)
                    .await?;
                staged_files.push((filename.to_owned(), downloaded));
            }

            let mut to_install = Vec::new();
            for (filename, staged_path) in staged_files {
                let extension = Path::new(&filename)
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let (install_name, validate_ios) = match image_type {
                    "qemu" if is_supported_qemu_extension(&extension) => {
                        (format!("{safe_name}-{filename}"), false)
                    }
                    "qemu" => continue,
                    "dynamips" if files.len() == 1 => {
                        let prefixed = format!("ishare-{image_id}-{filename}");
                        (
                            if prefixed.len() <= 255 {
                                prefixed
                            } else {
                                filename.to_owned()
                            },
                            true,
                        )
                    }
                    "dynamips" => {
                        return Err("The selected Dynamips image contains multiple files; install it with the upstream iShare2 CLI".to_owned());
                    }
                    _ => unreachable!("image type was validated"),
                };
                if validate_ios {
                    crate::dynamips::validate_ios_image(&staged_path).map_err(|error| {
                        format!("Downloaded iShare2 image failed Dynamips IOS validation: {}", error.message())
                    })?;
                }
                let destination = image_directory.join(install_name);
                if destination.exists() {
                    return Err(format!(
                        "Global image '{}' already exists",
                        destination.file_name().unwrap_or_default().to_string_lossy()
                    ));
                }
                to_install.push((staged_path, destination, filename));
            }
            if to_install.is_empty() {
                return Err(format!(
                    "The selected iShare2 {image_type} package contains no supported disk image file"
                ));
            }
            let mut installed_paths = Vec::new();
            for (staged_path, destination, _) in &to_install {
                if let Err(error) = fs::hard_link(staged_path, destination) {
                    for installed in &installed_paths {
                        let _ = fs::remove_file(installed);
                    }
                    return Err(format!("Could not install verified image: {error}"));
                }
                installed_paths.push(destination.clone());
            }
            let source_files = to_install
                .iter()
                .map(|(_, _, source)| source.to_owned())
                .collect();
            let installed = to_install
                .into_iter()
                .map(|(_, path, _)| {
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect();
            let _ = fs::remove_dir_all(&staging);
            Ok(InstalledIshareFiles {
                files: installed,
                source_files,
            })
        }
        .await;
        if download.is_err() {
            let _ = fs::remove_dir_all(&staging);
        }
        download
    }

    pub(super) async fn download_verified_file(
        &self,
        url: &str,
        target: &Path,
        manifest: &Value,
    ) -> Result<PathBuf, String> {
        let response = self
            .download_client
            .get(url)
            .send()
            .await
            .map_err(|error| format!("Could not download iShare2 image file: {error}"))?
            .error_for_status()
            .map_err(|error| format!("iShare2 image download failed: {error}"))?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_ISHARE_FILE_BYTES)
        {
            return Err("The iShare2 image file exceeds the 64 GiB download limit".to_owned());
        }
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)
            .await
            .map_err(|error| format!("Could not create image staging file: {error}"))?;
        let mut total = 0_u64;
        let mut md5 = Md5::new();
        let mut sha1 = Sha1::new();
        let mut response = response;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| format!("Image download stream failed: {error}"))?
        {
            total = total
                .checked_add(chunk.len() as u64)
                .filter(|total| *total <= MAX_ISHARE_FILE_BYTES)
                .ok_or_else(|| {
                    "The iShare2 image file exceeds the 64 GiB download limit".to_owned()
                })?;
            md5.update(&chunk);
            sha1.update(&chunk);
            file.write_all(&chunk)
                .await
                .map_err(|error| format!("Could not write image download: {error}"))?;
        }
        file.sync_all()
            .await
            .map_err(|error| format!("Could not flush image download: {error}"))?;
        drop(file);

        if let Some(expected) = manifest["size"].as_u64()
            && expected != total
        {
            return Err(format!(
                "iShare2 image size check failed (expected {expected} bytes, received {total})"
            ));
        }
        let expected_md5 = manifest["checksum"]["md5"].as_str().unwrap_or_default();
        if !expected_md5.is_empty() && format!("{:x}", md5.finalize()) != expected_md5 {
            return Err("iShare2 image MD5 check failed".to_owned());
        }
        let expected_sha1 = manifest["checksum"]["sha1"].as_str().unwrap_or_default();
        if !expected_sha1.is_empty() && format!("{:x}", sha1.finalize()) != expected_sha1 {
            return Err("iShare2 image SHA1 check failed".to_owned());
        }
        if expected_md5.is_empty() && expected_sha1.is_empty() {
            return Err("The iShare2 manifest provides no checksum for this file".to_owned());
        }
        Ok(target.to_path_buf())
    }
}
