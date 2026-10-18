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
struct Principal {
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
        println!("Initial administrator password for {username}: {password}");
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

fn auth_override() -> Option<bool> {
    std::env::var("NETWORK_SIMULATOR_AUTH_ENABLED")
        .ok()
        .and_then(|value| parse_bool(&value))
}

fn effective_auth_mode(
    persisted_enabled: bool,
    bind_is_non_loopback: bool,
    explicit_disable: bool,
) -> bool {
    if explicit_disable {
        false
    } else {
        persisted_enabled || bind_is_non_loopback
    }
}

fn request_requires_auth(
    persisted_enabled: bool,
    bind_is_non_loopback: bool,
    explicit_disable: bool,
    forwarded_headers: bool,
    trust_proxy_headers: bool,
) -> bool {
    effective_auth_mode(persisted_enabled, bind_is_non_loopback, explicit_disable)
        || (!bind_is_non_loopback && forwarded_headers && !trust_proxy_headers)
}

pub(crate) fn parse_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

pub(crate) async fn set_auth_enabled(pool: &SqlitePool, enabled: bool) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE auth_config SET auth_enabled = ? WHERE singleton = 1")
        .bind(enabled)
        .execute(pool)
        .await?;
    Ok(())
}

async fn seed_rbac(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let privileges = [
        ("User.Allocate", "Create or delete a user"),
        ("User.Audit", "View a user"),
        ("User.Modify", "Update a user"),
        ("Group.Allocate", "Create or delete a group"),
        ("Group.Audit", "View a group"),
        ("Group.Modify", "Update a group"),
        ("Role.Allocate", "Create or delete a role"),
        ("Role.Audit", "View a role"),
        ("Role.Modify", "Update a role"),
        ("ACE.Allocate", "Create or delete an ACL entry"),
        ("ACE.Audit", "View an ACL entry"),
        ("ACE.Modify", "Update an ACL entry"),
        ("Pool.Allocate", "Create or delete a resource pool"),
        ("Pool.Audit", "View a resource pool"),
        ("Pool.Modify", "Update a resource pool"),
        ("Template.Allocate", "Create or delete a template"),
        ("Template.Audit", "View a template"),
        ("Template.Modify", "Update a template"),
        ("Project.Allocate", "Create or delete a project"),
        ("Project.Audit", "View a project"),
        ("Project.Modify", "Update a project"),
        ("Snapshot.Allocate", "Create or delete project snapshots"),
        ("Snapshot.Restore", "Restore a snapshot"),
        ("Snapshot.Audit", "View a snapshot"),
        ("Node.Allocate", "Create or delete a node"),
        ("Node.Audit", "View a node"),
        ("Node.Modify", "Update a node"),
        ("Node.Console", "Console access to a node"),
        ("Node.PowerMgmt", "Power management for a node"),
        ("Link.Allocate", "Create or delete a link"),
        ("Link.Audit", "View a link"),
        ("Link.Modify", "Update a link"),
        ("Link.Capture", "Capture packets on a link"),
        ("Drawing.Allocate", "Create or delete a drawing"),
        ("Drawing.Audit", "View a drawing"),
        ("Drawing.Modify", "Update a drawing"),
        ("Symbol.Allocate", "Create or delete a symbol"),
        ("Symbol.Audit", "View a symbol"),
        ("Image.Allocate", "Create or delete an image"),
        ("Image.Audit", "View an image"),
        ("Compute.Allocate", "Create or delete a compute"),
        ("Compute.Modify", "Update a compute"),
        ("Compute.Audit", "View a compute"),
        ("Appliance.Allocate", "Install an appliance"),
        ("Appliance.Audit", "View an appliance"),
        ("Server.Audit", "View server settings"),
        ("Server.Modify", "Update server settings"),
    ];
    for (name, description) in privileges {
        sqlx::query("INSERT OR IGNORE INTO auth_privileges (privilege_id, name, description) VALUES (?, ?, ?)")
            .bind(Uuid::new_v4().to_string()).bind(name).bind(description).execute(pool).await?;
    }
    let roles = [
        ("Administrator", "Administrator role"),
        ("User", "Regular user role"),
        ("Auditor", "Read-only access"),
        ("Template manager", "Template management"),
        ("User manager", "User and group management"),
        ("ACL manager", "Role and ACL management"),
        ("No Access", "No access"),
    ];
    let mut role_ids = HashMap::new();
    for (name, description) in roles {
        sqlx::query("INSERT OR IGNORE INTO auth_roles (role_id, name, description, is_builtin) VALUES (?, ?, ?, 1)")
            .bind(Uuid::new_v4().to_string()).bind(name).bind(description).execute(pool).await?;
        let id: String = sqlx::query_scalar("SELECT role_id FROM auth_roles WHERE name = ?")
            .bind(name)
            .fetch_one(pool)
            .await?;
        role_ids.insert(name, id);
    }
    let all: Vec<String> = sqlx::query_scalar("SELECT privilege_id FROM auth_privileges")
        .fetch_all(pool)
        .await?;
    for privilege_id in all {
        sqlx::query(
            "INSERT OR IGNORE INTO auth_role_privileges (role_id, privilege_id) VALUES (?, ?)",
        )
        .bind(&role_ids["Administrator"])
        .bind(privilege_id)
        .execute(pool)
        .await?;
    }
    let defaults: [(&str, &[&str]); 5] = [
        (
            "User",
            &[
                "Project.Allocate",
                "Project.Audit",
                "Project.Modify",
                "Snapshot.Allocate",
                "Snapshot.Audit",
                "Snapshot.Restore",
                "Node.Allocate",
                "Node.Audit",
                "Node.Modify",
                "Node.Console",
                "Node.PowerMgmt",
                "Link.Allocate",
                "Link.Audit",
                "Link.Modify",
                "Link.Capture",
                "Drawing.Allocate",
                "Drawing.Audit",
                "Drawing.Modify",
                "Template.Audit",
                "Symbol.Audit",
                "Image.Audit",
                "Compute.Audit",
                "Appliance.Allocate",
                "Appliance.Audit",
            ],
        ),
        (
            "Auditor",
            &[
                "Project.Audit",
                "Snapshot.Audit",
                "Node.Audit",
                "Link.Audit",
                "Drawing.Audit",
                "Template.Audit",
                "Symbol.Audit",
                "Image.Audit",
                "Compute.Audit",
                "Appliance.Audit",
            ],
        ),
        (
            "Template manager",
            &[
                "Template.Allocate",
                "Template.Audit",
                "Template.Modify",
                "Symbol.Allocate",
                "Symbol.Audit",
                "Image.Allocate",
                "Image.Audit",
                "Appliance.Allocate",
                "Appliance.Audit",
            ],
        ),
        (
            "User manager",
            &[
                "User.Allocate",
                "User.Audit",
                "User.Modify",
                "Group.Allocate",
                "Group.Audit",
                "Group.Modify",
            ],
        ),
        (
            "ACL manager",
            &[
                "Role.Allocate",
                "Role.Audit",
                "Role.Modify",
                "ACE.Allocate",
                "ACE.Audit",
                "ACE.Modify",
            ],
        ),
    ];
    for (role_name, names) in defaults {
        for name in names {
            sqlx::query("INSERT OR IGNORE INTO auth_role_privileges (role_id, privilege_id) SELECT ?, privilege_id FROM auth_privileges WHERE name = ?")
                .bind(&role_ids[role_name]).bind(name).execute(pool).await?;
        }
    }
    for group in ["Administrators", "Users"] {
        sqlx::query(
            "INSERT OR IGNORE INTO auth_groups (user_group_id, name, is_builtin) VALUES (?, ?, 1)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(group)
        .execute(pool)
        .await?;
    }
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

async fn auth_gate(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let path = request.uri().path().to_owned();
    if !path.starts_with("/v3/")
        || matches!(
            path.as_str(),
            "/v3/access/users/login"
                | "/v3/access/users/authenticate"
                | "/v3/access/users/refresh"
                | "/v3/access/settings/status"
        )
    {
        return next.run(request).await;
    }
    let normalized = path.strip_prefix("/v3").unwrap_or(&path);
    let Some(class) = classify_route(request.method().as_str(), normalized) else {
        return AuthError::forbidden("This route has no authorization classification.")
            .into_response();
    };
    if class == RouteClass::Deny {
        return AuthError::forbidden("This route is denied by default.").into_response();
    }
    let persisted_enabled = match sqlx::query_scalar::<_, bool>(
        "SELECT auth_enabled FROM auth_config WHERE singleton = 1",
    )
    .fetch_optional(state.store.pool())
    .await
    {
        Ok(Some(value)) => value,
        Ok(None) => {
            tracing::error!("authentication configuration row is missing");
            return AuthError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Authentication configuration is missing.",
            )
            .into_response();
        }
        Err(error) => {
            tracing::error!(%error, "could not read authentication setting");
            return AuthError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not read authentication setting.",
            )
            .into_response();
        }
    };
    let explicit_disable = auth_override() == Some(false);
    let forwarded_headers = request_has_forwarded_headers(request.headers());
    let trust_proxy_headers = trust_proxy_headers_unauthenticated();
    let enabled = request_requires_auth(
        persisted_enabled,
        !state.bind_address.ip().is_loopback(),
        explicit_disable,
        forwarded_headers,
        trust_proxy_headers,
    );
    if enabled
        && !persisted_enabled
        && forwarded_headers
        && !trust_proxy_headers
        && !WARNED_FORWARDED_AUTH.swap(true, Ordering::Relaxed)
    {
        tracing::warn!(
            "authentication was forced on for a forwarded request to a loopback-bound controller"
        );
    }
    if !enabled {
        let row = match sqlx::query("SELECT user_id, must_change_password FROM auth_users WHERE is_superadmin = 1 AND is_active = 1 ORDER BY created_at LIMIT 1")
            .fetch_optional(state.store.pool()).await
        {
            Ok(Some(row)) => row,
            Ok(None) => return AuthError::new(StatusCode::INTERNAL_SERVER_ERROR, "No active super administrator is available.").into_response(),
            Err(error) => {
                tracing::error!(%error, "could not load active bootstrap administrator");
                return AuthError::new(StatusCode::INTERNAL_SERVER_ERROR, "Could not load authentication principal.").into_response();
            }
        };
        request.extensions_mut().insert(Principal {
            user_id: row.get("user_id"),
            is_superadmin: true,
            must_change_password: row.get("must_change_password"),
        });
        return next.run(request).await;
    }
    let principal = if is_ticket_endpoint(&path)
        && let Some(ticket) = request
            .uri()
            .query()
            .and_then(|query| url_query_value(query, "ticket"))
    {
        match consume_ws_ticket(state.store.pool(), &ticket, &path).await {
            Ok(Some(principal)) => principal,
            Ok(None) => return AuthError::unauthorized().into_response(),
            Err(error) => {
                tracing::error!(%error, "could not consume one-use access ticket");
                return AuthError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Could not validate access ticket.",
                )
                .into_response();
            }
        }
    } else {
        let token = request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(str::to_owned)
            .or_else(|| {
                if path.ends_with("/ws") {
                    request
                        .uri()
                        .query()
                        .and_then(|query| url_query_value(query, "token"))
                } else {
                    None
                }
            });
        let Some(token) = token else {
            return AuthError::unauthorized().into_response();
        };
        let Some(principal) = authenticate_token(state.store.pool(), &token).await else {
            return AuthError::unauthorized().into_response();
        };
        principal
    };
    if principal.must_change_password
        && path != "/v3/access/users/me"
        && path != "/v3/access/users/logout"
    {
        return AuthError::forbidden("Change the initial password before using this account.")
            .into_response();
    }
    match class {
        RouteClass::Public => return next.run(request).await,
        RouteClass::Deny => {
            return AuthError::forbidden("This route is denied by default.").into_response();
        }
        RouteClass::PrincipalScoped => {}
        RouteClass::Privilege(required) => {
            if !has_privilege(state.store.pool(), &principal, normalized, required).await {
                return AuthError::forbidden(format!(
                    "Permission denied (privilege {required} is required)"
                ))
                .into_response();
            }
        }
    }
    request.extensions_mut().insert(principal);
    next.run(request).await
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RouteClass {
    Public,
    PrincipalScoped,
    Deny,
    Privilege(&'static str),
}

#[derive(Clone, Copy)]
struct RouteRule {
    method: &'static str,
    path: &'static str,
    class: RouteClass,
}

macro_rules! rules {
    ($(($method:literal, $path:literal, $class:expr)),* $(,)?) => {
        &[$(RouteRule { method: $method, path: $path, class: $class }),*]
    };
}

const ROUTE_RULES: &[RouteRule] = rules![
    ("POST", "/access/users/login", RouteClass::Public),
    ("POST", "/access/users/authenticate", RouteClass::Public),
    ("POST", "/access/users/refresh", RouteClass::Public),
    ("GET", "/access/settings/status", RouteClass::Public),
    ("POST", "/access/users/logout", RouteClass::PrincipalScoped),
    ("GET", "/access/users/me", RouteClass::PrincipalScoped),
    ("PUT", "/access/users/me", RouteClass::PrincipalScoped),
    ("POST", "/access/ws-ticket", RouteClass::PrincipalScoped),
    ("GET", "/access/api-keys", RouteClass::PrincipalScoped),
    ("POST", "/access/api-keys", RouteClass::PrincipalScoped),
    (
        "POST",
        "/access/api-keys/{key_id}/revoke",
        RouteClass::PrincipalScoped
    ),
    (
        "POST",
        "/access/api-keys/{key_id}/restore",
        RouteClass::PrincipalScoped
    ),
    (
        "DELETE",
        "/access/api-keys/{key_id}",
        RouteClass::PrincipalScoped
    ),
    ("GET", "/access/users", RouteClass::Privilege("User.Audit")),
    (
        "POST",
        "/access/users",
        RouteClass::Privilege("User.Allocate")
    ),
    (
        "GET",
        "/access/users/{user_id}",
        RouteClass::Privilege("User.Audit")
    ),
    (
        "PUT",
        "/access/users/{user_id}",
        RouteClass::Privilege("User.Modify")
    ),
    (
        "DELETE",
        "/access/users/{user_id}",
        RouteClass::Privilege("User.Allocate")
    ),
    (
        "GET",
        "/access/users/{user_id}/groups",
        RouteClass::Privilege("User.Audit")
    ),
    (
        "GET",
        "/access/groups",
        RouteClass::Privilege("Group.Audit")
    ),
    (
        "POST",
        "/access/groups",
        RouteClass::Privilege("Group.Allocate")
    ),
    (
        "GET",
        "/access/groups/{group_id}",
        RouteClass::Privilege("Group.Audit")
    ),
    (
        "PUT",
        "/access/groups/{group_id}",
        RouteClass::Privilege("Group.Modify")
    ),
    (
        "DELETE",
        "/access/groups/{group_id}",
        RouteClass::Privilege("Group.Allocate")
    ),
    (
        "GET",
        "/access/groups/{group_id}/members",
        RouteClass::Privilege("Group.Audit")
    ),
    (
        "PUT",
        "/access/groups/{group_id}/members/{user_id}",
        RouteClass::Privilege("Group.Modify")
    ),
    (
        "DELETE",
        "/access/groups/{group_id}/members/{user_id}",
        RouteClass::Privilege("Group.Modify")
    ),
    ("GET", "/access/roles", RouteClass::Privilege("Role.Audit")),
    (
        "POST",
        "/access/roles",
        RouteClass::Privilege("Role.Allocate")
    ),
    (
        "GET",
        "/access/roles/{role_id}",
        RouteClass::Privilege("Role.Audit")
    ),
    (
        "PUT",
        "/access/roles/{role_id}",
        RouteClass::Privilege("Role.Modify")
    ),
    (
        "DELETE",
        "/access/roles/{role_id}",
        RouteClass::Privilege("Role.Allocate")
    ),
    (
        "GET",
        "/access/roles/{role_id}/privileges",
        RouteClass::Privilege("Role.Audit")
    ),
    (
        "PUT",
        "/access/roles/{role_id}/privileges/{privilege_id}",
        RouteClass::Privilege("Role.Modify")
    ),
    (
        "DELETE",
        "/access/roles/{role_id}/privileges/{privilege_id}",
        RouteClass::Privilege("Role.Modify")
    ),
    (
        "GET",
        "/access/privileges",
        RouteClass::Privilege("Role.Audit")
    ),
    ("GET", "/access/acl", RouteClass::Privilege("ACE.Audit")),
    ("POST", "/access/acl", RouteClass::Privilege("ACE.Allocate")),
    (
        "GET",
        "/access/acl/endpoints",
        RouteClass::Privilege("ACE.Audit")
    ),
    (
        "GET",
        "/access/acl/{ace_id}",
        RouteClass::Privilege("ACE.Audit")
    ),
    (
        "PUT",
        "/access/acl/{ace_id}",
        RouteClass::Privilege("ACE.Modify")
    ),
    (
        "DELETE",
        "/access/acl/{ace_id}",
        RouteClass::Privilege("ACE.Allocate")
    ),
    (
        "GET",
        "/access/settings",
        RouteClass::Privilege("Server.Audit")
    ),
    (
        "PUT",
        "/access/settings",
        RouteClass::Privilege("Server.Modify")
    ),
    ("GET", "/pools", RouteClass::Privilege("Pool.Audit")),
    ("POST", "/pools", RouteClass::Privilege("Pool.Allocate")),
    (
        "GET",
        "/pools/{pool_id}",
        RouteClass::Privilege("Pool.Audit")
    ),
    (
        "PUT",
        "/pools/{pool_id}",
        RouteClass::Privilege("Pool.Modify")
    ),
    (
        "DELETE",
        "/pools/{pool_id}",
        RouteClass::Privilege("Pool.Allocate")
    ),
    (
        "GET",
        "/pools/{pool_id}/resources",
        RouteClass::Privilege("Pool.Audit")
    ),
    (
        "PUT",
        "/pools/{pool_id}/resources/{resource_id}",
        RouteClass::Privilege("Pool.Modify")
    ),
    (
        "DELETE",
        "/pools/{pool_id}/resources/{resource_id}",
        RouteClass::Privilege("Pool.Modify")
    ),
    ("GET", "/version", RouteClass::Privilege("Server.Audit")),
    ("POST", "/version", RouteClass::Privilege("Project.Modify")),
    ("GET", "/runtimes", RouteClass::Privilege("Server.Audit")),
    (
        "GET",
        "/resources/monitor",
        RouteClass::Privilege("Server.Audit")
    ),
    (
        "POST",
        "/docker/images/pull",
        RouteClass::Privilege("Compute.Modify")
    ),
    (
        "GET",
        "/marketplace/ishare2/images",
        RouteClass::Privilege("Server.Audit")
    ),
    (
        "POST",
        "/marketplace/ishare2/images/{image_type}/{image_id}/download",
        RouteClass::Privilege("Image.Allocate")
    ),
    (
        "GET",
        "/marketplace/gns3/appliances",
        RouteClass::Privilege("Appliance.Audit")
    ),
    (
        "GET",
        "/marketplace/gns3/appliances/{file_name}",
        RouteClass::Privilege("Appliance.Audit")
    ),
    (
        "POST",
        "/marketplace/gns3/appliances/{file_name}/import",
        RouteClass::Privilege("Appliance.Allocate")
    ),
    (
        "GET",
        "/marketplace/gns3/imported",
        RouteClass::Privilege("Appliance.Audit")
    ),
    (
        "GET",
        "/marketplace/gns3/manager",
        RouteClass::Privilege("Appliance.Audit")
    ),
    (
        "POST",
        "/marketplace/gns3/custom",
        RouteClass::Privilege("Template.Allocate")
    ),
    (
        "PUT",
        "/marketplace/gns3/appliances/{file_name}/bindings",
        RouteClass::Privilege("Appliance.Allocate")
    ),
    (
        "DELETE",
        "/marketplace/gns3/appliances/{file_name}/bindings",
        RouteClass::Privilege("Appliance.Allocate")
    ),
    (
        "POST",
        "/marketplace/gns3/import/{file_name}",
        RouteClass::Privilege("Appliance.Allocate")
    ),
    ("GET", "/statistics", RouteClass::Privilege("Server.Audit")),
    (
        "GET",
        "/notifications",
        RouteClass::Privilege("Server.Audit")
    ),
    (
        "GET",
        "/notifications/ws",
        RouteClass::Privilege("Server.Audit")
    ),
    ("GET", "/settings", RouteClass::Privilege("Server.Audit")),
    ("PUT", "/settings", RouteClass::Privilege("Server.Modify")),
    ("GET", "/computes", RouteClass::Privilege("Compute.Audit")),
    (
        "GET",
        "/computes/{compute_id}",
        RouteClass::Privilege("Compute.Audit")
    ),
    ("GET", "/templates", RouteClass::Privilege("Template.Audit")),
    (
        "POST",
        "/templates",
        RouteClass::Privilege("Template.Allocate")
    ),
    (
        "GET",
        "/templates/{template_id}",
        RouteClass::Privilege("Template.Audit")
    ),
    (
        "PUT",
        "/templates/{template_id}",
        RouteClass::Privilege("Template.Modify")
    ),
    (
        "DELETE",
        "/templates/{template_id}",
        RouteClass::Privilege("Template.Allocate")
    ),
    (
        "POST",
        "/templates/{template_id}/duplicate",
        RouteClass::Privilege("Template.Allocate")
    ),
    (
        "POST",
        "/projects/load",
        RouteClass::Privilege("Project.Allocate")
    ),
    ("GET", "/projects", RouteClass::Privilege("Project.Audit")),
    (
        "POST",
        "/projects",
        RouteClass::Privilege("Project.Allocate")
    ),
    (
        "GET",
        "/projects/{project_id}",
        RouteClass::Privilege("Project.Audit")
    ),
    (
        "PUT",
        "/projects/{project_id}",
        RouteClass::Privilege("Project.Modify")
    ),
    (
        "DELETE",
        "/projects/{project_id}",
        RouteClass::Privilege("Project.Allocate")
    ),
    (
        "POST",
        "/projects/{project_id}/open",
        RouteClass::Privilege("Project.Allocate")
    ),
    (
        "POST",
        "/projects/{project_id}/close",
        RouteClass::Privilege("Project.Allocate")
    ),
    (
        "GET",
        "/projects/{project_id}/stats",
        RouteClass::Privilege("Project.Audit")
    ),
    (
        "GET",
        "/projects/{project_id}/locked",
        RouteClass::Privilege("Project.Audit")
    ),
    (
        "GET",
        "/projects/{project_id}/gns3file",
        RouteClass::Privilege("Project.Audit")
    ),
    (
        "GET",
        "/projects/{project_id}/export",
        RouteClass::Privilege("Project.Audit")
    ),
    (
        "POST",
        "/projects/{project_id}/duplicate",
        RouteClass::Privilege("Project.Allocate")
    ),
    (
        "POST",
        "/projects/{project_id}/import",
        RouteClass::Privilege("Project.Allocate")
    ),
    (
        "POST",
        "/projects/{project_id}/lock",
        RouteClass::Privilege("Project.Modify")
    ),
    (
        "POST",
        "/projects/{project_id}/unlock",
        RouteClass::Privilege("Project.Modify")
    ),
    (
        "GET",
        "/projects/{project_id}/files/{*file_path}",
        RouteClass::Privilege("Project.Audit")
    ),
    (
        "POST",
        "/projects/{project_id}/files/{*file_path}",
        RouteClass::Privilege("Project.Modify")
    ),
    (
        "GET",
        "/projects/{project_id}/notifications",
        RouteClass::Privilege("Project.Audit")
    ),
    (
        "GET",
        "/projects/{project_id}/notifications/ws",
        RouteClass::Privilege("Project.Audit")
    ),
    (
        "GET",
        "/projects/{project_id}/snapshots",
        RouteClass::Privilege("Snapshot.Audit")
    ),
    (
        "POST",
        "/projects/{project_id}/snapshots",
        RouteClass::Privilege("Snapshot.Allocate")
    ),
    (
        "DELETE",
        "/projects/{project_id}/snapshots/{snapshot_id}",
        RouteClass::Privilege("Snapshot.Allocate")
    ),
    (
        "POST",
        "/projects/{project_id}/snapshots/{snapshot_id}/restore",
        RouteClass::Privilege("Snapshot.Restore")
    ),
    (
        "POST",
        "/projects/{project_id}/nodes/start",
        RouteClass::Privilege("Node.PowerMgmt")
    ),
    (
        "POST",
        "/projects/{project_id}/nodes/stop",
        RouteClass::Privilege("Node.PowerMgmt")
    ),
    (
        "POST",
        "/projects/{project_id}/nodes/suspend",
        RouteClass::Privilege("Node.PowerMgmt")
    ),
    (
        "POST",
        "/projects/{project_id}/nodes/reload",
        RouteClass::Privilege("Node.PowerMgmt")
    ),
    (
        "POST",
        "/projects/{project_id}/nodes/console/reset",
        RouteClass::Privilege("Node.Console")
    ),
    (
        "GET",
        "/projects/{project_id}/nodes",
        RouteClass::Privilege("Node.Audit")
    ),
    (
        "POST",
        "/projects/{project_id}/nodes",
        RouteClass::Privilege("Node.Allocate")
    ),
    (
        "POST",
        "/projects/{project_id}/templates/{template_id}",
        RouteClass::Privilege("Node.Allocate")
    ),
    (
        "GET",
        "/projects/{project_id}/nodes/{node_id}",
        RouteClass::Privilege("Node.Audit")
    ),
    (
        "PUT",
        "/projects/{project_id}/nodes/{node_id}",
        RouteClass::Privilege("Node.Modify")
    ),
    (
        "DELETE",
        "/projects/{project_id}/nodes/{node_id}",
        RouteClass::Privilege("Node.Allocate")
    ),
    (
        "PUT",
        "/projects/{project_id}/nodes/{node_id}/config",
        RouteClass::Privilege("Node.Modify")
    ),
    (
        "POST",
        "/projects/{project_id}/nodes/{node_id}/start",
        RouteClass::Privilege("Node.PowerMgmt")
    ),
    (
        "POST",
        "/projects/{project_id}/nodes/{node_id}/stop",
        RouteClass::Privilege("Node.PowerMgmt")
    ),
    (
        "POST",
        "/projects/{project_id}/nodes/{node_id}/suspend",
        RouteClass::Privilege("Node.PowerMgmt")
    ),
    (
        "POST",
        "/projects/{project_id}/nodes/{node_id}/reload",
        RouteClass::Privilege("Node.PowerMgmt")
    ),
    (
        "POST",
        "/projects/{project_id}/nodes/{node_id}/duplicate",
        RouteClass::Privilege("Node.Allocate")
    ),
    (
        "POST",
        "/projects/{project_id}/nodes/{node_id}/isolate",
        RouteClass::Privilege("Link.Modify")
    ),
    (
        "POST",
        "/projects/{project_id}/nodes/{node_id}/unisolate",
        RouteClass::Privilege("Link.Modify")
    ),
    (
        "POST",
        "/projects/{project_id}/nodes/{node_id}/console/reset",
        RouteClass::Privilege("Node.Console")
    ),
    (
        "GET",
        "/projects/{project_id}/nodes/{node_id}/links",
        RouteClass::Privilege("Link.Audit")
    ),
    (
        "GET",
        "/projects/{project_id}/nodes/{node_id}/files",
        RouteClass::Privilege("Node.Audit")
    ),
    (
        "GET",
        "/projects/{project_id}/nodes/{node_id}/files/{*file_path}",
        RouteClass::Privilege("Node.Audit")
    ),
    (
        "POST",
        "/projects/{project_id}/nodes/{node_id}/files/{*file_path}",
        RouteClass::Privilege("Node.Modify")
    ),
    (
        "DELETE",
        "/projects/{project_id}/nodes/{node_id}/files/{*file_path}",
        RouteClass::Privilege("Node.Modify")
    ),
    (
        "POST",
        "/projects/{project_id}/nodes/{node_id}/qemu/disk_image/{disk_name}",
        RouteClass::Privilege("Node.Allocate")
    ),
    (
        "PUT",
        "/projects/{project_id}/nodes/{node_id}/qemu/disk_image/{disk_name}",
        RouteClass::Privilege("Node.Modify")
    ),
    (
        "DELETE",
        "/projects/{project_id}/nodes/{node_id}/qemu/disk_image/{disk_name}",
        RouteClass::Privilege("Node.Modify")
    ),
    (
        "GET",
        "/projects/{project_id}/nodes/{node_id}/dynamips/auto_idlepc",
        RouteClass::Privilege("Node.Audit")
    ),
    (
        "GET",
        "/projects/{project_id}/nodes/{node_id}/dynamips/idlepc_proposals",
        RouteClass::Privilege("Node.Audit")
    ),
    (
        "GET",
        "/projects/{project_id}/links",
        RouteClass::Privilege("Link.Audit")
    ),
    (
        "POST",
        "/projects/{project_id}/links",
        RouteClass::Privilege("Link.Allocate")
    ),
    (
        "GET",
        "/projects/{project_id}/links/{link_id}",
        RouteClass::Privilege("Link.Audit")
    ),
    (
        "PUT",
        "/projects/{project_id}/links/{link_id}",
        RouteClass::Privilege("Link.Modify")
    ),
    (
        "DELETE",
        "/projects/{project_id}/links/{link_id}",
        RouteClass::Privilege("Link.Allocate")
    ),
    (
        "GET",
        "/projects/{project_id}/links/{link_id}/available_filters",
        RouteClass::Privilege("Link.Audit")
    ),
    (
        "POST",
        "/projects/{project_id}/links/{link_id}/capture/start",
        RouteClass::Privilege("Link.Capture")
    ),
    (
        "POST",
        "/projects/{project_id}/links/{link_id}/capture/stop",
        RouteClass::Privilege("Link.Capture")
    ),
    (
        "GET",
        "/projects/{project_id}/links/{link_id}/capture/stream",
        RouteClass::Privilege("Link.Capture")
    ),
    (
        "GET",
        "/projects/{project_id}/links/{link_id}/capture/file",
        RouteClass::Privilege("Link.Capture")
    ),
    (
        "GET",
        "/projects/{project_id}/drawings",
        RouteClass::Privilege("Drawing.Audit")
    ),
    (
        "POST",
        "/projects/{project_id}/drawings",
        RouteClass::Privilege("Drawing.Allocate")
    ),
    (
        "GET",
        "/projects/{project_id}/drawings/{drawing_id}",
        RouteClass::Privilege("Drawing.Audit")
    ),
    (
        "PUT",
        "/projects/{project_id}/drawings/{drawing_id}",
        RouteClass::Privilege("Drawing.Modify")
    ),
    (
        "DELETE",
        "/projects/{project_id}/drawings/{drawing_id}",
        RouteClass::Privilege("Drawing.Allocate")
    ),
    (
        "GET",
        "/projects/{project_id}/topology",
        RouteClass::Privilege("Project.Audit")
    ),
    (
        "GET",
        "/compute/projects/{project_id}/qemu/nodes/{node_id}/console/ws",
        RouteClass::Privilege("Node.Console")
    ),
    (
        "GET",
        "/compute/projects/{project_id}/docker/nodes/{node_id}/console/ws",
        RouteClass::Privilege("Node.Console")
    ),
    (
        "GET",
        "/compute/projects/{project_id}/docker/nodes/{node_id}/aux_console/ws",
        RouteClass::Privilege("Node.Console")
    ),
    (
        "GET",
        "/compute/projects/{project_id}/vpcs/nodes/{node_id}/console/ws",
        RouteClass::Privilege("Node.Console")
    ),
    (
        "GET",
        "/compute/projects/{project_id}/dynamips/nodes/{node_id}/console/ws",
        RouteClass::Privilege("Node.Console")
    ),
    ("GET", "/images", RouteClass::Privilege("Image.Audit")),
    (
        "POST",
        "/images/upload/{*image_path}",
        RouteClass::Privilege("Image.Allocate")
    ),
    (
        "GET",
        "/images/{*image_path}",
        RouteClass::Privilege("Image.Audit")
    ),
    (
        "DELETE",
        "/images/{*image_path}",
        RouteClass::Privilege("Image.Allocate")
    ),
    (
        "POST",
        "/dynamips/images/{file_name}",
        RouteClass::Privilege("Image.Allocate")
    ),
    (
        "POST",
        "/qemu/images/{file_name}",
        RouteClass::Privilege("Image.Allocate")
    ),
    ("GET", "/symbols", RouteClass::Privilege("Symbol.Audit")),
    (
        "GET",
        "/symbols/default_symbols",
        RouteClass::Privilege("Symbol.Audit")
    ),
    (
        "GET",
        "/symbols/{symbol_id}/raw",
        RouteClass::Privilege("Symbol.Audit")
    ),
    (
        "POST",
        "/symbols/{symbol_id}/raw",
        RouteClass::Privilege("Symbol.Allocate")
    ),
    (
        "GET",
        "/symbols/{symbol_id}/dimensions",
        RouteClass::Privilege("Symbol.Audit")
    ),
    (
        "DELETE",
        "/symbols/{symbol_id}",
        RouteClass::Privilege("Symbol.Allocate")
    ),
    ("ANY", "", RouteClass::Public),
    ("ANY", "/{*path}", RouteClass::Deny),
];

fn classify_route(method: &str, path: &str) -> Option<RouteClass> {
    ROUTE_RULES
        .iter()
        .find(|rule| {
            (rule.method == method || rule.method == "ANY") && route_path_matches(path, rule.path)
        })
        .map(|rule| rule.class)
}

fn route_path_matches(path: &str, pattern: &str) -> bool {
    let path_segments = path.trim_matches('/').split('/').collect::<Vec<_>>();
    let pattern_segments = pattern.trim_matches('/').split('/').collect::<Vec<_>>();
    let mut path_index = 0;
    for (pattern_index, pattern_segment) in pattern_segments.iter().enumerate() {
        if pattern_segment.starts_with("{*") && pattern_segment.ends_with('}') {
            return pattern_index + 1 == pattern_segments.len() && path_index < path_segments.len();
        }
        let Some(path_segment) = path_segments.get(path_index) else {
            return false;
        };
        if !(pattern_segment.starts_with('{') && pattern_segment.ends_with('}'))
            && pattern_segment != path_segment
        {
            return false;
        }
        path_index += 1;
    }
    path_index == path_segments.len()
}

fn request_has_forwarded_headers(headers: &axum::http::HeaderMap) -> bool {
    [
        "forwarded",
        "x-forwarded-for",
        "x-real-ip",
        "x-forwarded-host",
    ]
    .iter()
    .any(|name| headers.contains_key(*name))
}

fn trust_proxy_headers_unauthenticated() -> bool {
    std::env::var("NETWORK_SIMULATOR_TRUST_PROXY_HEADERS_UNAUTH")
        .ok()
        .and_then(|value| parse_bool(&value))
        .unwrap_or(false)
}

fn is_ticket_endpoint(path: &str) -> bool {
    path.ends_with("/ws") || path.ends_with("/capture/stream")
}

async fn has_privilege(
    pool: &SqlitePool,
    principal: &Principal,
    path: &str,
    privilege: &str,
) -> bool {
    if principal.is_superadmin {
        return true;
    }
    let mut aces = direct_user_aces(pool, &principal.user_id, privilege).await;
    aces.extend(group_aces(pool, &principal.user_id, privilege).await);
    if let Some(result) = evaluate_acl(path, &aces) {
        return result;
    }
    pool_access_decision(pool, path, &aces)
        .await
        .unwrap_or(false)
}

type AclTuple = (String, bool, bool);

async fn direct_user_aces(pool: &SqlitePool, user_id: &str, privilege: &str) -> Vec<AclTuple> {
    let rows = sqlx::query("SELECT a.path, a.propagate, a.allowed FROM auth_acl a JOIN auth_role_privileges rp ON rp.role_id = a.role_id JOIN auth_privileges p ON p.privilege_id = rp.privilege_id WHERE a.ace_type = 'user' AND a.user_id = ? AND p.name = ? ORDER BY length(a.path) DESC")
        .bind(user_id).bind(privilege).fetch_all(pool).await.unwrap_or_default();
    rows.into_iter().map(row_acl_tuple).collect()
}

async fn group_aces(pool: &SqlitePool, user_id: &str, privilege: &str) -> Vec<AclTuple> {
    let rows = sqlx::query("SELECT DISTINCT a.path, a.propagate, a.allowed FROM auth_acl a JOIN auth_user_groups m ON m.user_group_id = a.group_id JOIN auth_role_privileges rp ON rp.role_id = a.role_id JOIN auth_privileges p ON p.privilege_id = rp.privilege_id WHERE a.ace_type = 'group' AND m.user_id = ? AND p.name = ? ORDER BY length(a.path) DESC")
        .bind(user_id).bind(privilege).fetch_all(pool).await.unwrap_or_default();
    rows.into_iter().map(row_acl_tuple).collect()
}

fn row_acl_tuple(row: sqlx::sqlite::SqliteRow) -> AclTuple {
    (row.get("path"), row.get("propagate"), row.get("allowed"))
}

async fn pool_access_decision(pool: &SqlitePool, path: &str, aces: &[AclTuple]) -> Option<bool> {
    let Some(project_id) = project_id_from_path(path) else {
        return None;
    };
    let pool_ids: Vec<String> = sqlx::query_scalar("SELECT m.resource_pool_id FROM resource_pool_map m JOIN resources r ON r.resource_id = m.resource_id WHERE r.resource_type = 'project' AND r.resource_id = ?")
        .bind(project_id).fetch_all(pool).await.unwrap_or_default();
    let mut denied = false;
    for pool_id in pool_ids {
        let root = format!("/pools/{pool_id}");
        let resource = format!("{root}/resources/{project_id}");
        let relevant = aces
            .iter()
            .filter(|(ace_path, _, _)| {
                let ace_path = normalize_acl_path(ace_path);
                ace_path == root || ace_path == resource
            })
            .cloned()
            .collect::<Vec<_>>();
        if let Some(allowed) = evaluate_acl(&resource, &relevant) {
            if allowed {
                return Some(true);
            }
            denied = true;
        }
    }
    denied.then_some(false)
}

fn evaluate_acl(path: &str, aces: &[(String, bool, bool)]) -> Option<bool> {
    let path = normalize_acl_path(path);
    let mut applicable = aces
        .iter()
        .filter(|(ace_path, _, _)| {
            let ace_path = normalize_acl_path(ace_path);
            path == ace_path
                || path
                    .strip_prefix(&ace_path)
                    .is_some_and(|suffix| ace_path == "/" || suffix.starts_with('/'))
        })
        .collect::<Vec<_>>();
    applicable.sort_by_key(|(ace_path, _, allowed)| {
        (
            std::cmp::Reverse(normalize_acl_path(ace_path).len()),
            *allowed,
            normalize_acl_path(ace_path),
        )
    });
    for (ace_path, propagate, allowed) in applicable {
        let ace_path = normalize_acl_path(ace_path);
        if path == ace_path {
            return Some(*allowed);
        }
        if !*allowed {
            return Some(false);
        }
        if *propagate {
            return Some(true);
        }
    }
    None
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

fn project_id_from_path(path: &str) -> Option<&str> {
    let mut segments = path.split('/').filter(|segment| !segment.is_empty());
    if segments.next()? == "projects" {
        segments.next()
    } else {
        None
    }
}

fn url_query_value(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| value.replace('+', " "))
    })
}

#[derive(Serialize, Deserialize)]
struct Claims {
    sub: String,
    exp: u64,
    iat: u64,
    nbf: u64,
    ver: i64,
    jti: String,
    iss: String,
    aud: String,
    #[serde(default = "access_type")]
    token_use: String,
}
fn access_type() -> String {
    "access".into()
}

fn encode_token(claims: &Claims, key: &str) -> Result<String, AuthError> {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"HS256","typ":"JWT"}"#);
    let payload =
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).map_err(|_| AuthError::unauthorized())?);
    let unsigned = format!("{header}.{payload}");
    let signing_key = hmac::Key::new(hmac::HMAC_SHA256, key.as_bytes());
    let signature = hmac::sign(&signing_key, unsigned.as_bytes());
    Ok(format!(
        "{unsigned}.{}",
        URL_SAFE_NO_PAD.encode(signature.as_ref())
    ))
}

fn decode_token(token: &str, key: &str) -> Option<Claims> {
    let mut pieces = token.split('.');
    let header = URL_SAFE_NO_PAD.decode(pieces.next()?).ok()?;
    let header: Value = serde_json::from_slice(&header).ok()?;
    if header.get("alg")?.as_str()? != "HS256" {
        return None;
    }
    let payload = pieces.next()?;
    let signature = URL_SAFE_NO_PAD.decode(pieces.next()?).ok()?;
    if pieces.next().is_some() {
        return None;
    }
    let unsigned = token.rsplit_once('.').map(|(unsigned, _)| unsigned)?;
    let signing_key = hmac::Key::new(hmac::HMAC_SHA256, key.as_bytes());
    hmac::verify(&signing_key, unsigned.as_bytes(), &signature).ok()?;
    let claims: Claims = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()?;
    let now = now_epoch();
    let leeway = 30;
    (claims.exp > now
        && claims.exp > claims.iat
        && claims.iat <= now.saturating_add(leeway)
        && claims.nbf <= now.saturating_add(leeway)
        && claims.iss == JWT_ISSUER
        && claims.aud == JWT_AUDIENCE
        && !claims.jti.is_empty())
    .then_some(claims)
}

fn new_claims(username: &str, version: i64, token_use: &str, ttl: u64) -> Claims {
    let now = now_epoch();
    Claims {
        sub: username.to_owned(),
        exp: now + ttl,
        iat: now,
        nbf: now,
        ver: version,
        jti: Uuid::new_v4().to_string(),
        iss: JWT_ISSUER.to_owned(),
        aud: JWT_AUDIENCE.to_owned(),
        token_use: token_use.to_owned(),
    }
}

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

async fn signing_key(pool: &SqlitePool) -> Option<String> {
    sqlx::query_scalar("SELECT jwt_signing_key FROM auth_config WHERE singleton = 1")
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

async fn authenticate_token(pool: &SqlitePool, token: &str) -> Option<Principal> {
    if let Some((key_id, secret)) = parse_api_key(token) {
        let row = sqlx::query("SELECT k.secret_hash, k.user_id, k.revoked, u.is_active, u.is_superadmin, u.must_change_password FROM auth_api_keys k JOIN auth_users u ON u.user_id = k.user_id WHERE k.api_key_id = ?")
            .bind(key_id).fetch_optional(pool).await.ok().flatten()?;
        if row.get::<bool, _>("revoked")
            || !row.get::<bool, _>("is_active")
            || !verify_password_limited(secret, row.get("secret_hash"))
                .await
                .ok()?
        {
            return None;
        }
        let user_id: String = row.get("user_id");
        let _ = sqlx::query(
            "UPDATE auth_api_keys SET last_used_at = CURRENT_TIMESTAMP WHERE api_key_id = ?",
        )
        .bind(key_id)
        .execute(pool)
        .await;
        return Some(Principal {
            user_id,
            is_superadmin: row.get("is_superadmin"),
            must_change_password: row.get("must_change_password"),
        });
    }
    let key = signing_key(pool).await?;
    let claims = decode_token(token, &key)?;
    if claims.token_use != "access" {
        return None;
    }
    let row = sqlx::query("SELECT user_id, is_active, is_superadmin, token_version, must_change_password FROM auth_users WHERE username = ?")
        .bind(claims.sub).fetch_optional(pool).await.ok().flatten()?;
    if !row.get::<bool, _>("is_active") || row.get::<i64, _>("token_version") != claims.ver {
        return None;
    }
    Some(Principal {
        user_id: row.get("user_id"),
        is_superadmin: row.get("is_superadmin"),
        must_change_password: row.get("must_change_password"),
    })
}

fn parse_api_key(token: &str) -> Option<(&str, &str)> {
    let mut parts = token.splitn(3, '_');
    if parts.next()? != "gns3" {
        return None;
    }
    Some((parts.next()?, parts.next()?))
}

fn random_hex(length: usize) -> Result<String, std::io::Error> {
    let mut bytes = vec![0_u8; length];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| std::io::Error::other("system random source failed"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
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

async fn list_users(State(state): State<AppState>) -> Result<Json<Value>, AuthError> {
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

async fn get_user(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> Result<Json<Value>, AuthError> {
    get_user_value(state.store.pool(), &user_id).await.map(Json)
}

async fn create_user(
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

async fn update_user(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AuthError> {
    update_user_row(state.store.pool(), &user_id, &body, false).await?;
    get_user_value(state.store.pool(), &user_id).await.map(Json)
}

async fn update_user_row(
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

async fn delete_user(
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

async fn get_user_value(pool: &SqlitePool, user_id: &str) -> Result<Value, AuthError> {
    let row = sqlx::query("SELECT user_id, username, email, full_name, is_active, is_superadmin, must_change_password, last_login, created_at, updated_at FROM auth_users WHERE user_id = ?")
        .bind(user_id).fetch_optional(pool).await.map_err(db_error)?.ok_or_else(|| AuthError::not_found(format!("User '{user_id}' not found")))?;
    Ok(
        json!({"user_id": row.get::<String,_>("user_id"), "username": row.get::<String,_>("username"), "email": row.get::<Option<String>,_>("email"), "full_name": row.get::<Option<String>,_>("full_name"), "is_active": row.get::<bool,_>("is_active"), "is_superadmin": row.get::<bool,_>("is_superadmin"), "must_change_password": row.get::<bool,_>("must_change_password"), "last_login": row.get::<Option<String>,_>("last_login"), "created_at": row.get::<String,_>("created_at"), "updated_at": row.get::<String,_>("updated_at")}),
    )
}

async fn user_memberships(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> Result<Json<Value>, AuthError> {
    let rows = sqlx::query("SELECT g.user_group_id, g.name, g.is_builtin, g.created_at, g.updated_at FROM auth_groups g JOIN auth_user_groups m ON m.user_group_id = g.user_group_id WHERE m.user_id = ? ORDER BY g.name")
        .bind(user_id).fetch_all(state.store.pool()).await.map_err(db_error)?;
    Ok(Json(Value::Array(rows.iter().map(group_json).collect())))
}

async fn list_groups(State(state): State<AppState>) -> Result<Json<Value>, AuthError> {
    let rows = sqlx::query("SELECT * FROM auth_groups ORDER BY name")
        .fetch_all(state.store.pool())
        .await
        .map_err(db_error)?;
    Ok(Json(Value::Array(rows.iter().map(group_json).collect())))
}
async fn get_group(
    State(state): State<AppState>,
    Path(group_id): Path<String>,
) -> Result<Json<Value>, AuthError> {
    get_group_value(state.store.pool(), &group_id)
        .await
        .map(Json)
}
async fn create_group(
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
async fn update_group(
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
async fn delete_group(
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
async fn group_members(
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
async fn add_group_member(
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
async fn remove_group_member(
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
async fn get_group_value(pool: &SqlitePool, group_id: &str) -> Result<Value, AuthError> {
    let row = sqlx::query("SELECT * FROM auth_groups WHERE user_group_id = ?")
        .bind(group_id)
        .fetch_optional(pool)
        .await
        .map_err(db_error)?
        .ok_or_else(|| AuthError::not_found("Group not found"))?;
    Ok(group_json(&row))
}
fn group_json(row: &sqlx::sqlite::SqliteRow) -> Value {
    json!({"user_group_id": row.get::<String,_>("user_group_id"), "name": row.get::<String,_>("name"), "is_builtin": row.get::<bool,_>("is_builtin"), "created_at": row.get::<String,_>("created_at"), "updated_at": row.get::<String,_>("updated_at")})
}

async fn list_roles(State(state): State<AppState>) -> Result<Json<Value>, AuthError> {
    let rows = sqlx::query("SELECT role_id FROM auth_roles ORDER BY name")
        .fetch_all(state.store.pool())
        .await
        .map_err(db_error)?;
    let mut roles = Vec::new();
    for row in rows {
        roles.push(role_value(state.store.pool(), &row.get::<String, _>("role_id")).await?);
    }
    Ok(Json(Value::Array(roles)))
}
async fn get_role(
    State(state): State<AppState>,
    Path(role_id): Path<String>,
) -> Result<Json<Value>, AuthError> {
    role_value(state.store.pool(), &role_id).await.map(Json)
}
async fn create_role(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), AuthError> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO auth_roles (role_id, name, description, is_builtin) VALUES (?, ?, ?, 0)",
    )
    .bind(&id)
    .bind(required_string(&body, "name")?)
    .bind(optional_string(&body, "description"))
    .execute(state.store.pool())
    .await
    .map_err(db_error)?;
    Ok((
        StatusCode::CREATED,
        Json(role_value(state.store.pool(), &id).await?),
    ))
}
async fn update_role(
    State(state): State<AppState>,
    Path(role_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AuthError> {
    if let Some(name) = optional_string(&body, "name") {
        sqlx::query("UPDATE auth_roles SET name = ?, updated_at = CURRENT_TIMESTAMP WHERE role_id = ? AND is_builtin = 0").bind(name).bind(&role_id).execute(state.store.pool()).await.map_err(db_error)?;
    }
    if let Some(description) = optional_string(&body, "description") {
        sqlx::query("UPDATE auth_roles SET description = ?, updated_at = CURRENT_TIMESTAMP WHERE role_id = ? AND is_builtin = 0").bind(description).bind(&role_id).execute(state.store.pool()).await.map_err(db_error)?;
    }
    role_value(state.store.pool(), &role_id).await.map(Json)
}
async fn delete_role(
    State(state): State<AppState>,
    Path(role_id): Path<String>,
) -> Result<StatusCode, AuthError> {
    let result = sqlx::query("DELETE FROM auth_roles WHERE role_id = ? AND is_builtin = 0")
        .bind(role_id)
        .execute(state.store.pool())
        .await
        .map_err(db_error)?;
    if result.rows_affected() == 0 {
        return Err(AuthError::not_found("Role not found or is built in."));
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn role_privileges(
    State(state): State<AppState>,
    Path(role_id): Path<String>,
) -> Result<Json<Value>, AuthError> {
    let rows = sqlx::query("SELECT p.* FROM auth_privileges p JOIN auth_role_privileges rp ON rp.privilege_id = p.privilege_id WHERE rp.role_id = ? ORDER BY p.name").bind(role_id).fetch_all(state.store.pool()).await.map_err(db_error)?;
    Ok(Json(Value::Array(
        rows.iter().map(privilege_json).collect(),
    )))
}
async fn add_role_privilege(
    State(state): State<AppState>,
    Path((role_id, privilege_id)): Path<(String, String)>,
) -> Result<StatusCode, AuthError> {
    sqlx::query("INSERT OR IGNORE INTO auth_role_privileges (role_id, privilege_id) VALUES (?, ?)")
        .bind(role_id)
        .bind(privilege_id)
        .execute(state.store.pool())
        .await
        .map_err(db_error)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn remove_role_privilege(
    State(state): State<AppState>,
    Path((role_id, privilege_id)): Path<(String, String)>,
) -> Result<StatusCode, AuthError> {
    sqlx::query("DELETE FROM auth_role_privileges WHERE role_id = ? AND privilege_id = ?")
        .bind(role_id)
        .bind(privilege_id)
        .execute(state.store.pool())
        .await
        .map_err(db_error)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn role_value(pool: &SqlitePool, role_id: &str) -> Result<Value, AuthError> {
    let row = sqlx::query("SELECT * FROM auth_roles WHERE role_id = ?")
        .bind(role_id)
        .fetch_optional(pool)
        .await
        .map_err(db_error)?
        .ok_or_else(|| AuthError::not_found("Role not found"))?;
    let privileges = sqlx::query("SELECT p.* FROM auth_privileges p JOIN auth_role_privileges rp ON rp.privilege_id = p.privilege_id WHERE rp.role_id = ? ORDER BY p.name").bind(role_id).fetch_all(pool).await.map_err(db_error)?;
    Ok(
        json!({"role_id": row.get::<String,_>("role_id"), "name": row.get::<String,_>("name"), "description": row.get::<Option<String>,_>("description"), "is_builtin": row.get::<bool,_>("is_builtin"), "privileges": privileges.iter().map(privilege_json).collect::<Vec<_>>(), "created_at": row.get::<String,_>("created_at"), "updated_at": row.get::<String,_>("updated_at")}),
    )
}
async fn list_privileges(State(state): State<AppState>) -> Result<Json<Value>, AuthError> {
    let rows = sqlx::query("SELECT * FROM auth_privileges ORDER BY name")
        .fetch_all(state.store.pool())
        .await
        .map_err(db_error)?;
    Ok(Json(Value::Array(
        rows.iter().map(privilege_json).collect(),
    )))
}
fn privilege_json(row: &sqlx::sqlite::SqliteRow) -> Value {
    json!({"privilege_id": row.get::<String,_>("privilege_id"), "name": row.get::<String,_>("name"), "description": row.get::<Option<String>,_>("description"), "created_at": row.get::<String,_>("created_at"), "updated_at": row.get::<String,_>("updated_at")})
}

async fn list_acl(State(state): State<AppState>) -> Result<Json<Value>, AuthError> {
    let rows = sqlx::query("SELECT * FROM auth_acl ORDER BY path")
        .fetch_all(state.store.pool())
        .await
        .map_err(db_error)?;
    Ok(Json(Value::Array(rows.iter().map(acl_json).collect())))
}
async fn get_acl(
    State(state): State<AppState>,
    Path(ace_id): Path<String>,
) -> Result<Json<Value>, AuthError> {
    acl_value(state.store.pool(), &ace_id).await.map(Json)
}
async fn create_acl(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), AuthError> {
    let ace_type = required_string(&body, "ace_type")?;
    if ace_type != "user" && ace_type != "group" {
        return Err(AuthError::bad_request(
            "ace_type must be 'user' or 'group'.",
        ));
    }
    let role_id = required_string(&body, "role_id")?;
    let path = normalize_acl_path(&required_string(&body, "path")?);
    let user_id = optional_string(&body, "user_id");
    let group_id = optional_string(&body, "group_id");
    if (ace_type == "user" && (user_id.is_none() || group_id.is_some()))
        || (ace_type == "group" && (group_id.is_none() || user_id.is_some()))
    {
        return Err(AuthError::bad_request(
            "Set exactly one of user_id or group_id according to ace_type.",
        ));
    }
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO auth_acl (ace_id, ace_type, path, propagate, allowed, user_id, group_id, role_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(&id).bind(ace_type).bind(path).bind(body.get("propagate").and_then(Value::as_bool).unwrap_or(true))
        .bind(body.get("allowed").and_then(Value::as_bool).unwrap_or(true)).bind(user_id).bind(group_id).bind(role_id)
        .execute(state.store.pool()).await.map_err(db_error)?;
    Ok((
        StatusCode::CREATED,
        Json(acl_value(state.store.pool(), &id).await?),
    ))
}
async fn update_acl(
    State(state): State<AppState>,
    Path(ace_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AuthError> {
    let current = sqlx::query("SELECT * FROM auth_acl WHERE ace_id = ?")
        .bind(&ace_id)
        .fetch_optional(state.store.pool())
        .await
        .map_err(db_error)?
        .ok_or_else(|| AuthError::not_found("ACL entry not found"))?;
    let ace_type = optional_string(&body, "ace_type").unwrap_or_else(|| current.get("ace_type"));
    if ace_type != "user" && ace_type != "group" {
        return Err(AuthError::bad_request(
            "ace_type must be 'user' or 'group'.",
        ));
    }
    let mut user_id: Option<String> = current.get("user_id");
    let mut group_id: Option<String> = current.get("group_id");
    if body.get("user_id").is_some() {
        user_id = optional_string(&body, "user_id");
    }
    if body.get("group_id").is_some() {
        group_id = optional_string(&body, "group_id");
    }
    if (ace_type == "user" && (user_id.is_none() || group_id.is_some()))
        || (ace_type == "group" && (group_id.is_none() || user_id.is_some()))
    {
        return Err(AuthError::bad_request(
            "Set exactly one of user_id or group_id according to ace_type.",
        ));
    }
    let path = optional_string(&body, "path")
        .map(|path| normalize_acl_path(&path))
        .unwrap_or_else(|| current.get("path"));
    let role_id = optional_string(&body, "role_id").unwrap_or_else(|| current.get("role_id"));
    let propagate = body
        .get("propagate")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| current.get("propagate"));
    let allowed = body
        .get("allowed")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| current.get("allowed"));
    sqlx::query("UPDATE auth_acl SET ace_type = ?, path = ?, propagate = ?, allowed = ?, user_id = ?, group_id = ?, role_id = ?, updated_at = CURRENT_TIMESTAMP WHERE ace_id = ?")
        .bind(ace_type).bind(path).bind(propagate).bind(allowed).bind(user_id).bind(group_id).bind(role_id).bind(&ace_id)
        .execute(state.store.pool()).await.map_err(db_error)?;
    acl_value(state.store.pool(), &ace_id).await.map(Json)
}
async fn delete_acl(
    State(state): State<AppState>,
    Path(ace_id): Path<String>,
) -> Result<StatusCode, AuthError> {
    let result = sqlx::query("DELETE FROM auth_acl WHERE ace_id = ?")
        .bind(ace_id)
        .execute(state.store.pool())
        .await
        .map_err(db_error)?;
    if result.rows_affected() == 0 {
        return Err(AuthError::not_found("ACL entry not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn acl_value(pool: &SqlitePool, ace_id: &str) -> Result<Value, AuthError> {
    let row = sqlx::query("SELECT * FROM auth_acl WHERE ace_id = ?")
        .bind(ace_id)
        .fetch_optional(pool)
        .await
        .map_err(db_error)?
        .ok_or_else(|| AuthError::not_found("ACL entry not found"))?;
    Ok(acl_json(&row))
}
fn acl_json(row: &sqlx::sqlite::SqliteRow) -> Value {
    json!({"ace_id": row.get::<String,_>("ace_id"), "ace_type": row.get::<String,_>("ace_type"), "path": row.get::<String,_>("path"), "propagate": row.get::<bool,_>("propagate"), "allowed": row.get::<bool,_>("allowed"), "user_id": row.get::<Option<String>,_>("user_id"), "group_id": row.get::<Option<String>,_>("group_id"), "role_id": row.get::<String,_>("role_id"), "created_at": row.get::<String,_>("created_at"), "updated_at": row.get::<String,_>("updated_at")})
}
async fn acl_endpoints(State(state): State<AppState>) -> Result<Json<Value>, AuthError> {
    let mut values = vec![
        json!({"endpoint":"/", "name":"All endpoints", "endpoint_type":"root"}),
        json!({"endpoint":"/projects", "name":"All projects", "endpoint_type":"project"}),
        json!({"endpoint":"/access/users", "name":"All users", "endpoint_type":"user"}),
        json!({"endpoint":"/access/groups", "name":"All groups", "endpoint_type":"group"}),
        json!({"endpoint":"/access/roles", "name":"All roles", "endpoint_type":"role"}),
    ];
    for (table, id_column, name_column, base, kind) in [
        ("projects", "project_id", "name", "/projects", "project"),
        ("auth_users", "user_id", "username", "/access/users", "user"),
        (
            "auth_groups",
            "user_group_id",
            "name",
            "/access/groups",
            "group",
        ),
        ("auth_roles", "role_id", "name", "/access/roles", "role"),
    ] {
        let query = format!("SELECT {id_column}, {name_column} FROM {table}");
        let rows = sqlx::query(&query)
            .fetch_all(state.store.pool())
            .await
            .map_err(db_error)?;
        for row in rows {
            let id: String = row.get(id_column);
            let name: String = row.get(name_column);
            values.push(json!({"endpoint":format!("{base}/{id}"), "name":format!("{kind} {name}"), "endpoint_type":kind}));
        }
    }
    Ok(Json(Value::Array(values)))
}

async fn list_pools(State(state): State<AppState>) -> Result<Json<Value>, AuthError> {
    let rows = sqlx::query("SELECT * FROM resource_pools ORDER BY name")
        .fetch_all(state.store.pool())
        .await
        .map_err(db_error)?;
    Ok(Json(Value::Array(rows.iter().map(pool_json).collect())))
}
async fn get_pool(
    State(state): State<AppState>,
    Path(pool_id): Path<String>,
) -> Result<Json<Value>, AuthError> {
    pool_value(state.store.pool(), &pool_id).await.map(Json)
}
async fn create_pool(
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
async fn update_pool(
    State(state): State<AppState>,
    Path(pool_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, AuthError> {
    if let Some(name) = optional_string(&body, "name") {
        sqlx::query("UPDATE resource_pools SET name = ?, updated_at = CURRENT_TIMESTAMP WHERE resource_pool_id = ?").bind(name).bind(&pool_id).execute(state.store.pool()).await.map_err(db_error)?;
    }
    pool_value(state.store.pool(), &pool_id).await.map(Json)
}
async fn delete_pool(
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
async fn pool_resources(
    State(state): State<AppState>,
    Path(pool_id): Path<String>,
) -> Result<Json<Value>, AuthError> {
    let rows = sqlx::query("SELECT r.* FROM resources r JOIN resource_pool_map m ON m.resource_id = r.resource_id WHERE m.resource_pool_id = ? ORDER BY r.name").bind(pool_id).fetch_all(state.store.pool()).await.map_err(db_error)?;
    Ok(Json(Value::Array(rows.iter().map(|row| json!({"resource_id":row.get::<String,_>("resource_id"), "resource_type":row.get::<String,_>("resource_type"), "name":row.get::<Option<String>,_>("name")})).collect())))
}
async fn add_pool_resource(
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
async fn remove_pool_resource(
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
async fn pool_value(pool: &SqlitePool, pool_id: &str) -> Result<Value, AuthError> {
    let row = sqlx::query("SELECT * FROM resource_pools WHERE resource_pool_id = ?")
        .bind(pool_id)
        .fetch_optional(pool)
        .await
        .map_err(db_error)?
        .ok_or_else(|| AuthError::not_found("Resource pool not found"))?;
    Ok(pool_json(&row))
}
fn pool_json(row: &sqlx::sqlite::SqliteRow) -> Value {
    json!({"resource_pool_id":row.get::<String,_>("resource_pool_id"),"name":row.get::<String,_>("name"),"created_at":row.get::<String,_>("created_at"),"updated_at":row.get::<String,_>("updated_at")})
}

#[derive(Deserialize)]
struct ApiKeyCreate {
    name: String,
}
async fn create_api_key(
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
async fn list_api_keys(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<Principal>,
) -> Result<Json<Value>, AuthError> {
    let rows = sqlx::query("SELECT api_key_id,name,key_prefix,revoked,created_at,last_used_at FROM auth_api_keys WHERE user_id = ? ORDER BY created_at DESC").bind(principal.user_id).fetch_all(state.store.pool()).await.map_err(db_error)?;
    Ok(Json(Value::Array(rows.iter().map(|row| json!({"api_key_id":row.get::<String,_>("api_key_id"),"name":row.get::<String,_>("name"),"key_prefix":row.get::<String,_>("key_prefix"),"revoked":row.get::<bool,_>("revoked"),"created_at":row.get::<String,_>("created_at"),"last_used_at":row.get::<Option<String>,_>("last_used_at")})).collect())))
}
async fn set_api_key_revoked(
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
async fn revoke_api_key(
    State(state): State<AppState>,
    Path(key_id): Path<String>,
    axum::Extension(principal): axum::Extension<Principal>,
) -> Result<Json<Value>, AuthError> {
    set_api_key_revoked(&state, &principal, &key_id, true).await?;
    Ok(Json(json!({"message":"API key revoked"})))
}
async fn restore_api_key(
    State(state): State<AppState>,
    Path(key_id): Path<String>,
    axum::Extension(principal): axum::Extension<Principal>,
) -> Result<Json<Value>, AuthError> {
    set_api_key_revoked(&state, &principal, &key_id, false).await?;
    Ok(Json(json!({"message":"API key restored"})))
}
async fn delete_api_key(
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
struct AuthSettingsUpdate {
    auth_enabled: bool,
}
async fn auth_status(
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
async fn get_auth_settings(
    State(state): State<AppState>,
    request: Request<Body>,
) -> Result<Json<Value>, AuthError> {
    auth_status(State(state), request).await
}
async fn update_auth_settings(
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
