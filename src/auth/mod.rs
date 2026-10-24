use std::{
    collections::HashMap,
    fs::OpenOptions,
    io::Write,
    path::Path as FsPath,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::{
    Json, Router,
    body::Body,
    extract::{ConnectInfo, Extension, Form, Path, State},
    http::{Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::{
    digest, hmac,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use tokio::sync::Semaphore;
use tower_http::limit::RequestBodyLimitLayer;
use uuid::Uuid;

use super::AppState;

use auth_mode::{auth_override, effective_auth_mode, request_requires_auth};
pub(crate) use auth_mode::parse_bool;
use gate::{auth_gate, has_privilege};
use gate::{is_ticket_endpoint, request_has_forwarded_headers, trust_proxy_headers_unauthenticated};
use groups::*;
use keys::*;
use pools::*;
use rbac::seed_rbac;
use roles::*;
use tokens::{authenticate_token, decode_token, encode_token, new_claims, now_epoch, random_hex, signing_key};
#[cfg(test)]
use tokens::Claims;
use route_class::classify_route;
use route_rules::RouteClass;
#[cfg(test)]
use route_rules::{RouteRule, ROUTE_RULES};
use users::*;

mod acl;
mod auth_mode;
mod gate;
mod groups;
mod keys;
mod pools;
mod rbac;
mod route_class;
mod route_rules;
mod roles;
mod users;
mod tokens;

const ACCESS_TTL: u64 = 60 * 60;
const REFRESH_TTL: u64 = 7 * 24 * 60 * 60;
const WS_TICKET_TTL: u64 = 60;
const HASH_CONCURRENCY: usize = 4;
const LOGIN_BACKOFF_BASE: Duration = Duration::from_millis(250);
const LOGIN_BACKOFF_MAX: Duration = Duration::from_secs(30);
const JWT_ISSUER: &str = "network-simulator";
const JWT_AUDIENCE: &str = "network-simulator-api";

static PASSWORD_HASH_SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
static LOGIN_ATTEMPTS: OnceLock<Mutex<HashMap<String, LoginAttempt>>> = OnceLock::new();
static WARNED_FORWARDED_AUTH: AtomicBool = AtomicBool::new(false);
static DUMMY_PASSWORD_HASH: OnceLock<String> = OnceLock::new();

#[derive(Clone, Copy)]
struct LoginAttempt {
    failures: u32,
    next_allowed: Instant,
    last_seen: Instant,
}

#[derive(Clone, Debug)]
pub(crate) struct Principal {
    user_id: String,
    is_superadmin: bool,
    must_change_password: bool,
}

#[derive(Debug)]
struct AuthError {
    status: StatusCode,
    message: String,
}

impl AuthError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, message)
    }
    fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "Authentication was unsuccessful.")
    }
    fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, message)
    }
    fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, message)
    }
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let mut response = (
            self.status,
            Json(json!({"message": self.message, "detail": self.message})),
        )
            .into_response();
        if self.status == StatusCode::UNAUTHORIZED {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                "Bearer".parse().expect("static header"),
            );
        }
        response
    }
}

pub(crate) async fn initialize(
    pool: &SqlitePool,
    auth_enabled: bool,
    initial_password_path: &FsPath,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let key = random_hex(64)?;
    sqlx::query("INSERT OR IGNORE INTO auth_config (singleton, auth_enabled, jwt_signing_key) VALUES (1, ?, ?)")
        .bind(auth_enabled)
        .bind(key)
        .execute(pool).await?;
    seed_rbac(pool).await?;
    let _ = dummy_password_hash()
        .await
        .map_err(|error| std::io::Error::other(error.message))?;
    let active_admin: Option<String> = sqlx::query_scalar(
        "SELECT user_id FROM auth_users WHERE is_superadmin = 1 AND is_active = 1 LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    if active_admin.is_none() {
        let password = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let hash = hash_password_limited(&password)
            .await
            .map_err(|error| std::io::Error::other(error.message))?;
        write_initial_password(initial_password_path, &password)?;
        let existing_admin = sqlx::query(
            "SELECT user_id, username FROM auth_users WHERE is_superadmin = 1 ORDER BY created_at LIMIT 1",
        )
        .fetch_optional(pool)
        .await?;
        let (user_id, username) = if let Some(row) = existing_admin {
            let user_id: String = row.get("user_id");
            let username: String = row.get("username");
            sqlx::query("UPDATE auth_users SET password_hash = ?, is_active = 1, must_change_password = 1, token_version = token_version + 1, updated_at = CURRENT_TIMESTAMP WHERE user_id = ?")
                .bind(hash)
                .bind(&user_id)
                .execute(pool)
                .await?;
            (user_id, username)
        } else {
            let user_id = Uuid::new_v4().to_string();
            sqlx::query("INSERT INTO auth_users (user_id, username, full_name, password_hash, is_superadmin, must_change_password) VALUES (?, 'admin', 'Super Administrator', ?, 1, 1)")
                .bind(&user_id).bind(hash).execute(pool).await?;
            (user_id, "admin".to_owned())
        };
        if let Some(group_id) = sqlx::query_scalar::<_, String>(
            "SELECT user_group_id FROM auth_groups WHERE name = 'Administrators'",
        )
        .fetch_optional(pool)
        .await?
        {
            sqlx::query(
                "INSERT OR IGNORE INTO auth_user_groups (user_id, user_group_id) VALUES (?, ?)",
            )
            .bind(&user_id)
            .bind(group_id)
            .execute(pool)
            .await?;
        }
        tracing::warn!(%username, password_file = %initial_password_path.display(), "Created or recovered an active bootstrap administrator; change the initial password after login");
    }
    Ok(())
}

fn write_initial_password(path: &FsPath, password: &str) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(0o600);
        let mut file = options.open(path)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        writeln!(file, "{password}")?;
        file.sync_all()
    }
    #[cfg(not(unix))]
    {
        let mut file = options.open(path)?;
        writeln!(file, "{password}")?;
        file.sync_all()
    }
}

pub(crate) async fn enforce_bind_auth_floor(
    pool: &SqlitePool,
    bind_is_non_loopback: bool,
) -> Result<bool, sqlx::Error> {
    enforce_auth_mode(pool, bind_is_non_loopback, auth_override()).await
}

async fn enforce_auth_mode(
    pool: &SqlitePool,
    bind_is_non_loopback: bool,
    override_value: Option<bool>,
) -> Result<bool, sqlx::Error> {
    let persisted =
        sqlx::query_scalar::<_, bool>("SELECT auth_enabled FROM auth_config WHERE singleton = 1")
            .fetch_optional(pool)
            .await?
            .ok_or(sqlx::Error::RowNotFound)?;
    let explicit_disable = override_value == Some(false);
    let enabled = effective_auth_mode(persisted, bind_is_non_loopback, explicit_disable);
    if enabled != persisted {
        set_auth_enabled(pool, enabled).await?;
    }
    if enabled && !persisted && bind_is_non_loopback && !explicit_disable {
        tracing::warn!(
            "authentication was enabled because the controller bind address is non-loopback"
        );
    }
    if bind_is_non_loopback && explicit_disable {
        tracing::error!(
            "AUTHENTICATION IS DISABLED ON A NON-LOOPBACK BIND BY NETWORK_SIMULATOR_AUTH_ENABLED=false"
        );
    }
    Ok(enabled)
}

pub(crate) async fn set_auth_enabled(pool: &SqlitePool, enabled: bool) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE auth_config SET auth_enabled = ? WHERE singleton = 1")
        .bind(enabled)
        .execute(pool)
        .await?;
    Ok(())
}

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/v3/access/users/login", post(login))
        .route("/v3/access/users/authenticate", post(authenticate))
        .route("/v3/access/users/refresh", post(refresh))
        .route("/v3/access/ws-ticket", post(create_ws_ticket))
        .route("/v3/access/users/logout", post(logout))
        .route("/v3/access/users/me", get(me).put(update_me))
        .route("/v3/access/users", get(list_users).post(create_user))
        .route(
            "/v3/access/users/{user_id}",
            get(get_user).put(update_user).delete(delete_user),
        )
        .route("/v3/access/users/{user_id}/groups", get(user_memberships))
        .route("/v3/access/groups", get(list_groups).post(create_group))
        .route(
            "/v3/access/groups/{group_id}",
            get(get_group).put(update_group).delete(delete_group),
        )
        .route("/v3/access/groups/{group_id}/members", get(group_members))
        .route(
            "/v3/access/groups/{group_id}/members/{user_id}",
            put(add_group_member).delete(remove_group_member),
        )
        .route("/v3/access/roles", get(list_roles).post(create_role))
        .route(
            "/v3/access/roles/{role_id}",
            get(get_role).put(update_role).delete(delete_role),
        )
        .route(
            "/v3/access/roles/{role_id}/privileges",
            get(role_privileges),
        )
        .route(
            "/v3/access/roles/{role_id}/privileges/{privilege_id}",
            put(add_role_privilege).delete(remove_role_privilege),
        )
        .route("/v3/access/privileges", get(list_privileges))
        .route("/v3/access/acl", get(list_acl).post(create_acl))
        .route("/v3/access/acl/endpoints", get(acl_endpoints))
        .route(
            "/v3/access/acl/{ace_id}",
            get(get_acl).put(update_acl).delete(delete_acl),
        )
        .route("/v3/pools", get(list_pools).post(create_pool))
        .route(
            "/v3/pools/{pool_id}",
            get(get_pool).put(update_pool).delete(delete_pool),
        )
        .route("/v3/pools/{pool_id}/resources", get(pool_resources))
        .route(
            "/v3/pools/{pool_id}/resources/{resource_id}",
            put(add_pool_resource).delete(remove_pool_resource),
        )
        .route(
            "/v3/access/api-keys",
            get(list_api_keys).post(create_api_key),
        )
        .route("/v3/access/api-keys/{key_id}/revoke", post(revoke_api_key))
        .route(
            "/v3/access/api-keys/{key_id}/restore",
            post(restore_api_key),
        )
        .route("/v3/access/api-keys/{key_id}", delete(delete_api_key))
        .route("/v3/access/settings/status", get(auth_status))
        .route(
            "/v3/access/settings",
            get(get_auth_settings).put(update_auth_settings),
        )
        .layer(RequestBodyLimitLayer::new(2 * 1024 * 1024))
}

pub(crate) fn protect(router: Router<AppState>, state: AppState) -> Router<AppState> {
    router.layer(middleware::from_fn_with_state(state, auth_gate))
}


pub(crate) async fn can_audit_project(
    pool: &SqlitePool,
    principal: &Principal,
    project_id: &str,
) -> bool {
    has_privilege(
        pool,
        principal,
        &format!("/projects/{project_id}"),
        "Project.Audit",
    )
    .await
}

#[cfg(test)]
fn evaluate_acl(path: &str, aces: &[(String, bool, bool)]) -> Option<bool> {
    acl::evaluate_acl(path, aces)
}

fn normalize_acl_path(path: &str) -> String {
    let mut path = path.trim().to_owned();
    if path.is_empty() {
        return "/".into();
    }
    if !path.starts_with('/') {
        path.insert(0, '/');
    }
    while path.len() > 1 && path.ends_with('/') {
        path.pop();
    }
    path
}

fn url_query_value(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| value.replace('+', " "))
    })
}


fn hash_password(password: &str) -> Result<String, AuthError> {
    let salt = random_hex(16).map_err(|_| {
        AuthError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not create password salt",
        )
    })?;
    let salt = SaltString::encode_b64(
        &hex_decode(&salt).map_err(|_| AuthError::bad_request("Invalid salt"))?,
    )
    .map_err(|_| {
        AuthError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not encode password salt",
        )
    })?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| AuthError::new(StatusCode::INTERNAL_SERVER_ERROR, "Could not hash password"))
}

fn password_hash_slots() -> Arc<Semaphore> {
    PASSWORD_HASH_SLOTS
        .get_or_init(|| Arc::new(Semaphore::new(HASH_CONCURRENCY)))
        .clone()
}

async fn hash_password_limited(password: &str) -> Result<String, AuthError> {
    let permit = password_hash_slots().acquire_owned().await.map_err(|_| {
        AuthError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Password hashing is unavailable.",
        )
    })?;
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        hash_password(&password)
    })
    .await
    .map_err(|_| {
        AuthError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Password hashing failed.",
        )
    })?
}

async fn verify_password_limited(password: &str, hash: &str) -> Result<bool, AuthError> {
    let permit = password_hash_slots().acquire_owned().await.map_err(|_| {
        AuthError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "Password verification is unavailable.",
        )
    })?;
    let password = password.to_owned();
    let hash = hash.to_owned();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        verify_password(&password, &hash)
    })
    .await
    .map_err(|_| {
        AuthError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Password verification failed.",
        )
    })
}

async fn dummy_password_hash() -> Result<String, AuthError> {
    if let Some(hash) = DUMMY_PASSWORD_HASH.get() {
        return Ok(hash.clone());
    }
    let hash = hash_password_limited("network-simulator-dummy-password").await?;
    let _ = DUMMY_PASSWORD_HASH.set(hash.clone());
    Ok(DUMMY_PASSWORD_HASH.get().cloned().unwrap_or(hash))
}

fn login_attempts() -> &'static Mutex<HashMap<String, LoginAttempt>> {
    LOGIN_ATTEMPTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn login_rate_key(username: &str, client_ip: &str) -> String {
    format!("{}\0{}", username.to_ascii_lowercase(), client_ip)
}

fn login_rate_limited(key: &str) -> bool {
    let now = Instant::now();
    let mut attempts = login_attempts()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    attempts.retain(|_, attempt| now.duration_since(attempt.last_seen) < Duration::from_secs(3600));
    attempts
        .get(key)
        .is_some_and(|attempt| attempt.next_allowed > now)
}

fn record_login_failure(key: String) {
    let now = Instant::now();
    let mut attempts = login_attempts()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let attempt = attempts.entry(key).or_insert(LoginAttempt {
        failures: 0,
        next_allowed: now,
        last_seen: now,
    });
    attempt.failures = attempt.failures.saturating_add(1);
    attempt.last_seen = now;
    let exponent = attempt.failures.saturating_sub(1).min(7);
    let delay = LOGIN_BACKOFF_BASE
        .saturating_mul(1_u32 << exponent)
        .min(LOGIN_BACKOFF_MAX);
    attempt.next_allowed = now + delay;
}

fn clear_login_failure(key: &str) {
    login_attempts()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(key);
}

fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash).ok().is_some_and(|parsed| {
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok()
    })
}

fn hex_decode(input: &str) -> Result<Vec<u8>, ()> {
    if input.len() % 2 != 0 {
        return Err(());
    }
    input
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16).ok_or(())?;
            let low = (pair[1] as char).to_digit(16).ok_or(())?;
            Ok(((high << 4) | low) as u8)
        })
        .collect()
}

fn validate_username(username: &str) -> Result<(), AuthError> {
    if username.len() < 3
        || !username
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(AuthError::bad_request(
            "Username must be at least 3 characters and contain only letters, numbers, '_' or '-'.",
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
struct LoginForm {
    username: String,
    password: String,
}
#[derive(Deserialize)]
struct LoginJson {
    username: String,
    password: String,
}
#[derive(Deserialize)]
struct RefreshBody {
    refresh_token: String,
}

async fn login(
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
async fn authenticate(
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
async fn token_for_credentials(
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

async fn refresh(
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
struct WsTicketRequest {
    path: String,
}

async fn create_ws_ticket(
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

async fn consume_ws_ticket(
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

async fn logout(
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

async fn me(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<Principal>,
) -> Result<Json<Value>, AuthError> {
    get_user_value(state.store.pool(), &principal.user_id)
        .await
        .map(Json)
}

async fn update_me(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<Principal>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AuthError> {
    update_user_row(state.store.pool(), &principal.user_id, &body, true).await?;
    get_user_value(state.store.pool(), &principal.user_id)
        .await
        .map(Json)
}

fn required_string(value: &Value, key: &str) -> Result<String, AuthError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| AuthError::bad_request(format!("'{key}' is required.")))
}
fn optional_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}
fn db_error(error: sqlx::Error) -> AuthError {
    if let sqlx::Error::Database(database) = &error {
        if database.is_unique_violation() {
            return AuthError::new(
                StatusCode::CONFLICT,
                "A record with that name or value already exists.",
            );
        }
    }
    tracing::error!(%error, "authentication database operation failed");
    AuthError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Authentication database operation failed.",
    )
}

#[cfg(test)]
mod tests;
