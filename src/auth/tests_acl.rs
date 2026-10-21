use super::*;

#[tokio::test]
async fn pool_access_obeys_propagation_and_direct_user_denies() {
    let (_directory, app, pool) = fixture().await;
    let project_id = Uuid::new_v4().to_string();
    let pool_id = Uuid::new_v4().to_string();
    let alice_id: String =
        sqlx::query_scalar("SELECT user_id FROM auth_users WHERE username = 'alice'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO projects (project_id, name, status, project_json) VALUES (?, 'Pool ACL project', 'closed', ?)")
        .bind(&project_id).bind(json!({"project_id":project_id,"name":"Pool ACL project","status":"closed"}).to_string())
        .execute(&pool).await.unwrap();
    let other_project = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO projects (project_id, name, status, project_json) VALUES (?, 'Other pool project', 'closed', ?)")
        .bind(&other_project).bind(json!({"project_id":other_project,"name":"Other pool project","status":"closed"}).to_string())
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO resource_pools (resource_pool_id, name) VALUES (?, 'ACL test pool')")
        .bind(&pool_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO resources (resource_id, resource_type, name) VALUES (?, 'project', 'ACL test project')")
        .bind(&project_id).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO resource_pool_map (resource_id, resource_pool_id) VALUES (?, ?)")
        .bind(&project_id)
        .bind(&pool_id)
        .execute(&pool)
        .await
        .unwrap();
    let auditor_role: String =
        sqlx::query_scalar("SELECT role_id FROM auth_roles WHERE name = 'Auditor'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let group_id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO auth_groups (user_group_id, name) VALUES (?, 'Pool ACL group')")
        .bind(&group_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO auth_user_groups (user_id, user_group_id) VALUES (?, ?)")
        .bind(&alice_id)
        .bind(&group_id)
        .execute(&pool)
        .await
        .unwrap();
    let root = format!("/pools/{pool_id}");
    sqlx::query("INSERT INTO auth_acl (ace_id, ace_type, path, propagate, allowed, group_id, role_id) VALUES (?, 'group', ?, 1, 1, ?, ?)")
        .bind(Uuid::new_v4().to_string()).bind(&root).bind(&group_id).bind(&auditor_role).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO auth_acl (ace_id, ace_type, path, propagate, allowed, user_id, role_id) VALUES (?, 'user', ?, 0, 0, ?, ?)")
        .bind(Uuid::new_v4().to_string()).bind(&root).bind(&alice_id).bind(&auditor_role).execute(&pool).await.unwrap();
    let principal = Principal {
        user_id: alice_id.clone(),
        is_superadmin: false,
        must_change_password: false,
    };
    let project_path = format!("/projects/{project_id}");
    assert!(!has_privilege(&pool, &principal, &project_path, "Project.Audit").await);

    sqlx::query("DELETE FROM auth_acl WHERE ace_type = 'user' AND user_id = ?")
        .bind(&alice_id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(has_privilege(&pool, &principal, &project_path, "Project.Audit").await);
    let alice_token = token(&pool, "alice", 0).await;
    let (status, projects) =
        json_route(&app, "GET", "/v3/projects", &alice_token, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(projects.as_array().unwrap().len(), 1);
    assert_eq!(projects[0]["project_id"], project_id);
    sqlx::query("UPDATE auth_acl SET propagate = 0 WHERE group_id = ?")
        .bind(&group_id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(!has_privilege(&pool, &principal, &project_path, "Project.Audit").await);

    sqlx::query("UPDATE auth_acl SET path = ?, propagate = 0 WHERE group_id = ?")
        .bind(format!("{root}/resources/{project_id}"))
        .bind(&group_id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(has_privilege(&pool, &principal, &project_path, "Project.Audit").await);
}

#[test]
fn equal_depth_acl_denies_are_order_independent() {
    let allow = ("/projects/classroom".to_owned(), true, true);
    let deny = ("/projects/classroom".to_owned(), false, false);
    assert_eq!(
        evaluate_acl("/projects/classroom", &[allow.clone(), deny.clone()]),
        Some(false)
    );
    assert_eq!(
        evaluate_acl("/projects/classroom", &[deny, allow]),
        Some(false)
    );
}

async fn insert_ace(
    pool: &SqlitePool,
    principal: (&str, &str),
    path: &str,
    propagate: bool,
    allowed: bool,
    role_id: &str,
) {
    let (ace_type, principal_id) = principal;
    let (user_id, group_id) = if ace_type == "user" {
        (Some(principal_id), None)
    } else {
        (None, Some(principal_id))
    };
    sqlx::query("INSERT INTO auth_acl (ace_id, ace_type, path, propagate, allowed, user_id, group_id, role_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(Uuid::new_v4().to_string()).bind(ace_type).bind(path).bind(propagate).bind(allowed)
        .bind(user_id).bind(group_id).bind(role_id)
        .execute(pool).await.unwrap();
}

#[tokio::test]
async fn user_aces_replace_group_aces_and_pool_aces_join_depth_ordering() {
    let (_directory, _app, pool) = fixture().await;
    let alice_id: String =
        sqlx::query_scalar("SELECT user_id FROM auth_users WHERE username = 'alice'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let role_id: String = sqlx::query_scalar("SELECT role_id FROM auth_roles WHERE name = 'User'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let group_id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO auth_groups (user_group_id, name) VALUES (?, 'Precedence group')")
        .bind(&group_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO auth_user_groups (user_id, user_group_id) VALUES (?, ?)")
        .bind(&alice_id)
        .bind(&group_id)
        .execute(&pool)
        .await
        .unwrap();
    let project_id = Uuid::new_v4().to_string();
    let pool_id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO projects (project_id, name, status, project_json) VALUES (?, 'Precedence', 'closed', '{}')")
        .bind(&project_id).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO resource_pools (resource_pool_id, name) VALUES (?, 'Precedence pool')",
    )
    .bind(&pool_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO resources (resource_id, resource_type, name) VALUES (?, 'project', 'Precedence')")
        .bind(&project_id).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO resource_pool_map (resource_id, resource_pool_id) VALUES (?, ?)")
        .bind(&project_id)
        .bind(&pool_id)
        .execute(&pool)
        .await
        .unwrap();
    let principal = Principal {
        user_id: alice_id.clone(),
        is_superadmin: false,
        must_change_password: false,
    };
    let user = ("user", alice_id.as_str());
    let group = ("group", group_id.as_str());
    let project = format!("/projects/{project_id}");
    let nodes = format!("{project}/nodes");
    let pool_root = format!("/pools/{pool_id}");
    let clear = || async {
        sqlx::query("DELETE FROM auth_acl")
            .execute(&pool)
            .await
            .unwrap();
    };

    // A user allow replaces a group deny on the same path.
    insert_ace(&pool, group, &project, true, false, &role_id).await;
    insert_ace(&pool, user, &project, true, true, &role_id).await;
    assert!(has_privilege(&pool, &principal, &project, "Project.Audit").await);
    clear().await;

    // A user deny on an ancestor replaces a deeper group allow.
    insert_ace(&pool, user, "/projects", true, false, &role_id).await;
    insert_ace(&pool, group, &project, true, true, &role_id).await;
    assert!(!has_privilege(&pool, &principal, &project, "Project.Audit").await);
    clear().await;

    // A pool deny acts at project depth, so it beats an inherited allow.
    insert_ace(&pool, group, "/", true, true, &role_id).await;
    insert_ace(&pool, group, &pool_root, false, false, &role_id).await;
    assert!(!has_privilege(&pool, &principal, &project, "Project.Audit").await);
    assert!(has_privilege(&pool, &principal, "/projects/unpooled", "Project.Audit").await);
    clear().await;

    // A pool grant still yields to a deeper direct deny.
    insert_ace(&pool, group, &pool_root, true, true, &role_id).await;
    insert_ace(&pool, group, &nodes, true, false, &role_id).await;
    assert!(has_privilege(&pool, &principal, &project, "Project.Audit").await);
    assert!(!has_privilege(&pool, &principal, &nodes, "Node.Audit").await);
    clear().await;

    // Pool descendants other than the project's resource entry grant nothing,
    // and a non-propagating resource entry covers only the project itself.
    insert_ace(
        &pool,
        group,
        &format!("{pool_root}/resources"),
        true,
        true,
        &role_id,
    )
    .await;
    assert!(!has_privilege(&pool, &principal, &project, "Project.Audit").await);
    clear().await;
    insert_ace(
        &pool,
        group,
        &format!("{pool_root}/resources/{project_id}"),
        false,
        true,
        &role_id,
    )
    .await;
    assert!(has_privilege(&pool, &principal, &project, "Project.Audit").await);
    assert!(!has_privilege(&pool, &principal, &nodes, "Node.Audit").await);
    clear().await;

    // ACE rows are evaluated identically regardless of insertion order.
    for order in [[true, false], [false, true]] {
        for allowed in order {
            insert_ace(&pool, group, &project, true, allowed, &role_id).await;
        }
        assert!(!has_privilege(&pool, &principal, &project, "Project.Audit").await);
        clear().await;
    }
}
