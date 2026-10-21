use sqlx::{Row, SqlitePool};

type AclTuple = (String, bool, bool);

/// Resolves direct and pool-derived entries before checking group entries.
/// A user's direct decision replaces group decisions, including denials.
pub(super) async fn privilege_decision(
    pool: &SqlitePool,
    user_id: &str,
    path: &str,
    privilege: &str,
) -> Result<bool, sqlx::Error> {
    let user = direct_user_aces(pool, user_id, privilege).await?;
    if let Some(decision) = principal_acl_decision(pool, path, user).await? {
        return Ok(decision);
    }
    let group = group_aces(pool, user_id, privilege).await?;
    Ok(principal_acl_decision(pool, path, group)
        .await?
        .unwrap_or(false))
}

/// Console routes use a different URL prefix from their project's resources.
pub(super) fn canonical_resource_path(path: &str) -> String {
    let parts = path.trim_matches('/').split('/').collect::<Vec<_>>();
    if parts.len() >= 7
        && parts[0] == "compute"
        && parts[1] == "projects"
        && parts[4] == "nodes"
        && matches!(parts[6], "console" | "aux_console")
    {
        return format!("/projects/{}/nodes/{}/console", parts[2], parts[5]);
    }
    path.to_owned()
}

async fn principal_acl_decision(
    pool: &SqlitePool,
    path: &str,
    mut aces: Vec<AclTuple>,
) -> Result<Option<bool>, sqlx::Error> {
    if aces.is_empty() {
        return Ok(None);
    }
    aces.extend(pool_project_aces(pool, path, &aces).await?);
    Ok(evaluate_acl(path, &aces))
}

async fn direct_user_aces(
    pool: &SqlitePool,
    user_id: &str,
    privilege: &str,
) -> Result<Vec<AclTuple>, sqlx::Error> {
    let rows = sqlx::query("SELECT a.path, a.propagate, a.allowed FROM auth_acl a JOIN auth_role_privileges rp ON rp.role_id = a.role_id JOIN auth_privileges p ON p.privilege_id = rp.privilege_id WHERE a.ace_type = 'user' AND a.user_id = ? AND p.name = ?")
        .bind(user_id).bind(privilege).fetch_all(pool).await?;
    Ok(rows.into_iter().map(row_acl_tuple).collect())
}

async fn group_aces(
    pool: &SqlitePool,
    user_id: &str,
    privilege: &str,
) -> Result<Vec<AclTuple>, sqlx::Error> {
    let rows = sqlx::query("SELECT DISTINCT a.path, a.propagate, a.allowed FROM auth_acl a JOIN auth_user_groups m ON m.user_group_id = a.group_id JOIN auth_role_privileges rp ON rp.role_id = a.role_id JOIN auth_privileges p ON p.privilege_id = rp.privilege_id WHERE a.ace_type = 'group' AND m.user_id = ? AND p.name = ?")
        .bind(user_id).bind(privilege).fetch_all(pool).await?;
    Ok(rows.into_iter().map(row_acl_tuple).collect())
}

fn row_acl_tuple(row: sqlx::sqlite::SqliteRow) -> AclTuple {
    (row.get("path"), row.get("propagate"), row.get("allowed"))
}

/// Pool entries are mapped onto their member project so path depth and
/// deny-first ordering match regular project ACEs.
async fn pool_project_aces(
    pool: &SqlitePool,
    path: &str,
    aces: &[AclTuple],
) -> Result<Vec<AclTuple>, sqlx::Error> {
    let Some(project_id) = project_id_from_path(path) else {
        return Ok(Vec::new());
    };
    if !aces
        .iter()
        .any(|(ace_path, _, _)| super::normalize_acl_path(ace_path).starts_with("/pools/"))
    {
        return Ok(Vec::new());
    }
    let pool_ids: Vec<String> = sqlx::query_scalar("SELECT m.resource_pool_id FROM resource_pool_map m JOIN resources r ON r.resource_id = m.resource_id WHERE r.resource_type = 'project' AND r.resource_id = ?")
        .bind(project_id).fetch_all(pool).await?;
    let project_path = format!("/projects/{project_id}");
    let mut translated = Vec::new();
    for pool_id in pool_ids {
        let root = format!("/pools/{pool_id}");
        let resource = format!("{root}/resources/{project_id}");
        for (ace_path, propagate, allowed) in aces {
            let ace_path = super::normalize_acl_path(ace_path);
            if ace_path == resource {
                translated.push((project_path.clone(), *propagate, *allowed));
            } else if ace_path == root && (*propagate || !*allowed) {
                translated.push((project_path.clone(), true, *allowed));
            }
        }
    }
    Ok(translated)
}

pub(super) fn evaluate_acl(path: &str, aces: &[(String, bool, bool)]) -> Option<bool> {
    let path = super::normalize_acl_path(path);
    let mut applicable = aces
        .iter()
        .filter(|(ace_path, _, _)| {
            let ace_path = super::normalize_acl_path(ace_path);
            path == ace_path
                || path
                    .strip_prefix(&ace_path)
                    .is_some_and(|suffix| ace_path == "/" || suffix.starts_with('/'))
        })
        .collect::<Vec<_>>();
    applicable.sort_by_key(|(ace_path, _, allowed)| {
        (
            std::cmp::Reverse(super::normalize_acl_path(ace_path).len()),
            *allowed,
            super::normalize_acl_path(ace_path),
        )
    });
    for (ace_path, propagate, allowed) in applicable {
        let ace_path = super::normalize_acl_path(ace_path);
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

fn project_id_from_path(path: &str) -> Option<&str> {
    let mut segments = path.split('/').filter(|segment| !segment.is_empty());
    (segments.next()? == "projects")
        .then(|| segments.next())
        .flatten()
}
