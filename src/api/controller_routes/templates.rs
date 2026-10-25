use super::*;

pub(super) async fn list_api_templates(
    State(state): State<AppState>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Vec<Value>>, ApiError> {
    super::super::templates::sync_templates_from_images(&state).await?;
    let mut templates = available_templates(&state).await?;
    let rows = sqlx::query("SELECT template_json FROM controller_templates ORDER BY template_id")
        .fetch_all(state.store.pool())
        .await?;
    for row in rows {
        templates.push(decode_json(row.get("template_json"))?);
    }
    if let Some(name) = query.get("name") {
        templates.retain(|template| template.get("name").and_then(Value::as_str) == Some(name));
    }
    Ok(Json(templates))
}

pub(crate) async fn check_version(Json(body): Json<Value>) -> Result<Json<Value>, ApiError> {
    let requested = body
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad_request("version is required"))?;
    if requested != "3.1.0.dev7" {
        return Err(ApiError::conflict(format!(
            "Client version {requested} is not the same as server version 3.1.0.dev7"
        )));
    }
    Ok(Json(json!({"version":"3.1.0.dev7"})))
}

pub(super) async fn create_template(
    State(state): State<AppState>,
    Json(mut template): Json<Value>,
) -> Result<Response, ApiError> {
    let obj = as_object_mut(&mut template, "template body must be an object")?;
    let template_id = parse_or_generate_uuid(obj.get("template_id"), "template_id")?;
    let name = required_string(obj, "name")?.to_owned();
    let template_type = required_string(obj, "template_type")?.to_owned();
    obj.insert("template_id".into(), json!(template_id));
    obj.insert("name".into(), json!(name));
    obj.insert("template_type".into(), json!(template_type));
    obj.entry("compute_id").or_insert(json!("local"));
    obj.entry("builtin").or_insert(json!(false));
    sqlx::query("INSERT INTO controller_templates (template_id, template_json) VALUES (?, ?)")
        .bind(&template_id)
        .bind(serde_json::to_string(&template).map_err(internal_json_error)?)
        .execute(state.store.pool())
        .await?;
    Ok((StatusCode::CREATED, Json(template)).into_response())
}

pub(super) async fn get_template(
    State(state): State<AppState>,
    AxumPath(template_id): AxumPath<Uuid>,
) -> Result<Json<Value>, ApiError> {
    if let Some(row) =
        sqlx::query("SELECT template_json FROM controller_templates WHERE template_id = ?")
            .bind(template_id.to_string())
            .fetch_optional(state.store.pool())
            .await?
    {
        return Ok(Json(decode_json(row.get("template_json"))?));
    }
    let id = template_id.to_string();
    let template = available_templates(&state)
        .await?
        .into_iter()
        .find(|item| item["template_id"].as_str() == Some(id.as_str()))
        .ok_or_else(|| ApiError::not_found(format!("Template {id} does not exist")))?;
    Ok(Json(template))
}

pub(super) async fn update_template(
    State(state): State<AppState>,
    AxumPath(template_id): AxumPath<Uuid>,
    Json(patch): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let template_id = template_id.to_string();
    let row = sqlx::query("SELECT template_json FROM controller_templates WHERE template_id = ?")
        .bind(&template_id)
        .fetch_optional(state.store.pool())
        .await?;
    let Some(row) = row else {
        return super::update_template_profile(
            State(state),
            AxumPath(Uuid::parse_str(&template_id).unwrap()),
            Json(patch),
        )
        .await;
    };
    let mut template = decode_json(row.get("template_json"))?;
    let patch = patch
        .as_object()
        .ok_or_else(|| ApiError::bad_request("template update body must be an object"))?;
    let target = as_object_mut(&mut template, "stored template must be an object")?;
    for (key, value) in patch {
        if !matches!(key.as_str(), "template_id" | "created_at" | "updated_at") {
            target.insert(key.clone(), value.clone());
        }
    }
    let serialized = serde_json::to_string(&template).map_err(internal_json_error)?;
    sqlx::query("UPDATE controller_templates SET template_json = ?, updated_at = CURRENT_TIMESTAMP WHERE template_id = ?")
        .bind(serialized)
        .bind(&template_id)
        .execute(state.store.pool())
        .await?;
    Ok(Json(template))
}

pub(super) async fn delete_template(
    State(state): State<AppState>,
    AxumPath(template_id): AxumPath<Uuid>,
) -> Result<StatusCode, ApiError> {
    let result = sqlx::query("DELETE FROM controller_templates WHERE template_id = ?")
        .bind(template_id.to_string())
        .execute(state.store.pool())
        .await?;
    if result.rows_affected() == 0 {
        return super::reset_template_profile(State(state), AxumPath(template_id))
            .await
            .map(|_| StatusCode::NO_CONTENT);
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn duplicate_template(
    State(state): State<AppState>,
    AxumPath(template_id): AxumPath<Uuid>,
) -> Result<Response, ApiError> {
    let original = get_template(State(state.clone()), AxumPath(template_id))
        .await?
        .0;
    let mut duplicate = original;
    let id = Uuid::new_v4().to_string();
    let name = duplicate["name"].as_str().unwrap_or("Template").to_owned();
    duplicate["template_id"] = json!(id);
    duplicate["name"] = json!(format!("{name} copy"));
    duplicate["builtin"] = json!(false);
    sqlx::query("INSERT INTO controller_templates (template_id, template_json) VALUES (?, ?)")
        .bind(&id)
        .bind(serde_json::to_string(&duplicate).map_err(internal_json_error)?)
        .execute(state.store.pool())
        .await?;
    Ok((StatusCode::CREATED, Json(duplicate)).into_response())
}
