use super::*;

#[derive(Deserialize)]
pub(super) struct CatalogSearchQuery {
    q: Option<String>,
    kind: Option<String>,
}

pub(super) async fn search_ishare2_images(
    State(state): State<AppState>,
    Query(query): Query<CatalogSearchQuery>,
) -> Result<Json<Value>, ApiError> {
    let results = state
        .marketplace
        .search_ishare2(
            query.q.as_deref().unwrap_or_default(),
            query.kind.as_deref().unwrap_or("all"),
        )
        .await
        .map_err(catalog_api_error)?;
    Ok(Json(json!({
        "source": "iShare2 / LabHub",
        "results": results
    })))
}

pub(super) async fn download_ishare2_image(
    State(state): State<AppState>,
    Path((image_type, image_id)): Path<(String, u64)>,
) -> Result<Json<crate::marketplace::IshareDownloadResult>, ApiError> {
    state
        .marketplace
        .download_ishare2(&image_type, image_id)
        .await
        .map(Json)
        .map_err(catalog_api_error)
}

pub(super) async fn search_gns3_appliances(
    State(state): State<AppState>,
    Query(query): Query<CatalogSearchQuery>,
) -> Result<Json<Value>, ApiError> {
    let results = state
        .marketplace
        .search_gns3_appliances(query.q.as_deref().unwrap_or_default())
        .await
        .map_err(catalog_api_error)?;
    Ok(Json(json!({
        "source": "GNS3 Marketplace appliance registry",
        "results": results
    })))
}

pub(super) async fn get_gns3_appliance(
    State(state): State<AppState>,
    Path(file_name): Path<String>,
) -> Result<Json<ApplianceRecord>, ApiError> {
    state
        .marketplace
        .appliance_details(&file_name)
        .await
        .map(Json)
        .map_err(catalog_api_error)
}

pub(super) async fn import_gns3_appliance(
    State(state): State<AppState>,
    Path(file_name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let appliance = state
        .marketplace
        .import_appliance(&file_name)
        .await
        .map_err(catalog_api_error)?;
    let templates = available_templates(&state).await?;
    let matching_templates = templates
        .iter()
        .filter(|template| template["appliance_id"].as_str() == Some(&appliance.appliance_id))
        .count();
    Ok(Json(json!({
        "appliance": appliance,
        "matching_local_templates": matching_templates,
        "message": if matching_templates > 0 {
            "Appliance imported and matched to installed runtime images"
        } else {
            "Appliance definition imported; install or select its required image to use it"
        }
    })))
}

pub(super) async fn list_imported_appliances(
    State(state): State<AppState>,
) -> Result<Json<Vec<ApplianceRecord>>, ApiError> {
    state
        .marketplace
        .imported_appliances()
        .map(Json)
        .map_err(catalog_api_error)
}

pub(super) async fn get_appliance_manager(
    State(state): State<AppState>,
) -> Result<Json<crate::marketplace::ApplianceManagerState>, ApiError> {
    let image_options = base_runtime_templates(&state).await;
    state
        .marketplace
        .appliance_manager(&image_options)
        .await
        .map(Json)
        .map_err(catalog_api_error)
}

#[derive(Deserialize)]
pub(super) struct CreateCustomAppliance {
    name: String,
    source_template_id: String,
    #[serde(default = "default_object")]
    runtime_settings: Value,
}

pub(super) fn default_object() -> Value {
    json!({})
}

pub(super) async fn create_custom_appliance(
    State(state): State<AppState>,
    Json(request): Json<CreateCustomAppliance>,
) -> Result<Json<crate::marketplace::ApplianceRecord>, ApiError> {
    let image_options = base_runtime_templates(&state).await;
    state
        .marketplace
        .create_custom_appliance(
            &request.name,
            &request.source_template_id,
            &request.runtime_settings,
            &image_options,
        )
        .map(Json)
        .map_err(ApiError::bad_request)
}

#[derive(Deserialize)]
pub(super) struct SetApplianceImageBinding {
    version: String,
    image_key: String,
    image_id: String,
    #[serde(default)]
    allow_checksum_mismatch: bool,
}

pub(super) async fn set_appliance_image_binding(
    State(state): State<AppState>,
    Path(file_name): Path<String>,
    Json(request): Json<SetApplianceImageBinding>,
) -> Result<Json<Value>, ApiError> {
    let templates = base_runtime_templates(&state).await;
    let appliance = state
        .marketplace
        .set_image_binding(
            &file_name,
            &request.version,
            &request.image_key,
            &request.image_id,
            request.allow_checksum_mismatch,
            &templates,
        )
        .await
        .map_err(catalog_api_error)?;
    Ok(Json(json!({ "appliance": appliance })))
}

#[derive(Deserialize)]
pub(super) struct ClearApplianceImageBinding {
    version: String,
    image_key: String,
}

pub(super) async fn clear_appliance_image_binding(
    State(state): State<AppState>,
    Path(file_name): Path<String>,
    Json(request): Json<ClearApplianceImageBinding>,
) -> Result<Json<Value>, ApiError> {
    let appliance = state
        .marketplace
        .clear_image_binding(&file_name, &request.version, &request.image_key)
        .await
        .map_err(catalog_api_error)?;
    Ok(Json(json!({ "appliance": appliance })))
}

pub(super) async fn import_appliance_file(
    State(state): State<AppState>,
    Path(file_name): Path<String>,
    body: Body,
) -> Result<Json<ApplianceRecord>, ApiError> {
    let bytes = axum::body::to_bytes(body, MAX_BODY_BYTES)
        .await
        .map_err(|error| {
            ApiError::bad_request(format!("Could not read appliance file: {error}"))
        })?;
    state
        .marketplace
        .import_appliance_file(&file_name, &bytes)
        .map(Json)
        .map_err(catalog_api_error)
}

pub(super) fn catalog_api_error(message: String) -> ApiError {
    ApiError::new(StatusCode::BAD_GATEWAY, message)
}

pub(super) async fn upload_dynamips_image(
    State(state): State<AppState>,
    Path(file_name): Path<String>,
    body: Body,
) -> Result<Json<Value>, ApiError> {
    let (temporary_path, final_path) = state
        .dynamips
        .prepare_upload_name(&file_name)
        .map_err(runtime_api_error)?;
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)
        .await
        .map_err(|error| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Could not create temporary IOS upload file: {error}"),
            )
        })?;
    let mut stream = body.into_data_stream();
    let mut size = 0_usize;
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                drop(file);
                let _ = std::fs::remove_file(&temporary_path);
                return Err(ApiError::bad_request(format!(
                    "IOS image upload failed: {error}"
                )));
            }
        };
        size = size.saturating_add(chunk.len());
        if size > MAX_DYNAMIPS_IMAGE_BYTES {
            drop(file);
            let _ = std::fs::remove_file(&temporary_path);
            return Err(ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "IOS images must be 512 MiB or smaller",
            ));
        }
        if let Err(error) = file.write_all(&chunk).await {
            drop(file);
            let _ = std::fs::remove_file(&temporary_path);
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Could not write IOS image upload: {error}"),
            ));
        }
    }
    if let Err(error) = file.flush().await {
        drop(file);
        let _ = std::fs::remove_file(&temporary_path);
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not flush IOS image upload: {error}"),
        ));
    }
    drop(file);
    if size == 0 {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(ApiError::bad_request("The IOS image upload is empty"));
    }
    let image = match state
        .dynamips
        .finalize_uploaded_image(&temporary_path, &final_path)
    {
        Ok(image) => image,
        Err(error) => {
            let _ = std::fs::remove_file(&temporary_path);
            return Err(runtime_api_error(error));
        }
    };
    Ok(Json(json!({
        "name": file_name,
        "image": image,
        "template_type": "dynamips",
        "bytes": size
    })))
}

pub(super) async fn upload_qemu_image(
    State(state): State<AppState>,
    Path(file_name): Path<String>,
    body: Body,
) -> Result<Json<Value>, ApiError> {
    let (temporary_path, final_path) = state
        .qemu
        .prepare_upload_name(&file_name)
        .map_err(runtime_api_error)?;
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)
        .await
        .map_err(|error| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Could not create temporary QEMU upload file: {error}"),
            )
        })?;
    let mut stream = body.into_data_stream();
    let mut size = 0_usize;
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                drop(file);
                let _ = std::fs::remove_file(&temporary_path);
                return Err(ApiError::bad_request(format!(
                    "QEMU image upload failed: {error}"
                )));
            }
        };
        size = size.saturating_add(chunk.len());
        if size > MAX_QEMU_IMAGE_BYTES {
            drop(file);
            let _ = std::fs::remove_file(&temporary_path);
            return Err(ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "QEMU disk images must be 64 GiB or smaller",
            ));
        }
        if let Err(error) = file.write_all(&chunk).await {
            drop(file);
            let _ = std::fs::remove_file(&temporary_path);
            return Err(ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Could not write QEMU image upload: {error}"),
            ));
        }
    }
    if let Err(error) = file.flush().await {
        drop(file);
        let _ = std::fs::remove_file(&temporary_path);
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Could not flush QEMU image upload: {error}"),
        ));
    }
    drop(file);
    if size == 0 {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(ApiError::bad_request("QEMU image upload is empty"));
    }
    match state
        .qemu
        .finalize_uploaded_image(&temporary_path, &final_path)
        .await
    {
        Ok(image) => Ok(Json(json!({
            "name": file_name,
            "image": image,
            "template_type": "qemu",
            "bytes": size
        }))),
        Err(error) => {
            let _ = std::fs::remove_file(&temporary_path);
            Err(runtime_api_error(error))
        }
    }
}
