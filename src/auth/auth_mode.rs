pub(super) fn auth_override() -> Option<bool> {
    std::env::var("NETWORK_SIMULATOR_AUTH_ENABLED")
        .ok()
        .and_then(|value| parse_bool(&value))
}


pub(super) fn effective_auth_mode(
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


pub(super) fn request_requires_auth(
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

