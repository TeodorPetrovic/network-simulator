use super::*;

pub(super) async fn list_users(State(state): State<AppState>) -> Result<Json<Value>, AuthError> {
    let rows = sqlx::query("SELECT user_id FROM auth_users ORDER BY username")
        .fetch_all(state.store.pool())
        .await
        .map_err(db_error)?;
    let mut users = Vec::new();
    for row in rows {
        users.push(get_user_value(state.store.pool(), &row.get::<String, _>("user_id")).await?);
    }
    Ok(Json(Value::Array(users)))
}

pub(super) async fn get_user(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> Result<Json<Value>, AuthError> {
    get_user_value(state.store.pool(), &user_id).await.map(Json)
}

pub(super) async fn create_user(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), AuthError> {
    let username = required_string(&body, "username")?;
    validate_username(&username)?;
    let password = required_string(&body, "password")?;
    if password.len() < 8 || password.len() > 100 {
        return Err(AuthError::bad_request(
            "Password must be between 8 and 100 characters.",
        ));
    }
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO auth_users (user_id, username, email, full_name, password_hash, is_active, is_superadmin) VALUES (?, ?, ?, ?, ?, ?, 0)")
        .bind(&id).bind(username).bind(optional_string(&body, "email")).bind(optional_string(&body, "full_name"))
        .bind(hash_password_limited(&password).await?).bind(body.get("is_active").and_then(Value::as_bool).unwrap_or(true))
        .execute(state.store.pool()).await.map_err(db_error)?;
    Ok((
        StatusCode::CREATED,
        Json(get_user_value(state.store.pool(), &id).await?),
    ))
}

pub(super) async fn update_user(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AuthError> {
    update_user_row(state.store.pool(), &user_id, &body, false).await?;
    get_user_value(state.store.pool(), &user_id).await.map(Json)
}

pub(super) async fn update_user_row(
    pool: &SqlitePool,
    user_id: &str,
    body: &Value,
    self_update: bool,
) -> Result<(), AuthError> {
    if sqlx::query_scalar::<_, String>("SELECT user_id FROM auth_users WHERE user_id = ?")
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .map_err(db_error)?
        .is_none()
    {
        return Err(AuthError::not_found(format!("User '{user_id}' not found")));
    }
    if let Some(username) = optional_string(body, "username") {
        if self_update {
            return Err(AuthError::bad_request(
                "Username cannot be changed through /me.",
            ));
        }
        validate_username(&username)?;
        sqlx::query(
            "UPDATE auth_users SET username = ?, updated_at = CURRENT_TIMESTAMP WHERE user_id = ?",
        )
        .bind(username)
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(db_error)?;
    }
    if let Some(email) = optional_string(body, "email") {
        sqlx::query(
            "UPDATE auth_users SET email = ?, updated_at = CURRENT_TIMESTAMP WHERE user_id = ?",
        )
        .bind(email)
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(db_error)?;
    }
    if let Some(full_name) = optional_string(body, "full_name") {
        sqlx::query(
            "UPDATE auth_users SET full_name = ?, updated_at = CURRENT_TIMESTAMP WHERE user_id = ?",
        )
        .bind(full_name)
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(db_error)?;
    }
    if let Some(active) = body.get("is_active").and_then(Value::as_bool) {
        if self_update && !active {
            return Err(AuthError::bad_request(
                "You cannot disable your own account.",
            ));
        }
        let result = sqlx::query("UPDATE auth_users SET is_active = ?, updated_at = CURRENT_TIMESTAMP WHERE user_id = ? AND (? = 1 OR is_superadmin = 0 OR (SELECT COUNT(*) FROM auth_users WHERE is_superadmin = 1 AND is_active = 1) > 1)")
        .bind(active)
        .bind(user_id)
        .bind(active)
        .execute(pool)
        .await
        .map_err(db_error)?;
        if result.rows_affected() == 0 {
            let is_active_admin: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM auth_users WHERE user_id = ? AND is_superadmin = 1 AND is_active = 1)")
                .bind(user_id)
                .fetch_one(pool)
                .await
                .map_err(db_error)?;
            if !active && is_active_admin {
                return Err(AuthError::forbidden(
                    "The last active super administrator cannot be disabled.",
                ));
            }
        }
    }
    if let Some(password) = optional_string(body, "password") {
        if password.len() < 8 || password.len() > 100 {
            return Err(AuthError::bad_request(
                "Password must be between 8 and 100 characters.",
            ));
        }
        sqlx::query("UPDATE auth_users SET password_hash = ?, must_change_password = 0, token_version = token_version + 1, updated_at = CURRENT_TIMESTAMP WHERE user_id = ?")
            .bind(hash_password_limited(&password).await?).bind(user_id).execute(pool).await.map_err(db_error)?;
    }
    Ok(())
}

pub(super) async fn delete_user(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
    axum::Extension(principal): axum::Extension<Principal>,
) -> Result<StatusCode, AuthError> {
    if user_id == principal.user_id {
        return Err(AuthError::bad_request(
            "You cannot delete your own account.",
        ));
    }
    let result = sqlx::query("DELETE FROM auth_users WHERE user_id = ? AND is_superadmin = 0")
        .bind(user_id)
        .execute(state.store.pool())
        .await
        .map_err(db_error)?;
    if result.rows_affected() == 0 {
        return Err(AuthError::not_found(
            "User not found or is a protected administrator.",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn get_user_value(pool: &SqlitePool, user_id: &str) -> Result<Value, AuthError> {
    let row = sqlx::query("SELECT user_id, username, email, full_name, is_active, is_superadmin, must_change_password, last_login, created_at, updated_at FROM auth_users WHERE user_id = ?")
        .bind(user_id).fetch_optional(pool).await.map_err(db_error)?.ok_or_else(|| AuthError::not_found(format!("User '{user_id}' not found")))?;
    Ok(
        json!({"user_id": row.get::<String,_>("user_id"), "username": row.get::<String,_>("username"), "email": row.get::<Option<String>,_>("email"), "full_name": row.get::<Option<String>,_>("full_name"), "is_active": row.get::<bool,_>("is_active"), "is_superadmin": row.get::<bool,_>("is_superadmin"), "must_change_password": row.get::<bool,_>("must_change_password"), "last_login": row.get::<Option<String>,_>("last_login"), "created_at": row.get::<String,_>("created_at"), "updated_at": row.get::<String,_>("updated_at")}),
    )
}

pub(super) async fn user_memberships(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> Result<Json<Value>, AuthError> {
    let rows = sqlx::query("SELECT g.user_group_id, g.name, g.is_builtin, g.created_at, g.updated_at FROM auth_groups g JOIN auth_user_groups m ON m.user_group_id = g.user_group_id WHERE m.user_id = ? ORDER BY g.name")
        .bind(user_id).fetch_all(state.store.pool()).await.map_err(db_error)?;
    Ok(Json(Value::Array(rows.iter().map(group_json).collect())))
}
