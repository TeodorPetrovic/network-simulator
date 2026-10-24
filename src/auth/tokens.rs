use super::*;

#[derive(Serialize, Deserialize)]
pub(super) struct Claims {
    pub(super) sub: String,
    pub(super) exp: u64,
    pub(super) iat: u64,
    pub(super) nbf: u64,
    pub(super) ver: i64,
    pub(super) jti: String,
    pub(super) iss: String,
    pub(super) aud: String,
    #[serde(default = "access_type")]
    pub(super) token_use: String,
}
pub(super) fn access_type() -> String {
    "access".into()
}

pub(super) fn encode_token(claims: &Claims, key: &str) -> Result<String, AuthError> {
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

pub(super) fn decode_token(token: &str, key: &str) -> Option<Claims> {
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

pub(super) fn new_claims(username: &str, version: i64, token_use: &str, ttl: u64) -> Claims {
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

pub(super) fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(super) async fn signing_key(pool: &SqlitePool) -> Option<String> {
    sqlx::query_scalar("SELECT jwt_signing_key FROM auth_config WHERE singleton = 1")
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

pub(super) async fn authenticate_token(pool: &SqlitePool, token: &str) -> Option<Principal> {
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

pub(super) fn parse_api_key(token: &str) -> Option<(&str, &str)> {
    let mut parts = token.splitn(3, '_');
    if parts.next()? != "gns3" {
        return None;
    }
    Some((parts.next()?, parts.next()?))
}

pub(super) fn random_hex(length: usize) -> Result<String, std::io::Error> {
    let mut bytes = vec![0_u8; length];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| std::io::Error::other("system random source failed"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
