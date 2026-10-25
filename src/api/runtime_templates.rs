use super::*;

pub(super) fn valid_container_path(path: &str) -> bool {
    path.starts_with('/') && !path.contains("..") && path.len() <= 512 && !path.contains('\0')
}

pub(super) fn validate_docker_extra_configs_api(
    configs: Option<&Value>,
    startup_config_content: Option<&str>,
) -> Result<Vec<crate::runtime::DockerExtraConfig>, ApiError> {
    let configs = match configs {
        Some(configs) if !configs.is_null() => {
            serde_json::from_value::<Vec<crate::runtime::DockerExtraConfig>>(configs.clone())
                .map_err(|_| {
                    ApiError::bad_request("extra_configs must be an array of target/content files")
                })?
        }
        Some(_) | None => Vec::new(),
    };
    crate::runtime::validate_docker_extra_configs(&configs, startup_config_content)
        .map_err(|error| ApiError::bad_request(error.message()))?;
    Ok(configs)
}

pub(super) async fn pull_docker_image(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let image = body
        .get("image")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|image| !image.is_empty())
        .ok_or_else(|| ApiError::bad_request("Docker image reference is required"))?;
    let image_id = state
        .runtime
        .pull_image(image)
        .await
        .map_err(runtime_api_error)?;
    let templates = base_runtime_templates(&state).await;
    let template = templates
        .into_iter()
        .find(|template| {
            template["template_type"] == "docker"
                && template["image"].as_str().is_some_and(|candidate| {
                    candidate == image
                        || (!image.contains(':') && candidate.starts_with(&format!("{image}:")))
                })
        })
        .ok_or_else(|| {
            ApiError::conflict("Docker pulled the image but did not expose a local template")
        })?;
    Ok(Json(json!({ "image_id": image_id, "template": template })))
}

pub(super) async fn base_runtime_templates(state: &AppState) -> Vec<Value> {
    let mut templates = BuiltinRuntime::templates();
    match state.runtime.templates().await {
        Ok(available) => templates.extend(available),
        Err(error) => tracing::debug!(error = %error.message(), "Docker templates are unavailable"),
    }
    match state.qemu.templates().await {
        Ok(available) => templates.extend(available),
        Err(error) => tracing::debug!(error = %error.message(), "QEMU templates are unavailable"),
    }
    match state.vpcs.templates().await {
        Ok(available) => templates.extend(
            available
                .into_iter()
                .filter(|template| template["template_type"] != "vpcs"),
        ),
        Err(error) => tracing::debug!(error = %error.message(), "VPCS templates are unavailable"),
    }
    match state.dynamips.templates().await {
        Ok(available) => templates.extend(available),
        Err(error) => {
            tracing::debug!(error = %error.message(), "Dynamips templates are unavailable")
        }
    }
    templates
}

pub(super) fn runtime_api_error(error: RuntimeError) -> ApiError {
    let status = match &error {
        RuntimeError::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
        RuntimeError::NotFound(_) => StatusCode::NOT_FOUND,
        RuntimeError::Failed(_) => StatusCode::CONFLICT,
    };
    ApiError::new(status, error.message())
}
