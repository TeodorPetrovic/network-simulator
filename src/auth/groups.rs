use super::*;

pub(super) async fn list_groups(State(state): State<AppState>) -> Result<Json<Value>, AuthError> {
    let rows = sqlx::query("SELECT * FROM auth_groups ORDER BY name")
        .fetch_all(state.store.pool())
        .await
        .map_err(db_error)?;
    Ok(Json(Value::Array(rows.iter().map(group_json).collect())))
}
pub(super) async fn get_group(
    State(state): State<AppState>,
    Path(group_id): Path<String>,
) -> Result<Json<Value>, AuthError> {
    get_group_value(state.store.pool(), &group_id)
        .await
        .map(Json)
}
pub(super) async fn create_group(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), AuthError> {
    let name = required_string(&body, "name")?;
    if name.len() < 3 {
        return Err(AuthError::bad_request(
            "Group names must contain at least 3 characters.",
        ));
    }
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO auth_groups (user_group_id, name, is_builtin) VALUES (?, ?, 0)")
        .bind(&id)
        .bind(name)
        .execute(state.store.pool())
        .await
        .map_err(db_error)?;
    Ok((
        StatusCode::CREATED,
        Json(get_group_value(state.store.pool(), &id).await?),
    ))
}
pub(super) async fn update_group(
    State(state): State<AppState>,
    Path(group_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AuthError> {
    if let Some(name) = optional_string(&body, "name") {
        sqlx::query("UPDATE auth_groups SET name = ?, updated_at = CURRENT_TIMESTAMP WHERE user_group_id = ? AND is_builtin = 0").bind(name).bind(&group_id).execute(state.store.pool()).await.map_err(db_error)?;
    }
    get_group_value(state.store.pool(), &group_id)
        .await
        .map(Json)
}
pub(super) async fn delete_group(
    State(state): State<AppState>,
    Path(group_id): Path<String>,
) -> Result<StatusCode, AuthError> {
    let result = sqlx::query("DELETE FROM auth_groups WHERE user_group_id = ? AND is_builtin = 0")
        .bind(group_id)
        .execute(state.store.pool())
        .await
        .map_err(db_error)?;
    if result.rows_affected() == 0 {
        return Err(AuthError::not_found("Group not found or is built in."));
    }
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn group_members(
    State(state): State<AppState>,
    Path(group_id): Path<String>,
) -> Result<Json<Value>, AuthError> {
    let exists = sqlx::query_scalar::<_, String>(
        "SELECT user_group_id FROM auth_groups WHERE user_group_id = ?",
    )
    .bind(&group_id)
    .fetch_optional(state.store.pool())
    .await
    .map_err(db_error)?;
    if exists.is_none() {
        return Err(AuthError::not_found("Group not found"));
    }
    let rows = sqlx::query("SELECT u.user_id FROM auth_users u JOIN auth_user_groups m ON m.user_id = u.user_id WHERE m.user_group_id = ? ORDER BY u.username").bind(group_id).fetch_all(state.store.pool()).await.map_err(db_error)?;
    let mut users = Vec::new();
    for row in rows {
        users.push(get_user_value(state.store.pool(), &row.get::<String, _>("user_id")).await?);
    }
    Ok(Json(Value::Array(users)))
}
pub(super) async fn add_group_member(
    State(state): State<AppState>,
    Path((group_id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, AuthError> {
    sqlx::query("INSERT OR IGNORE INTO auth_user_groups (user_id, user_group_id) VALUES (?, ?)")
        .bind(user_id)
        .bind(group_id)
        .execute(state.store.pool())
        .await
        .map_err(db_error)?;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn remove_group_member(
    State(state): State<AppState>,
    Path((group_id, user_id)): Path<(String, String)>,
) -> Result<StatusCode, AuthError> {
    sqlx::query("DELETE FROM auth_user_groups WHERE user_id = ? AND user_group_id = ?")
        .bind(user_id)
        .bind(group_id)
        .execute(state.store.pool())
        .await
        .map_err(db_error)?;
    Ok(StatusCode::NO_CONTENT)
}
pub(super) async fn get_group_value(pool: &SqlitePool, group_id: &str) -> Result<Value, AuthError> {
    let row = sqlx::query("SELECT * FROM auth_groups WHERE user_group_id = ?")
        .bind(group_id)
        .fetch_optional(pool)
        .await
        .map_err(db_error)?
        .ok_or_else(|| AuthError::not_found("Group not found"))?;
    Ok(group_json(&row))
}
pub(super) fn group_json(row: &sqlx::sqlite::SqliteRow) -> Value {
    json!({"user_group_id": row.get::<String,_>("user_group_id"), "name": row.get::<String,_>("name"), "is_builtin": row.get::<bool,_>("is_builtin"), "created_at": row.get::<String,_>("created_at"), "updated_at": row.get::<String,_>("updated_at")})
}
