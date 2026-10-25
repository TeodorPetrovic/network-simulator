use super::*;

pub(super) async fn auth_gate(
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
                if path.ends_with("/ws") || path.ends_with("/capture/web-wireshark") {
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
            // The collection handler applies Project.Audit to each row. A
            // project-specific ACE must still let a student open the list.
            let project_collection =
                request.method() == axum::http::Method::GET && normalized == "/projects";
            if !project_collection
                && !has_privilege(state.store.pool(), &principal, normalized, required).await
            {
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

pub(super) fn request_has_forwarded_headers(headers: &axum::http::HeaderMap) -> bool {
    [
        "forwarded",
        "x-forwarded-for",
        "x-real-ip",
        "x-forwarded-host",
    ]
    .iter()
    .any(|name| headers.contains_key(*name))
}

pub(super) fn trust_proxy_headers_unauthenticated() -> bool {
    std::env::var("NETWORK_SIMULATOR_TRUST_PROXY_HEADERS_UNAUTH")
        .ok()
        .and_then(|value| parse_bool(&value))
        .unwrap_or(false)
}

pub(super) fn is_ticket_endpoint(path: &str) -> bool {
    path.ends_with("/ws")
        || path.ends_with("/capture/stream")
        || path.ends_with("/capture/web-wireshark")
}

pub(super) async fn has_privilege(
    pool: &SqlitePool,
    principal: &Principal,
    path: &str,
    privilege: &str,
) -> bool {
    if principal.is_superadmin {
        return true;
    }
    let path = acl::canonical_resource_path(path);
    match acl::privilege_decision(pool, &principal.user_id, &path, privilege).await {
        Ok(decision) => decision,
        Err(error) => {
            tracing::error!(%error, %path, privilege, "could not evaluate access control entries");
            false
        }
    }
}
