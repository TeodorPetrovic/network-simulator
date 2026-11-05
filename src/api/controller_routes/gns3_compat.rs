//! GNS3 3.1 controller routes that map onto this port's own storage: the
//! `/v3/appliances` catalog, blank QEMU disk creation, image pruning, and
//! per-template base-config files.

use super::*;
use super::super::marketplace_api::catalog_api_error;

// ---------------------------------------------------------------- appliances

pub(super) async fn list_appliances(
    State(state): State<AppState>,
) -> Result<Json<Vec<crate::marketplace::ApplianceRecord>>, ApiError> {
    state
        .marketplace
        .imported_appliances()
        .map(Json)
        .map_err(catalog_api_error)
}

fn find_appliance(
    state: &AppState,
    appliance_id: &str,
) -> Result<crate::marketplace::ApplianceRecord, ApiError> {
    state
        .marketplace
        .imported_appliances()
        .map_err(catalog_api_error)?
        .into_iter()
        .find(|appliance| appliance.appliance_id == appliance_id)
        .ok_or_else(|| ApiError::not_found(format!("Could not find appliance '{appliance_id}'")))
}

pub(super) async fn get_appliance(
    State(state): State<AppState>,
    AxumPath(appliance_id): AxumPath<String>,
) -> Result<Json<crate::marketplace::ApplianceRecord>, ApiError> {
    find_appliance(&state, &appliance_id).map(Json)
}

#[derive(Deserialize)]
pub(super) struct InstallQuery {
    version: Option<String>,
}

/// Templates here are derived from imported appliance definitions plus the images on disk,
/// so "installing" re-runs that sync and returns the template that now matches.
pub(super) async fn install_appliance(
    State(state): State<AppState>,
    AxumPath(appliance_id): AxumPath<String>,
    Query(query): Query<InstallQuery>,
) -> Result<Response, ApiError> {
    let appliance = find_appliance(&state, &appliance_id)?;
    super::super::templates::sync_templates_from_images(&state).await?;
    let templates = available_templates(&state).await?;
    let template = templates.into_iter().find(|template| {
        template["appliance_id"].as_str() == Some(appliance.appliance_id.as_str())
            && query.version.as_deref().is_none_or(|version| {
                template["appliance_version"].as_str() == Some(version)
                    || template["version"].as_str() == Some(version)
            })
    });
    match template {
        Some(template) => Ok((StatusCode::CREATED, Json(template)).into_response()),
        None => Err(ApiError::conflict(format!(
            "No template is available for appliance '{}'{}: upload its required images first",
            appliance.name,
            query
                .version
                .as_deref()
                .map(|v| format!(" version '{v}'"))
                .unwrap_or_default()
        ))),
    }
}

// ------------------------------------------------------------- qemu disk image

#[derive(Deserialize)]
pub(super) struct QemuDiskImageCreate {
    format: String,
    /// Size in MB.
    size: u64,
    #[serde(default)]
    preallocation: Option<String>,
}

pub(super) async fn create_qemu_image(
    State(state): State<AppState>,
    AxumPath(image_path): AxumPath<String>,
    Json(body): Json<QemuDiskImageCreate>,
) -> Result<Response, ApiError> {
    if !matches!(
        body.format.as_str(),
        "qcow2" | "qcow" | "vpc" | "vdi" | "vmdk" | "vhdx" | "qed"
    ) {
        return Err(ApiError::bad_request(
            "format must be one of qcow2, qcow, vpc, vdi, vmdk, vhdx, qed (raw images are not allowed)",
        ));
    }
    if body.size == 0 || body.size > 4 * 1024 * 1024 {
        return Err(ApiError::bad_request("size must be between 1 and 4194304 MB"));
    }
    if let Some(mode) = body.preallocation.as_deref() {
        if !matches!(mode, "off" | "metadata" | "falloc" | "full") {
            return Err(ApiError::bad_request(
                "preallocation must be off, metadata, falloc or full",
            ));
        }
    }
    let relative = safe_upload_relative_path(&image_path)?;
    let name = relative
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| ApiError::bad_request("image path must end in a filename"))?
        .to_owned();
    let dest = prepare_nested_destination(&state.qemu.info().await.image_directory, &relative).await?;
    let mut command = tokio::process::Command::new("qemu-img");
    command.arg("create").args(["-f", &body.format]);
    if let Some(mode) = body.preallocation.as_deref() {
        command.args(["-o", &format!("preallocation={mode}")]);
    }
    command.arg(&dest).arg(format!("{}M", body.size));
    let output = command.output().await.map_err(|error| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            format!("could not run qemu-img: {error}"),
        )
    })?;
    if !output.status.success() {
        let _ = tokio::fs::remove_file(&dest).await;
        return Err(ApiError::bad_request(format!(
            "qemu-img failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    super::super::templates::sync_templates_from_images(&state).await?;
    let size = tokio::fs::metadata(&dest).await.map(|m| m.len()).unwrap_or(0);
    Ok((
        StatusCode::CREATED,
        Json(json!({"filename":name,"path":dest.to_string_lossy(),"image_type":"qemu","image_size":size,"format":body.format})),
    )
        .into_response())
}

// ---------------------------------------------------------------------- prune

fn collect_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => collect_files(&path, out),
            Ok(kind) if kind.is_file() => out.push(path),
            _ => {}
        }
    }
}

/// Delete image files that no template and no node in any project refers to.
/// A file counts as referenced when its file name (or its path relative to the
/// image directory) appears anywhere in a template or node definition.
pub(super) async fn prune_images(State(state): State<AppState>) -> Result<StatusCode, ApiError> {
    let mut haystack = String::new();
    for template in available_templates(&state).await? {
        haystack.push_str(&template.to_string());
        haystack.push('\n');
    }
    let rows = sqlx::query("SELECT object_json FROM topology_objects WHERE kind = 'node'")
        .fetch_all(state.store.pool())
        .await?;
    for row in rows {
        haystack.push_str(&row.get::<String, _>("object_json"));
        haystack.push('\n');
    }
    let roots = [
        PathBuf::from(state.qemu.info().await.image_directory),
        PathBuf::from(state.dynamips.info().await.image_directory),
    ];
    let mut removed = 0usize;
    for root in roots {
        let mut files = Vec::new();
        collect_files(&root, &mut files);
        for file in files {
            let Some(name) = file.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name.starts_with('.') || name.ends_with(".md5sum") || name.ends_with(".tmp") {
                continue;
            }
            let relative = file
                .strip_prefix(&root)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            if haystack.contains(name) || (!relative.is_empty() && haystack.contains(&relative)) {
                continue;
            }
            if tokio::fs::remove_file(&file).await.is_ok() {
                removed += 1;
                let _ = tokio::fs::remove_file(format!("{}.md5sum", file.display())).await;
            }
        }
    }
    tracing::info!(removed, "pruned unreferenced images");
    super::super::templates::sync_templates_from_images(&state).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ------------------------------------------------------------- base configs

fn template_files_dir(template_id: &str) -> PathBuf {
    crate::image_paths::application_data_directory()
        .join("template_files")
        .join(template_id)
}

fn safe_config_name(filename: &str) -> Result<String, ApiError> {
    let name = Path::new(filename)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if name.is_empty()
        || name.starts_with('.')
        || name.len() > 128
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(ApiError::bad_request(
            "filename may only use letters, digits, dot, underscore and hyphen",
        ));
    }
    Ok(name.to_owned())
}

async fn require_template(state: &AppState, template_id: &Uuid) -> Result<String, ApiError> {
    let id = template_id.to_string();
    let found = available_templates(state)
        .await?
        .iter()
        .any(|template| template["template_id"].as_str() == Some(id.as_str()));
    if found {
        Ok(id)
    } else {
        Err(ApiError::not_found(format!("Template '{id}' not found")))
    }
}

pub(super) async fn list_base_configs(
    State(state): State<AppState>,
    AxumPath(template_id): AxumPath<Uuid>,
) -> Result<Json<Vec<String>>, ApiError> {
    let id = require_template(&state, &template_id).await?;
    let mut names = Vec::new();
    if let Ok(mut entries) = tokio::fs::read_dir(template_files_dir(&id)).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_owned());
            }
        }
    }
    names.sort();
    Ok(Json(names))
}

pub(super) async fn get_base_config(
    State(state): State<AppState>,
    AxumPath((template_id, filename)): AxumPath<(Uuid, String)>,
) -> Result<Json<Value>, ApiError> {
    let id = require_template(&state, &template_id).await?;
    let name = safe_config_name(&filename)?;
    let content = tokio::fs::read_to_string(template_files_dir(&id).join(&name))
        .await
        .map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => ApiError::not_found(format!("File '{name}' not found")),
            _ => ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
        })?;
    Ok(Json(json!({"template_id":id,"filename":name,"content":content})))
}

pub(super) async fn put_base_config(
    State(state): State<AppState>,
    AxumPath((template_id, filename)): AxumPath<(Uuid, String)>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let id = require_template(&state, &template_id).await?;
    let name = safe_config_name(&filename)?;
    let content = body
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad_request("Missing 'content' field"))?;
    if content.len() > 1024 * 1024 {
        return Err(ApiError::bad_request("content is limited to 1 MiB"));
    }
    let directory = template_files_dir(&id);
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    tokio::fs::write(directory.join(&name), content)
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({"template_id":id,"filename":name,"content":content})))
}
