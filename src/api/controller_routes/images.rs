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
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    error.to_string(),
                ));
            }
        };
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        {
            if !entry
                .file_type()
                .await
                .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
                .is_file()
            {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_string();
            if query.name.as_deref().is_some_and(|name| name != file_name) {
                continue;
            }
            let bytes = tokio::fs::read(entry.path())
                .await
                .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            let md5 = format!("{:x}", md5::Md5::digest(&bytes));
            items.push(json!({"filename":file_name,"path":entry.path().to_string_lossy(),"image_type":kind,"image_size":bytes.len(),"checksum":md5,"checksum_algorithm":"md5","availability":"available"}));
        }
    }
    Ok(Json(items))
}
pub(super) async fn upload_image(
    State(state): State<AppState>,
    AxumPath(image_path): AxumPath<String>,
    request: Request,
) -> Result<Response, ApiError> {
    let name = Path::new(&image_path)
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| ApiError::bad_request("image path must end in a filename"))?;
    if is_qemu_image_upload(name) {
        let (tmp, dest) = state
            .qemu
            .prepare_upload_name(name)
            .map_err(runtime_api_error)?;
        let (size, checksum) =
            write_image_upload(request.into_body(), &tmp, MAX_QEMU_IMAGE_BYTES as u64).await?;
        if let Err(error) = state.qemu.finalize_uploaded_image(&tmp, &dest).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(runtime_api_error(error));
        }
        Ok((StatusCode::CREATED, Json(json!({"filename":name,"path":dest.to_string_lossy(),"image_type":"qemu","image_size":size,"checksum":checksum,"checksum_algorithm":"md5","availability":"available"}))).into_response())
    } else {
        let (tmp, dest) = state
            .dynamips
            .prepare_upload_name(name)
            .map_err(runtime_api_error)?;
        let (size, checksum) =
            write_image_upload(request.into_body(), &tmp, MAX_DYNAMIPS_IMAGE_BYTES as u64).await?;
        if let Err(error) = state.dynamips.finalize_uploaded_image(&tmp, &dest) {
            let _ = tokio::fs::remove_file(&tmp).await;
            return Err(runtime_api_error(error));
        }
        Ok((StatusCode::CREATED, Json(json!({"filename":name,"path":dest.to_string_lossy(),"image_type":"ios","image_size":size,"checksum":checksum,"checksum_algorithm":"md5","availability":"available"}))).into_response())
    }
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
