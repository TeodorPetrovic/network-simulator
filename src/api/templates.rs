use super::*;

pub(super) async fn list_templates(
    State(state): State<AppState>,
) -> Result<Json<Vec<Value>>, ApiError> {
    Ok(Json(available_templates(&state).await?))
}

pub(super) async fn available_templates(state: &AppState) -> Result<Vec<Value>, ApiError> {
    // Discovered runtime images are only inputs to appliances, never palette entries;
    // the palette is the built-in nodes plus imported/custom appliances.
    let base = base_runtime_templates(state).await;
    let mut templates = BuiltinRuntime::templates();
    templates.extend(state.marketplace.templates(base));
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
    Ok(templates)
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
