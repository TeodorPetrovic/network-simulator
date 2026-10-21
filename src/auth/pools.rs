use super::*;

pub(super) async fn list_pools(State(state): State<AppState>) -> Result<Json<Value>, AuthError> {
    let rows = sqlx::query("SELECT * FROM resource_pools ORDER BY name")
        .fetch_all(state.store.pool())
        .await
        .map_err(db_error)?;
    Ok(Json(Value::Array(rows.iter().map(pool_json).collect())))
}
pub(super) async fn get_pool(
    State(state): State<AppState>,
    Path(pool_id): Path<String>,
) -> Result<Json<Value>, AuthError> {
    pool_value(state.store.pool(), &pool_id).await.map(Json)
}
pub(super) async fn create_pool(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), AuthError> {
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO resource_pools (resource_pool_id, name) VALUES (?, ?)")
        .bind(&id)
        .bind(required_string(&body, "name")?)
        .execute(state.store.pool())
        .await
        .map_err(db_error)?;
    Ok((
        StatusCode::CREATED,
        Json(pool_value(state.store.pool(), &id).await?),
    ))
}
pub(super) async fn update_pool(
    State(state): State<AppState>,
    Path(pool_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AuthError> {
    if let Some(name) = optional_string(&body, "name") {
        sqlx::query("UPDATE resource_pools SET name = ?, updated_at = CURRENT_TIMESTAMP WHERE resource_pool_id = ?").bind(name).bind(&pool_id).execute(state.store.pool()).await.map_err(db_error)?;
    }
    pool_value(state.store.pool(), &pool_id).await.map(Json)
}
pub(super) async fn delete_pool(
    State(state): State<AppState>,
    Path(pool_id): Path<String>,
) -> Result<StatusCode, AuthError> {
    let path = format!("/pools/{pool_id}");
    let child_path = format!("{path}/%");
    let ace_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM auth_acl WHERE path = ? OR path LIKE ?")
            .bind(&path)
            .bind(child_path)
            .fetch_one(state.store.pool())
            .await
            .map_err(db_error)?;
    if ace_count > 0 {
        return Err(AuthError::bad_request(
            "Resource pool cannot be deleted while ACL entries refer to it.",
        ));
    }
    let result = sqlx::query("DELETE FROM resource_pools WHERE resource_pool_id = ?")
        .bind(&pool_id)
        .execute(state.store.pool())
        .await
        .map_err(db_error)?;
    if result.rows_affected() == 0 {
        return Err(AuthError::not_found("Resource pool not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn pool_resources(
    State(state): State<AppState>,
    Path(pool_id): Path<String>,
) -> Result<Json<Value>, AuthError> {
    let rows = sqlx::query("SELECT r.* FROM resources r JOIN resource_pool_map m ON m.resource_id = r.resource_id WHERE m.resource_pool_id = ? ORDER BY r.name").bind(pool_id).fetch_all(state.store.pool()).await.map_err(db_error)?;
    Ok(Json(Value::Array(rows.iter().map(|row| json!({"resource_id":row.get::<String,_>("resource_id"), "resource_type":row.get::<String,_>("resource_type"), "name":row.get::<Option<String>,_>("name")})).collect())))
}
pub(super) async fn add_pool_resource(
    State(state): State<AppState>,
    Path((pool_id, resource_id)): Path<(String, String)>,
) -> Result<StatusCode, AuthError> {
    let project = sqlx::query("SELECT project_json FROM projects WHERE project_id = ?")
        .bind(&resource_id)
        .fetch_optional(state.store.pool())
        .await
        .map_err(db_error)?
        .ok_or_else(|| AuthError::not_found(format!("Project '{resource_id}' not found")))?;
    let project: Value = serde_json::from_str(project.get::<String, _>("project_json").as_str())
        .unwrap_or(Value::Null);
    let name = project
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("Project");
    sqlx::query("INSERT OR IGNORE INTO resources (resource_id, resource_type, name) VALUES (?, 'project', ?)").bind(&resource_id).bind(name).execute(state.store.pool()).await.map_err(db_error)?;
    sqlx::query(
        "INSERT OR IGNORE INTO resource_pool_map (resource_id, resource_pool_id) VALUES (?, ?)",
    )
    .bind(resource_id)
    .bind(pool_id)
    .execute(state.store.pool())
    .await
    .map_err(db_error)?;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn remove_pool_resource(
    State(state): State<AppState>,
    Path((pool_id, resource_id)): Path<(String, String)>,
) -> Result<StatusCode, AuthError> {
    sqlx::query("DELETE FROM resource_pool_map WHERE resource_id = ? AND resource_pool_id = ?")
        .bind(&resource_id)
        .bind(pool_id)
        .execute(state.store.pool())
        .await
        .map_err(db_error)?;
    sqlx::query("DELETE FROM resources WHERE resource_id = ? AND NOT EXISTS (SELECT 1 FROM resource_pool_map WHERE resource_id = ?)").bind(&resource_id).bind(&resource_id).execute(state.store.pool()).await.map_err(db_error)?;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn pool_value(pool: &SqlitePool, pool_id: &str) -> Result<Value, AuthError> {
    let row = sqlx::query("SELECT * FROM resource_pools WHERE resource_pool_id = ?")
        .bind(pool_id)
        .fetch_optional(pool)
        .await
        .map_err(db_error)?
        .ok_or_else(|| AuthError::not_found("Resource pool not found"))?;
    Ok(pool_json(&row))
}
pub(super) fn pool_json(row: &sqlx::sqlite::SqliteRow) -> Value {
    json!({"resource_pool_id":row.get::<String,_>("resource_pool_id"),"name":row.get::<String,_>("name"),"created_at":row.get::<String,_>("created_at"),"updated_at":row.get::<String,_>("updated_at")})
}
