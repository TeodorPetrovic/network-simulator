use super::*;

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

