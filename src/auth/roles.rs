use super::*;

pub(super) async fn list_roles(State(state): State<AppState>) -> Result<Json<Value>, AuthError> {
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
pub(super) async fn get_role(
    State(state): State<AppState>,
    Path(role_id): Path<String>,
) -> Result<Json<Value>, AuthError> {
    role_value(state.store.pool(), &role_id).await.map(Json)
}
pub(super) async fn create_role(
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
pub(super) async fn update_role(
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
pub(super) async fn delete_role(
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
pub(super) async fn role_privileges(
    State(state): State<AppState>,
    Path(role_id): Path<String>,
) -> Result<Json<Value>, AuthError> {
    let rows = sqlx::query("SELECT p.* FROM auth_privileges p JOIN auth_role_privileges rp ON rp.privilege_id = p.privilege_id WHERE rp.role_id = ? ORDER BY p.name").bind(role_id).fetch_all(state.store.pool()).await.map_err(db_error)?;
    Ok(Json(Value::Array(
        rows.iter().map(privilege_json).collect(),
    )))
}
pub(super) async fn add_role_privilege(
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
pub(super) async fn remove_role_privilege(
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
pub(super) async fn role_value(pool: &SqlitePool, role_id: &str) -> Result<Value, AuthError> {
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
pub(super) async fn list_privileges(
    State(state): State<AppState>,
) -> Result<Json<Value>, AuthError> {
    let rows = sqlx::query("SELECT * FROM auth_privileges ORDER BY name")
        .fetch_all(state.store.pool())
        .await
        .map_err(db_error)?;
    Ok(Json(Value::Array(
        rows.iter().map(privilege_json).collect(),
    )))
}
pub(super) fn privilege_json(row: &sqlx::sqlite::SqliteRow) -> Value {
    json!({"privilege_id": row.get::<String,_>("privilege_id"), "name": row.get::<String,_>("name"), "description": row.get::<Option<String>,_>("description"), "created_at": row.get::<String,_>("created_at"), "updated_at": row.get::<String,_>("updated_at")})
}

pub(super) async fn list_acl(State(state): State<AppState>) -> Result<Json<Value>, AuthError> {
    let rows = sqlx::query("SELECT * FROM auth_acl ORDER BY path")
        .fetch_all(state.store.pool())
        .await
        .map_err(db_error)?;
    Ok(Json(Value::Array(rows.iter().map(acl_json).collect())))
}
pub(super) async fn get_acl(
    State(state): State<AppState>,
    Path(ace_id): Path<String>,
) -> Result<Json<Value>, AuthError> {
    acl_value(state.store.pool(), &ace_id).await.map(Json)
}
pub(super) async fn create_acl(
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
pub(super) async fn update_acl(
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
pub(super) async fn delete_acl(
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
pub(super) async fn acl_value(pool: &SqlitePool, ace_id: &str) -> Result<Value, AuthError> {
    let row = sqlx::query("SELECT * FROM auth_acl WHERE ace_id = ?")
        .bind(ace_id)
        .fetch_optional(pool)
        .await
        .map_err(db_error)?
        .ok_or_else(|| AuthError::not_found("ACL entry not found"))?;
    Ok(acl_json(&row))
}
pub(super) fn acl_json(row: &sqlx::sqlite::SqliteRow) -> Value {
    json!({"ace_id": row.get::<String,_>("ace_id"), "ace_type": row.get::<String,_>("ace_type"), "path": row.get::<String,_>("path"), "propagate": row.get::<bool,_>("propagate"), "allowed": row.get::<bool,_>("allowed"), "user_id": row.get::<Option<String>,_>("user_id"), "group_id": row.get::<Option<String>,_>("group_id"), "role_id": row.get::<String,_>("role_id"), "created_at": row.get::<String,_>("created_at"), "updated_at": row.get::<String,_>("updated_at")})
}
pub(super) async fn acl_endpoints(State(state): State<AppState>) -> Result<Json<Value>, AuthError> {
    let mut values = vec![
        json!({"endpoint":"/", "name":"All endpoints", "endpoint_type":"root"}),
        json!({"endpoint":"/projects", "name":"All projects", "endpoint_type":"project"}),
        json!({"endpoint":"/access/users", "name":"All users", "endpoint_type":"user"}),
        json!({"endpoint":"/access/groups", "name":"All groups", "endpoint_type":"group"}),
        json!({"endpoint":"/access/roles", "name":"All roles", "endpoint_type":"role"}),
    ];
    let projects = sqlx::query("SELECT project_id, name FROM projects ORDER BY name")
        .fetch_all(state.store.pool())
        .await
        .map_err(db_error)?;
    for project in projects {
        let project_id: String = project.get("project_id");
        let project_name: String = project.get("name");
        values.push(json!({
            "endpoint":format!("/projects/{project_id}/nodes"),
            "name":format!("All nodes in project \"{project_name}\""),
            "endpoint_type":"node"
        }));
        values.push(json!({
            "endpoint":format!("/projects/{project_id}/links"),
            "name":format!("All links in project \"{project_name}\""),
            "endpoint_type":"link"
        }));
        let objects = sqlx::query("SELECT kind, object_id, object_json FROM topology_objects WHERE project_id = ? AND kind IN ('node', 'link') ORDER BY kind, name")
            .bind(&project_id)
            .fetch_all(state.store.pool())
            .await
            .map_err(db_error)?;
        let mut node_names = HashMap::new();
        let mut links = Vec::new();
        for object in objects {
            let kind: String = object.get("kind");
            let object_id: String = object.get("object_id");
            let value: Value = serde_json::from_str(&object.get::<String, _>("object_json"))
                .unwrap_or(Value::Null);
            if kind == "node" {
                let name = value["name"].as_str().unwrap_or("Node").to_owned();
                node_names.insert(object_id.clone(), name.clone());
                values.push(json!({
                    "endpoint":format!("/projects/{project_id}/nodes/{object_id}"),
                    "name":format!("Node \"{name}\" in project \"{project_name}\""),
                    "endpoint_type":"node"
                }));
            } else {
                links.push((object_id, value));
            }
        }
        for (link_id, link) in links {
            let endpoints = link["nodes"].as_array();
            let left = endpoints
                .and_then(|nodes| nodes.first())
                .and_then(|node| node["node_id"].as_str())
                .and_then(|id| node_names.get(id))
                .map(String::as_str)
                .unwrap_or("N/A");
            let right = endpoints
                .and_then(|nodes| nodes.get(1))
                .and_then(|node| node["node_id"].as_str())
                .and_then(|id| node_names.get(id))
                .map(String::as_str)
                .unwrap_or("N/A");
            values.push(json!({
                "endpoint":format!("/projects/{project_id}/links/{link_id}"),
                "name":format!("Link from \"{left}\" to \"{right}\" in project \"{project_name}\""),
                "endpoint_type":"link"
            }));
        }
    }
    // Templates and image catalog entries are controller resources rather than
    // project objects, so expose them at their API paths in the ACL picker.
    let templates = sqlx::query(
        "SELECT template_id, template_json FROM controller_templates ORDER BY template_id",
    )
    .fetch_all(state.store.pool())
    .await
    .map_err(db_error)?;
    for row in templates {
        let id: String = row.get("template_id");
        let template: Value =
            serde_json::from_str(&row.get::<String, _>("template_json")).unwrap_or(Value::Null);
        let name = template["name"].as_str().unwrap_or("Template");
        values.push(json!({"endpoint":format!("/templates/{id}"),"name":format!("Template \"{name}\""),"endpoint_type":"template"}));
    }
    for (kind, table) in [("QEMU", "qemu_images"), ("Dynamips", "dynamips_images")] {
        let directory = match table {
            "qemu_images" => crate::image_paths::qemu_images(),
            _ => crate::image_paths::dynamips_images(),
        };
        if let Ok(entries) = std::fs::read_dir(directory) {
            for entry in entries
                .flatten()
                .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            {
                let name = entry.file_name().to_string_lossy().into_owned();
                values.push(json!({"endpoint":format!("/images/{}", name),"name":format!("{kind} image \"{name}\""),"endpoint_type":"image"}));
            }
        }
    }
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
