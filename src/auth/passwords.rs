use super::*;

const HASH_CONCURRENCY: usize = 4;
const LOGIN_BACKOFF_BASE: Duration = Duration::from_millis(250);
const LOGIN_BACKOFF_MAX: Duration = Duration::from_secs(30);

static PASSWORD_HASH_SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
static LOGIN_ATTEMPTS: OnceLock<Mutex<HashMap<String, LoginAttempt>>> = OnceLock::new();
static DUMMY_PASSWORD_HASH: OnceLock<String> = OnceLock::new();
#[derive(Clone, Copy)]
struct LoginAttempt {
    failures: u32,
    next_allowed: Instant,
    last_seen: Instant,
}

pub(super) fn hash_password(password: &str) -> Result<String, AuthError> {
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

pub(super) async fn hash_password_limited(password: &str) -> Result<String, AuthError> {
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

pub(super) async fn verify_password_limited(password: &str, hash: &str) -> Result<bool, AuthError> {
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

pub(super) async fn dummy_password_hash() -> Result<String, AuthError> {
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

pub(super) fn login_rate_key(username: &str, client_ip: &str) -> String {
    format!("{}\0{}", username.to_ascii_lowercase(), client_ip)
}

pub(super) fn login_rate_limited(key: &str) -> bool {
    let now = Instant::now();
    let mut attempts = login_attempts()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    attempts.retain(|_, attempt| now.duration_since(attempt.last_seen) < Duration::from_secs(3600));
    attempts
        .get(key)
        .is_some_and(|attempt| attempt.next_allowed > now)
}

pub(super) fn record_login_failure(key: String) {
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

pub(super) fn clear_login_failure(key: &str) {
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

pub(super) fn validate_username(username: &str) -> Result<(), AuthError> {
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
