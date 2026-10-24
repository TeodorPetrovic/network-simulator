use super::*;

#[derive(Deserialize)]
pub(super) struct LoginForm {
    username: String,
    password: String,
}
#[derive(Deserialize)]
pub(super) struct LoginJson {
    username: String,
    password: String,
}
#[derive(Deserialize)]
pub(super) struct RefreshBody {
    refresh_token: String,
}

pub(super) async fn login(
    State(state): State<AppState>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    Form(form): Form<LoginForm>,
) -> Result<Json<Value>, AuthError> {
    let ip = connect_info
        .map(|Extension(ConnectInfo(address))| address.ip().to_string())
        .unwrap_or_else(|| "unknown".to_owned());
    token_for_credentials(state.store.pool(), &form.username, &form.password, &ip)
        .await
        .map(Json)
}
pub(super) async fn authenticate(
    State(state): State<AppState>,
    connect_info: Option<Extension<ConnectInfo<std::net::SocketAddr>>>,
    Json(body): Json<LoginJson>,
) -> Result<Json<Value>, AuthError> {
    let ip = connect_info
        .map(|Extension(ConnectInfo(address))| address.ip().to_string())
        .unwrap_or_else(|| "unknown".to_owned());
    token_for_credentials(state.store.pool(), &body.username, &body.password, &ip)
        .await
        .map(Json)
}
pub(super) async fn token_for_credentials(
    pool: &SqlitePool,
    username: &str,
    password: &str,
    client_ip: &str,
) -> Result<Value, AuthError> {
    let key = login_rate_key(username, client_ip);
    if login_rate_limited(&key) {
        return Err(AuthError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Authentication was unsuccessful.",
        ));
    }
    let row = sqlx::query("SELECT user_id, username, password_hash, is_active, is_superadmin, must_change_password, token_version FROM auth_users WHERE username = ?")
        .bind(username).fetch_optional(pool).await.map_err(db_error)?;
    let (valid, row) = match row {
        Some(row) if row.get::<bool, _>("is_active") => {
            let valid = verify_password_limited(password, row.get("password_hash")).await?;
            (valid, Some(row))
        }
        _ => {
            let dummy_hash = dummy_password_hash().await?;
            let _ = verify_password_limited(password, &dummy_hash).await?;
            (false, None)
        }
    };
    if !valid {
        record_login_failure(key);
        return Err(AuthError::unauthorized());
    }
    clear_login_failure(&key);
    let row = row.expect("valid credentials have an active user row");
    let id: String = row.get("user_id");
    let token_version = row.get::<i64, _>("token_version");
    let key = signing_key(pool).await.ok_or_else(|| {
        AuthError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "JWT signing key is not initialized",
        )
    })?;
    let mut tx = pool.begin().await.map_err(db_error)?;
    sqlx::query("UPDATE auth_users SET last_login = CURRENT_TIMESTAMP WHERE user_id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    let family_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO auth_refresh_families (family_id, user_id, token_version) VALUES (?, ?, ?)",
    )
    .bind(&family_id)
    .bind(&id)
    .bind(token_version)
    .execute(&mut *tx)
    .await
    .map_err(db_error)?;
    let access = encode_token(
        &new_claims(username, token_version, "access", ACCESS_TTL),
        &key,
    )?;
    let refresh_claims = new_claims(username, token_version, "refresh", REFRESH_TTL);
    let refresh = encode_token(&refresh_claims, &key)?;
    sqlx::query("INSERT INTO auth_refresh_tokens (jti, family_id, expires_at) VALUES (?, ?, ?)")
        .bind(&refresh_claims.jti)
        .bind(&family_id)
        .bind(refresh_claims.exp as i64)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    Ok(json!({
        "access_token": access,
        "token_type": "bearer",
        "expires_in": ACCESS_TTL,
        "refresh_token": refresh,
        "refresh_expires_in": REFRESH_TTL,
        "must_change_password": row.get::<bool, _>("must_change_password"),
    }))
}

pub(super) async fn refresh(
    State(state): State<AppState>,
    Json(body): Json<RefreshBody>,
) -> Result<Json<Value>, AuthError> {
    let key = signing_key(state.store.pool())
        .await
        .ok_or_else(AuthError::unauthorized)?;
    let claims = decode_token(&body.refresh_token, &key)
        .filter(|claims| claims.token_use == "refresh")
        .ok_or_else(AuthError::unauthorized)?;
    let mut tx = state.store.pool().begin().await.map_err(db_error)?;
    let now = now_epoch() as i64;
    let update = sqlx::query("UPDATE auth_refresh_tokens SET consumed_at = CURRENT_TIMESTAMP WHERE jti = ? AND consumed_at IS NULL AND expires_at > ?")
        .bind(&claims.jti)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    if update.rows_affected() == 0 {
        let prior =
            sqlx::query("SELECT family_id, consumed_at FROM auth_refresh_tokens WHERE jti = ?")
                .bind(&claims.jti)
                .fetch_optional(&mut *tx)
                .await
                .map_err(db_error)?;
        if let Some(prior) = prior
            && prior.get::<Option<String>, _>("consumed_at").is_some()
        {
            sqlx::query("UPDATE auth_refresh_families SET revoked = 1 WHERE family_id = ?")
                .bind(prior.get::<String, _>("family_id"))
                .execute(&mut *tx)
                .await
                .map_err(db_error)?;
            tx.commit().await.map_err(db_error)?;
        }
        return Err(AuthError::unauthorized());
    }
    let row = sqlx::query("SELECT f.family_id, f.user_id, f.token_version, f.revoked, u.username, u.is_active, u.token_version AS current_version, u.must_change_password FROM auth_refresh_tokens t JOIN auth_refresh_families f ON f.family_id = t.family_id JOIN auth_users u ON u.user_id = f.user_id WHERE t.jti = ?")
        .bind(&claims.jti)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_error)?
        .ok_or_else(AuthError::unauthorized)?;
    if row.get::<bool, _>("revoked")
        || !row.get::<bool, _>("is_active")
        || row.get::<i64, _>("token_version") != claims.ver
        || row.get::<i64, _>("current_version") != claims.ver
        || row.get::<String, _>("username") != claims.sub
    {
        sqlx::query("UPDATE auth_refresh_families SET revoked = 1 WHERE family_id = ?")
            .bind(row.get::<String, _>("family_id"))
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        return Err(AuthError::unauthorized());
    }
    let family_id: String = row.get("family_id");
    let user_id: String = row.get("user_id");
    let username: String = row.get("username");
    let version: i64 = row.get("token_version");
    let access = encode_token(&new_claims(&username, version, "access", ACCESS_TTL), &key)?;
    let refresh_claims = new_claims(&username, version, "refresh", REFRESH_TTL);
    let refresh = encode_token(&refresh_claims, &key)?;
    sqlx::query("INSERT INTO auth_refresh_tokens (jti, family_id, expires_at) VALUES (?, ?, ?)")
        .bind(&refresh_claims.jti)
        .bind(&family_id)
        .bind(refresh_claims.exp as i64)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    let _ = user_id;
    tx.commit().await.map_err(db_error)?;
    Ok(Json(json!({
        "access_token": access,
        "token_type": "bearer",
        "expires_in": ACCESS_TTL,
        "refresh_token": refresh,
        "refresh_expires_in": REFRESH_TTL,
        "must_change_password": row.get::<bool, _>("must_change_password"),
    })))
}

#[derive(Deserialize)]
pub(super) struct WsTicketRequest {
    path: String,
}

pub(super) async fn create_ws_ticket(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Json(body): Json<WsTicketRequest>,
) -> Result<Json<Value>, AuthError> {
    if !body.path.starts_with("/v3/")
        || body.path.contains('?')
        || body.path.contains('#')
        || !is_ticket_endpoint(&body.path)
    {
        return Err(AuthError::bad_request(
            "A ticket can only be issued for a registered WebSocket or capture stream path.",
        ));
    }
    let normalized = body.path.strip_prefix("/v3").unwrap_or(&body.path);
    if !matches!(
        classify_route("GET", normalized),
        Some(RouteClass::Privilege(_))
    ) {
        return Err(AuthError::bad_request(
            "The requested ticket path is not registered.",
        ));
    }
    let required = match classify_route("GET", normalized) {
        Some(RouteClass::Privilege(privilege)) => privilege,
        _ => {
            return Err(AuthError::bad_request(
                "The requested ticket path is not registered.",
            ));
        }
    };
    if !has_privilege(state.store.pool(), &principal, normalized, required).await {
        return Err(AuthError::forbidden(format!(
            "Permission denied (privilege {required} is required)"
        )));
    }
    let user = sqlx::query("SELECT token_version, is_active FROM auth_users WHERE user_id = ?")
        .bind(&principal.user_id)
        .fetch_optional(state.store.pool())
        .await
        .map_err(db_error)?
        .ok_or_else(AuthError::unauthorized)?;
    if !user.get::<bool, _>("is_active") {
        return Err(AuthError::unauthorized());
    }
    let mut random = [0_u8; 32];
    SystemRandom::new().fill(&mut random).map_err(|_| {
        AuthError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not create access ticket.",
        )
    })?;
    let ticket = URL_SAFE_NO_PAD.encode(random);
    let ticket_hash = hash_ticket(&ticket);
    let expires_at = now_epoch() + WS_TICKET_TTL;
    sqlx::query("INSERT INTO auth_ws_tickets (ticket_hash, user_id, token_version, path, expires_at) VALUES (?, ?, ?, ?, ?)")
        .bind(ticket_hash)
        .bind(&principal.user_id)
        .bind(user.get::<i64, _>("token_version"))
        .bind(&body.path)
        .bind(expires_at as i64)
        .execute(state.store.pool())
        .await
        .map_err(db_error)?;
    Ok(Json(json!({"ticket":ticket,"expires_in":WS_TICKET_TTL})))
}

fn hash_ticket(ticket: &str) -> String {
    URL_SAFE_NO_PAD.encode(digest::digest(&digest::SHA256, ticket.as_bytes()).as_ref())
}

pub(super) async fn consume_ws_ticket(
    pool: &SqlitePool,
    ticket: &str,
    path: &str,
) -> Result<Option<Principal>, sqlx::Error> {
    let mut tx: Transaction<'_, Sqlite> = pool.begin().await?;
    let result = sqlx::query("UPDATE auth_ws_tickets SET used_at = CURRENT_TIMESTAMP WHERE ticket_hash = ? AND path = ? AND used_at IS NULL AND expires_at > ?")
        .bind(hash_ticket(ticket))
        .bind(path)
        .bind(now_epoch() as i64)
        .execute(&mut *tx)
        .await?;
    if result.rows_affected() == 0 {
        return Ok(None);
    }
    let row = sqlx::query("SELECT t.token_version AS ticket_version, u.user_id, u.token_version AS current_version, u.is_active, u.is_superadmin, u.must_change_password FROM auth_ws_tickets t JOIN auth_users u ON u.user_id = t.user_id WHERE t.ticket_hash = ?")
        .bind(hash_ticket(ticket))
        .fetch_optional(&mut *tx)
        .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let principal = if row.get::<bool, _>("is_active")
        && row.get::<i64, _>("ticket_version") == row.get::<i64, _>("current_version")
    {
        Some(Principal {
            user_id: row.get("user_id"),
            is_superadmin: row.get("is_superadmin"),
            must_change_password: row.get("must_change_password"),
        })
    } else {
        None
    };
    tx.commit().await?;
    Ok(principal)
}

pub(super) async fn logout(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<Principal>,
) -> Result<StatusCode, AuthError> {
    sqlx::query("UPDATE auth_users SET token_version = token_version + 1 WHERE user_id = ?")
        .bind(principal.user_id)
        .execute(state.store.pool())
        .await
        .map_err(db_error)?;
    Ok(StatusCode::NO_CONTENT)
}

