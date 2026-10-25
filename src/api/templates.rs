use super::*;

pub(super) async fn list_templates(
    State(state): State<AppState>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Vec<Value>>, ApiError> {
    sync_templates_from_images(&state).await?;
    let mut templates = available_templates(&state).await?;
    mark_missing_images(&mut templates);
    if let Some(name) = query.get("name") {
        templates.retain(|template| template.get("name").and_then(Value::as_str) == Some(name));
    }
    Ok(Json(templates))
}

pub(super) async fn available_templates(state: &AppState) -> Result<Vec<Value>, ApiError> {
    // Discovered runtime images are only inputs to appliances, never palette entries;
    // the palette is the built-in nodes plus imported/custom appliances.
    let base = base_runtime_templates(state).await;
    let mut templates = BuiltinRuntime::templates();
    let marketplace_templates = state.marketplace.templates(base);
    let rows = sqlx::query("SELECT template_id, template_json FROM runtime_template_profiles")
        .fetch_all(state.store.pool())
        .await?;
    for row in rows {
        let template_id: String = row.get("template_id");
        let profile = decode_json(row.get::<String, _>("template_json"))?;
        if let Some(template) = templates
            .iter_mut()
            .find(|template| template["template_id"].as_str() == Some(template_id.as_str()))
            && let (Some(base), Some(overrides)) = (template.as_object_mut(), profile.as_object())
        {
            for (key, value) in overrides {
                base.insert(key.clone(), value.clone());
            }
        }
    }
    let saved = sqlx::query("SELECT template_json FROM controller_templates")
        .fetch_all(state.store.pool())
        .await?;
    let saved = saved
        .iter()
        .map(|row| decode_json(row.get::<String, _>("template_json")))
        .collect::<Result<Vec<_>, _>>()?;
    templates.extend(marketplace_templates.into_iter().filter(|candidate| {
        !saved
            .iter()
            .any(|stored| same_image_reference(candidate, stored))
    }));
    Ok(templates)
}

fn same_image_reference(left: &Value, right: &Value) -> bool {
    let key = match left["template_type"].as_str() {
        Some("qemu") => "hda_disk_image",
        Some("dynamips") => "image",
        _ => return false,
    };
    if right["template_type"] != left["template_type"] {
        return false;
    }
    let (Some(left), Some(right)) = (left[key].as_str(), right[key].as_str()) else {
        return false;
    };
    std::path::Path::new(left).file_name() == std::path::Path::new(right).file_name()
}

/// Create missing palette templates for image files in the runtime image
/// directories. Generated records are persisted once and are never refreshed,
/// so a user's later edits remain intact.
pub(super) async fn sync_templates_from_images(state: &AppState) -> Result<(), ApiError> {
    use std::path::Path;

    let dirs = [
        ("qemu", state.qemu.info().await.image_directory),
        ("ios", state.dynamips.info().await.image_directory),
    ];
    let mut existing = available_templates(state).await?;
    let stored = sqlx::query("SELECT template_json FROM controller_templates")
        .fetch_all(state.store.pool())
        .await?;
    for row in stored {
        existing.push(decode_json(row.get("template_json"))?);
    }
    for (kind, directory) in dirs {
        let mut pending = vec![Path::new(&directory).to_path_buf()];
        while let Some(dir) = pending.pop() {
            let mut entries = match tokio::fs::read_dir(&dir).await {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
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
                let ty = entry
                    .file_type()
                    .await
                    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                if ty.is_symlink() {
                    continue;
                }
                if ty.is_dir() {
                    pending.push(entry.path());
                    continue;
                }
                if !ty.is_file() {
                    continue;
                }
                let path = entry.path();
                let filename = entry.file_name().to_string_lossy().to_string();
                let template_type = if kind == "qemu" {
                    if !super::controller_routes::is_qemu_image_upload(&filename) {
                        continue;
                    }
                    "qemu"
                } else {
                    "dynamips"
                };
                if existing
                    .iter()
                    .any(|template| image_ref_matches(template, template_type, &path))
                {
                    continue;
                }
                let template = generated_image_template(template_type, &filename, &path);
                let id = template["template_id"].as_str().unwrap().to_owned();
                sqlx::query("INSERT OR IGNORE INTO controller_templates (template_id, template_json) VALUES (?, ?)")
                    .bind(&id).bind(serde_json::to_string(&template).map_err(internal_json_error)?)
                    .execute(state.store.pool()).await?;
                existing.push(template);
            }
        }
    }
    Ok(())
}

pub(super) fn mark_missing_images(templates: &mut [Value]) {
    for template in templates {
        let key = match template["template_type"].as_str() {
            Some("qemu") => "hda_disk_image",
            Some("dynamips") => "image",
            _ => continue,
        };
        if let Some(image) = template[key].as_str().filter(|value| !value.is_empty()) {
            template["image_missing"] = json!(!std::path::Path::new(image).is_file());
        }
    }
}

fn image_ref_matches(template: &Value, kind: &str, path: &std::path::Path) -> bool {
    let key = if kind == "qemu" {
        "hda_disk_image"
    } else {
        "image"
    };
    let Some(reference) = template.get(key).and_then(Value::as_str) else {
        return false;
    };
    let reference_path = std::path::Path::new(reference);
    reference == path.to_string_lossy() || reference_path.file_name() == path.file_name()
}

fn generated_image_template(kind: &str, filename: &str, path: &std::path::Path) -> Value {
    let id = Uuid::new_v5(&Uuid::NAMESPACE_URL, path.to_string_lossy().as_bytes()).to_string();
    let image_path = path.to_string_lossy();
    if kind == "qemu" {
        let chr = filename.to_ascii_lowercase().starts_with("mikrotik-chr-");
        let version = if chr {
            filename
                .trim_start_matches("mikrotik-chr-")
                .trim_end_matches(".qcow2")
        } else {
            ""
        };
        json!({"template_id":id,"name":if chr { format!("MikroTik CHR {version}") } else { filename.to_owned() },"template_type":"qemu","compute_id":"local","builtin":false,"auto_image_template":true,"category":"router","symbol":"/symbols/router.svg","hda_disk_image":image_path,"memory":if chr {256} else {512},"adapters":if chr {4} else {1},"cpus":1,"console_type":"telnet","platform":"x86_64","adapter_type":"e1000","hda_disk_interface":if chr {"ide"} else {"virtio"},"boot_priority":if chr {"c"} else {"c"},"linked_clone":chr})
    } else {
        let (platform, chassis) =
            crate::marketplace::detect_dynamips_platform(filename).unwrap_or(("c7200", ""));
        let model = platform.trim_start_matches('c');
        let version = filename
            .strip_prefix(&format!("c{model}"))
            .unwrap_or(filename);
        let slot0 = crate::marketplace::default_dynamips_slot0(platform, chassis);
        json!({"template_id":id,"name":format!("Cisco {model} · {}", version.trim_matches(['-', '_', '.'])),"template_type":"dynamips","compute_id":"local","builtin":false,"auto_image_template":true,"category":"router","symbol":"/symbols/router.svg","image":image_path,"platform":platform,"chassis":if chassis.is_empty(){Value::Null}else{json!(chassis)},"slot0":slot0,"memory":256,"ram":256,"nvram":if platform == "c7200" {512}else if matches!(platform,"c2691"|"c3725"|"c3745") {256}else{128},"adapters":1,"idlepc":"","console_type":"telnet"})
    }
}

pub(super) async fn update_template_profile(
    State(state): State<AppState>,
    Path(template_id): Path<Uuid>,
    Json(patch): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let template_id = template_id.to_string();
    let template = available_templates(&state)
        .await?
        .into_iter()
        .find(|template| template["template_id"].as_str() == Some(template_id.as_str()))
        .ok_or_else(|| ApiError::not_found(format!("Template {template_id} does not exist")))?;
    let patch = validate_template_profile_patch(&template, patch)?;
    let row =
        sqlx::query("SELECT template_json FROM runtime_template_profiles WHERE template_id = ?")
            .bind(&template_id)
            .fetch_optional(state.store.pool())
            .await?;
    let mut profile = row
        .map(|row| decode_json(row.get::<String, _>("template_json")))
        .transpose()?
        .unwrap_or_else(|| json!({}));
    {
        let profile_object = as_object_mut(&mut profile, "template profile must be an object")?;
        for (key, value) in patch.as_object().expect("validated profile is an object") {
            profile_object.insert(key.clone(), value.clone());
        }
    }
    let profile_json = serde_json::to_string(&profile).map_err(internal_json_error)?;
    sqlx::query(
        "INSERT INTO runtime_template_profiles (template_id, template_type, template_json)
         VALUES (?, ?, ?)
         ON CONFLICT(template_id) DO UPDATE SET
           template_type = excluded.template_type,
           template_json = excluded.template_json,
           updated_at = CURRENT_TIMESTAMP",
    )
    .bind(&template_id)
    .bind(template["template_type"].as_str().unwrap_or_default())
    .bind(profile_json)
    .execute(state.store.pool())
    .await?;
    let mut updated = template;
    for (key, value) in profile.as_object().expect("profile is an object") {
        updated[key] = value.clone();
    }
    Ok(Json(updated))
}

pub(super) async fn reset_template_profile(
    State(state): State<AppState>,
    Path(template_id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let template_id = template_id.to_string();
    sqlx::query("DELETE FROM runtime_template_profiles WHERE template_id = ?")
        .bind(&template_id)
        .execute(state.store.pool())
        .await?;
    let template = available_templates(&state)
        .await?
        .into_iter()
        .find(|template| template["template_id"].as_str() == Some(template_id.as_str()))
        .ok_or_else(|| ApiError::not_found(format!("Template {template_id} does not exist")))?;
    Ok(Json(template))
}
