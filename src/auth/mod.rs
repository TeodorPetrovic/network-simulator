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
use passwords::*;
use pools::*;
use rbac::seed_rbac;
use roles::*;
use session::{consume_ws_ticket, create_ws_ticket, login, authenticate, logout, refresh};
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
mod passwords;
mod rbac;
mod route_class;
mod route_rules;
mod roles;
mod session;
mod users;
mod tokens;

const ACCESS_TTL: u64 = 60 * 60;
const REFRESH_TTL: u64 = 7 * 24 * 60 * 60;
const WS_TICKET_TTL: u64 = 60;
const JWT_ISSUER: &str = "network-simulator";
const JWT_AUDIENCE: &str = "network-simulator-api";

static WARNED_FORWARDED_AUTH: AtomicBool = AtomicBool::new(false);


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
