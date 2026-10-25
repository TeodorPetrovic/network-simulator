use super::*;

#[derive(Deserialize)]
pub(super) struct ImageQuery {
    image_type: Option<String>,
    name: Option<String>,
}
pub(super) async fn list_images(
    State(state): State<AppState>,
    Query(query): Query<ImageQuery>,
) -> Result<Json<Vec<Value>>, ApiError> {
    let mut dirs = Vec::new();
    if query.image_type.as_deref().is_none_or(|t| t == "qemu") {
        dirs.push(("qemu", state.qemu.info().await.image_directory));
    }
    if query.image_type.as_deref().is_none_or(|t| t == "ios") {
        dirs.push(("ios", state.dynamips.info().await.image_directory));
    }
    if query
        .image_type
        .as_deref()
        .is_some_and(|t| !matches!(t, "qemu" | "ios" | "iou"))
    {
        return Err(ApiError::bad_request(
            "image_type must be qemu, ios, or iou",
        ));
    }
    let mut items = Vec::new();
    for (kind, dir) in dirs {
        let root = match tokio::fs::canonicalize(&dir).await {
            Ok(root) => root,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    error.to_string(),
                ));
            }
        };
        let mut pending = vec![root.clone()];
        while let Some(directory) = pending.pop() {
            let mut entries = tokio::fs::read_dir(&directory)
                .await
                .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            {
                let file_type = entry
                    .file_type()
                    .await
                    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                if file_type.is_symlink() {
                    continue;
                }
                if file_type.is_dir() {
                    pending.push(entry.path());
                    continue;
                }
                if !file_type.is_file() {
                    continue;
                }
                let relative = entry
                    .path()
                    .strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                let file_name = entry.file_name().to_string_lossy().to_string();
                if query
                    .name
                    .as_deref()
                    .is_some_and(|name| name != file_name && name != relative)
                {
                    continue;
                }
                let bytes = tokio::fs::read(entry.path())
                    .await
                    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                let md5 = format!("{:x}", md5::Md5::digest(&bytes));
                items.push(json!({"filename":file_name,"path":relative,"image_type":kind,"image_size":bytes.len(),"checksum":md5,"checksum_algorithm":"md5","availability":"available"}));
            }
        }
    }
    Ok(Json(items))
}
pub(super) async fn upload_image(
    State(state): State<AppState>,
    AxumPath(image_path): AxumPath<String>,
    request: Request,
) -> Result<Response, ApiError> {
    let relative = safe_upload_relative_path(&image_path)?;
    let name = relative
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| ApiError::bad_request("image path must end in a filename"))?;
    if is_qemu_image_upload(name) {
        let (tmp, _) = state
            .qemu
            .prepare_upload_name(name)
            .map_err(runtime_api_error)?;
        let dest =
            prepare_nested_destination(&state.qemu.info().await.image_directory, &relative).await?;
        let (size, checksum) =
            write_image_upload(request.into_body(), &tmp, MAX_QEMU_IMAGE_BYTES as u64).await?;
        if let Err(error) = state.qemu.finalize_uploaded_image(&tmp, &dest).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(runtime_api_error(error));
        }
        let can_create_template = can_create_template(&state, name, &checksum);
        Ok((StatusCode::CREATED, Json(json!({"filename":name,"path":dest.to_string_lossy(),"image_type":"qemu","image_size":size,"checksum":checksum,"checksum_algorithm":"md5","availability":"available","can_create_template":can_create_template}))).into_response())
    } else {
        let (tmp, _) = state
            .dynamips
            .prepare_upload_name(name)
            .map_err(runtime_api_error)?;
        let dest =
            prepare_nested_destination(&state.dynamips.info().await.image_directory, &relative)
                .await?;
        let (size, checksum) =
            write_image_upload(request.into_body(), &tmp, MAX_DYNAMIPS_IMAGE_BYTES as u64).await?;
        if let Err(error) = state.dynamips.finalize_uploaded_image(&tmp, &dest) {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(runtime_api_error(error));
        }
        let can_create_template = can_create_template(&state, name, &checksum);
        Ok((StatusCode::CREATED, Json(json!({"filename":name,"path":dest.to_string_lossy(),"image_type":"ios","image_size":size,"checksum":checksum,"checksum_algorithm":"md5","availability":"available","can_create_template":can_create_template}))).into_response())
    }
}

fn can_create_template(state: &AppState, filename: &str, md5: &str) -> bool {
    state
        .marketplace
        .imported_appliances()
        .is_ok_and(|appliances| appliance_image_matches(&appliances, filename, md5))
}

fn safe_upload_relative_path(value: &str) -> Result<PathBuf, ApiError> {
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(ApiError::bad_request(
            "image path must be a relative path without traversal components",
        ));
    }
    Ok(path.to_path_buf())
}

async fn prepare_nested_destination(root: &str, relative: &Path) -> Result<PathBuf, ApiError> {
    let root = tokio::fs::canonicalize(root)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let parent = root.join(relative).parent().unwrap().to_path_buf();
    tokio::fs::create_dir_all(&parent)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let canonical_parent = tokio::fs::canonicalize(&parent)
        .await
        .map_err(|e| ApiError::bad_request(format!("invalid image destination: {e}")))?;
    if !canonical_parent.starts_with(&root) {
        return Err(ApiError::bad_request(
            "image destination escapes configured image directory",
        ));
    }
    let target = canonical_parent.join(relative.file_name().unwrap());
    if tokio::fs::symlink_metadata(&target).await.is_ok() {
        return Err(ApiError::bad_request("image destination already exists"));
    }
    Ok(target)
}

pub(super) fn is_qemu_image_upload(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "qcow2" | "qcow" | "img" | "raw" | "vmdk" | "vdi"
            )
        })
}

pub(super) async fn write_image_upload(
    body: Body,
    temporary_path: &Path,
    max_bytes: u64,
) -> Result<(u64, String), ApiError> {
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary_path)
        .await
        .map_err(|error| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Could not create temporary image upload file: {error}"),
            )
        })?;
    let mut stream = body.into_data_stream();
    let mut size = 0_u64;
    let mut checksum = md5::Md5::new();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                drop(file);
                let _ = tokio::fs::remove_file(temporary_path).await;
                return Err(ApiError::bad_request(format!(
                    "Image upload failed: {error}"
                )));
            }
        };
        size = size.saturating_add(chunk.len() as u64);
        if size > max_bytes {
            drop(file);
            let _ = tokio::fs::remove_file(temporary_path).await;
            return Err(ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "Image upload exceeds the size limit",
            ));
        }
        if let Err(error) = file.write_all(&chunk).await {
            drop(file);
            let _ = tokio::fs::remove_file(temporary_path).await;
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Could not write image upload: {error}"),
            ));
        }
        checksum.update(&chunk);
    }
    if let Err(error) = file.flush().await {
        drop(file);
        let _ = tokio::fs::remove_file(temporary_path).await;
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not flush image upload: {error}"),
        ));
    }
    drop(file);
    if size == 0 {
        let _ = tokio::fs::remove_file(temporary_path).await;
        return Err(ApiError::bad_request("image upload is empty"));
    }
    Ok((size, format!("{:x}", checksum.finalize())))
}
pub(super) async fn get_image(
    State(state): State<AppState>,
    AxumPath(image_path): AxumPath<String>,
) -> Result<Json<Value>, ApiError> {
    let needle = Path::new(&image_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&image_path);
    let images = list_images(
        State(state),
        Query(ImageQuery {
            image_type: None,
            name: None,
        }),
    )
    .await?
    .0;
    images
        .into_iter()
        .find(|image| {
            image["filename"].as_str() == Some(needle)
                || image["path"].as_str() == Some(image_path.as_str())
        })
        .map(Json)
        .ok_or_else(|| ApiError::not_found("Image does not exist"))
}
pub(super) async fn delete_image(
    State(state): State<AppState>,
    AxumPath(image_path): AxumPath<String>,
) -> Result<StatusCode, ApiError> {
    let path = Path::new(&image_path);
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(ApiError::bad_request(
            "image path must not contain parent references",
        ));
    }
    let roots = [
        Path::new(&state.qemu.info().await.image_directory).to_path_buf(),
        Path::new(&state.dynamips.info().await.image_directory).to_path_buf(),
    ];
    let target = if path.is_absolute() {
        path.to_path_buf()
    } else {
        let mut found = None;
        for root in &roots {
            let p = root.join(path);
            if p.is_file() {
                found = Some(p);
                break;
            }
        }
        found.ok_or_else(|| ApiError::not_found("Image does not exist"))?
    };
    if !roots.iter().any(|root| target.starts_with(root)) {
        return Err(ApiError::bad_request(
            "image path is outside the configured image directories",
        ));
    }
    let target_string = target.to_string_lossy().to_string();
    let rows = sqlx::query("SELECT object_json FROM topology_objects WHERE kind='node'")
        .fetch_all(state.store.pool())
        .await?;
    for row in rows {
        let v = decode_json(row.get("object_json"))?;
        for key in ["image", "hda_disk_image"] {
            if v[key].as_str() == Some(target_string.as_str()) {
                return Err(ApiError::conflict("Image is used by a node"));
            }
        }
    }
    tokio::fs::remove_file(target)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod image_path_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn image_upload_paths_reject_traversal_and_absolute_paths() {
        assert!(safe_upload_relative_path("../outside.qcow2").is_err());
        assert!(safe_upload_relative_path("nested/../../outside.qcow2").is_err());
        assert!(safe_upload_relative_path("/tmp/outside.qcow2").is_err());
        assert_eq!(
            safe_upload_relative_path("nested/router.qcow2")
                .unwrap()
                .to_str(),
            Some("nested/router.qcow2")
        );
    }

    #[tokio::test]
    async fn image_listing_recurses_and_returns_relative_paths() {
        let directory = TempDir::new().unwrap();
        let store =
            crate::storage::Store::connect(directory.path().join("api.sqlite").to_str().unwrap())
                .await
                .unwrap();
        let images = directory.path().join("qemu");
        let nested = images.join("vendor/router");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("disk.qcow2"), b"disk").unwrap();
        let mut state = AppState::new(store);
        state.qemu = crate::qemu::QemuRuntime::with_paths(
            "qemu",
            "qemu-img",
            &images,
            directory.path().join("state"),
            directory.path().join("qmp"),
        );
        let result = list_images(
            State(state),
            Query(ImageQuery {
                image_type: Some("qemu".into()),
                name: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(result.0.len(), 1);
        assert_eq!(result.0[0]["path"], "vendor/router/disk.qcow2");
        assert_eq!(result.0[0]["filename"], "disk.qcow2");
    }

    #[test]
    fn upload_template_match_accepts_registered_filename_or_md5() {
        use crate::marketplace::{
            ApplianceFile, ApplianceImageRequirement, ApplianceRecord, ApplianceVersion,
        };
        let appliance = ApplianceRecord {
            file_name: "router.gns3a".into(),
            appliance_id: "id".into(),
            name: "Router".into(),
            origin: "test".into(),
            category: None,
            description: None,
            status: None,
            registry_version: None,
            template_type: None,
            imported: true,
            source_url: String::new(),
            required_files: vec![ApplianceFile {
                filename: "disk.qcow2".into(),
                version: None,
                size: None,
                md5sum: Some("abc123".into()),
                download_url: None,
            }],
            versions: vec![ApplianceVersion {
                name: "Default".into(),
                settings: None,
                template_type: Some("qemu".into()),
                runtime_supported: true,
                docker_image: None,
                docker_image_id: None,
                docker_image_available: false,
                images: vec![ApplianceImageRequirement {
                    key: "hda_disk_image".into(),
                    filename: "disk.qcow2".into(),
                    assignable: true,
                    filesize: None,
                    md5sum: Some("abc123".into()),
                    assigned_image_id: None,
                    assigned_image_name: None,
                    suggested_image_id: None,
                    suggested_image_name: None,
                    image_path: None,
                    actual_size: None,
                    actual_md5sum: None,
                    actual_sha1sum: None,
                    status: "missing".into(),
                    allow_checksum_mismatch: false,
                }],
            }],
        };
        assert!(appliance_image_matches(
            std::slice::from_ref(&appliance),
            "disk.qcow2",
            "other"
        ));
        assert!(appliance_image_matches(
            std::slice::from_ref(&appliance),
            "renamed.qcow2",
            "ABC123"
        ));
        assert!(!appliance_image_matches(
            &[appliance],
            "other.qcow2",
            "other"
        ));
    }
}

fn appliance_image_matches(
    appliances: &[crate::marketplace::ApplianceRecord],
    filename: &str,
    md5: &str,
) -> bool {
    appliances
        .iter()
        .flat_map(|appliance| &appliance.versions)
        .flat_map(|version| &version.images)
        .any(|image| {
            image.filename == filename
                || image
                    .md5sum
                    .as_deref()
                    .is_some_and(|expected| expected.eq_ignore_ascii_case(md5))
        })
}
