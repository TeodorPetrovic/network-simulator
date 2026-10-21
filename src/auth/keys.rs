use super::*;

#[derive(Deserialize)]
pub(super) struct ApiKeyCreate {
    name: String,
}
pub(super) async fn create_api_key(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Json(body): Json<ApiKeyCreate>,
) -> Result<(StatusCode, Json<Value>), AuthError> {
    let id = Uuid::new_v4().to_string();
    let secret = random_hex(32).map_err(|_| {
        AuthError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not create API key",
        )
    })?;
    let hash = hash_password_limited(&secret).await?;
    let key = format!("gns3_{id}_{secret}");
    let prefix = key.chars().take(8).collect::<String>();
    sqlx::query("INSERT INTO auth_api_keys (api_key_id,user_id,name,key_prefix,secret_hash) VALUES (?,?,?,?,?)").bind(&id).bind(principal.user_id).bind(&body.name).bind(&prefix).bind(hash).execute(state.store.pool()).await.map_err(db_error)?;
    let created_at: String =
        sqlx::query_scalar("SELECT created_at FROM auth_api_keys WHERE api_key_id = ?")
            .bind(&id)
            .fetch_one(state.store.pool())
            .await
            .map_err(db_error)?;
    Ok((
        StatusCode::CREATED,
        Json(
            json!({"api_key_id":id,"api_key":key,"name":body.name,"key_prefix":prefix,"created_at":created_at}),
        ),
    ))
}
pub(super) async fn list_api_keys(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<Principal>,
) -> Result<Json<Value>, AuthError> {
    let rows = sqlx::query("SELECT api_key_id,name,key_prefix,revoked,created_at,last_used_at FROM auth_api_keys WHERE user_id = ? ORDER BY created_at DESC").bind(principal.user_id).fetch_all(state.store.pool()).await.map_err(db_error)?;
    Ok(Json(Value::Array(rows.iter().map(|row| json!({"api_key_id":row.get::<String,_>("api_key_id"),"name":row.get::<String,_>("name"),"key_prefix":row.get::<String,_>("key_prefix"),"revoked":row.get::<bool,_>("revoked"),"created_at":row.get::<String,_>("created_at"),"last_used_at":row.get::<Option<String>,_>("last_used_at")})).collect())))
}
pub(super) async fn set_api_key_revoked(
    state: &AppState,
    principal: &Principal,
    key_id: &str,
    revoked: bool,
) -> Result<(), AuthError> {
    let result =
        sqlx::query("UPDATE auth_api_keys SET revoked = ? WHERE api_key_id = ? AND user_id = ?")
            .bind(revoked)
            .bind(key_id)
            .bind(&principal.user_id)
            .execute(state.store.pool())
            .await
            .map_err(db_error)?;
    if result.rows_affected() == 0 {
        return Err(AuthError::not_found("API key not found"));
    }
    Ok(())
}
pub(super) async fn revoke_api_key(
    State(state): State<AppState>,
    Path(key_id): Path<String>,
    axum::Extension(principal): axum::Extension<Principal>,
) -> Result<Json<Value>, AuthError> {
    set_api_key_revoked(&state, &principal, &key_id, true).await?;
    Ok(Json(json!({"message":"API key revoked"})))
}
pub(super) async fn restore_api_key(
    State(state): State<AppState>,
    Path(key_id): Path<String>,
    axum::Extension(principal): axum::Extension<Principal>,
) -> Result<Json<Value>, AuthError> {
    set_api_key_revoked(&state, &principal, &key_id, false).await?;
    Ok(Json(json!({"message":"API key restored"})))
}
pub(super) async fn delete_api_key(
    State(state): State<AppState>,
    Path(key_id): Path<String>,
    axum::Extension(principal): axum::Extension<Principal>,
) -> Result<StatusCode, AuthError> {
    let result = sqlx::query("DELETE FROM auth_api_keys WHERE api_key_id = ? AND user_id = ?")
        .bind(key_id)
        .bind(principal.user_id)
        .execute(state.store.pool())
        .await
        .map_err(db_error)?;
    if result.rows_affected() == 0 {
        return Err(AuthError::not_found("API key not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub(super) struct AuthSettingsUpdate {
    auth_enabled: bool,
}
pub(super) async fn auth_status(
    State(state): State<AppState>,
    request: Request<Body>,
) -> Result<Json<Value>, AuthError> {
    let persisted =
        sqlx::query_scalar::<_, bool>("SELECT auth_enabled FROM auth_config WHERE singleton = 1")
            .fetch_optional(state.store.pool())
            .await
            .map_err(db_error)?
            .ok_or_else(|| {
                AuthError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Authentication configuration is missing.",
                )
            })?;
    let enabled = request_requires_auth(
        persisted,
        !state.bind_address.ip().is_loopback(),
        auth_override() == Some(false),
        request_has_forwarded_headers(request.headers()),
        trust_proxy_headers_unauthenticated(),
    );
    Ok(Json(json!({"auth_enabled":enabled})))
}
pub(super) async fn get_auth_settings(
    State(state): State<AppState>,
    request: Request<Body>,
) -> Result<Json<Value>, AuthError> {
    auth_status(State(state), request).await
}
pub(super) async fn update_auth_settings(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Json(body): Json<AuthSettingsUpdate>,
) -> Result<Json<Value>, AuthError> {
    if !principal.is_superadmin {
        return Err(AuthError::forbidden(
            "Only a super administrator can change authentication settings.",
        ));
    }
    let enabled = body.auth_enabled
        || (!state.bind_address.ip().is_loopback() && auth_override() != Some(false));
    sqlx::query("UPDATE auth_config SET auth_enabled = ? WHERE singleton = 1")
        .bind(enabled)
        .execute(state.store.pool())
        .await
        .map_err(db_error)?;
    Ok(Json(json!({"auth_enabled":enabled})))
}
