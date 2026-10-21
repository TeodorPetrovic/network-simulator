use super::*;

#[tokio::test]
async fn classroom_flow_isolates_students_by_project_and_pool_acl() {
    let (_directory, app, pool) = fixture().await;
    let admin = token(&pool, "admin", 0).await;

    // The instructor builds a student role with full control inside a lab.
    let role = admin_create(
        &app,
        &admin,
        "/v3/access/roles",
        json!({"name":"Classroom student"}),
    )
    .await;
    let role_id = role["role_id"].as_str().unwrap().to_owned();
    let (status, privileges) =
        json_route(&app, "GET", "/v3/access/privileges", &admin, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    let lab_privileges = privileges
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| {
            let name = entry["name"].as_str().unwrap();
            ["Project.", "Node.", "Link.", "Snapshot.", "Drawing."]
                .iter()
                .any(|prefix| name.starts_with(prefix))
        })
        .map(|entry| entry["privilege_id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert!(
        lab_privileges.len() >= 15,
        "seeded lab privileges: {lab_privileges:?}"
    );
    for privilege_id in &lab_privileges {
        admin_no_content(
            &app,
            &admin,
            "PUT",
            &format!("/v3/access/roles/{role_id}/privileges/{privilege_id}"),
        )
        .await;
    }

    let mut user_ids = Vec::new();
    for username in ["student-one", "student-two"] {
        let user = admin_create(
            &app,
            &admin,
            "/v3/access/users",
            json!({"username":username,"password":"classroom-password","full_name":username}),
        )
        .await;
        user_ids.push(user["user_id"].as_str().unwrap().to_owned());
    }
    let first = classroom_project(&app, &pool, &admin, "Student one lab").await;
    let second = classroom_project(&app, &pool, &admin, "Student two lab").await;

    // Student one: group membership plus a group ACE on the lab project.
    let group = admin_create(
        &app,
        &admin,
        "/v3/access/groups",
        json!({"name":"Classroom A"}),
    )
    .await;
    let group_id = group["user_group_id"].as_str().unwrap().to_owned();
    admin_no_content(
        &app,
        &admin,
        "PUT",
        &format!("/v3/access/groups/{group_id}/members/{}", user_ids[0]),
    )
    .await;
    admin_create(
        &app,
        &admin,
        "/v3/access/acl",
        json!({"ace_type":"group","group_id":group_id,"role_id":role_id,
        "path":format!("/projects/{}", first.project_id),"propagate":true,"allowed":true}),
    )
    .await;

    // Student two: a user ACE on a resource pool containing the lab project.
    let resource_pool = admin_create(
        &app,
        &admin,
        "/v3/pools",
        json!({"name":"Student two pool"}),
    )
    .await;
    let pool_id = resource_pool["resource_pool_id"]
        .as_str()
        .unwrap()
        .to_owned();
    admin_no_content(
        &app,
        &admin,
        "PUT",
        &format!("/v3/pools/{pool_id}/resources/{}", second.project_id),
    )
    .await;
    admin_create(
        &app,
        &admin,
        "/v3/access/acl",
        json!({"ace_type":"user","user_id":user_ids[1],"role_id":role_id,
        "path":format!("/pools/{pool_id}"),"propagate":true,"allowed":true}),
    )
    .await;

    let student_one = student_login(&app, "student-one", [192, 0, 2, 41]).await;
    let student_two = student_login(&app, "student-two", [192, 0, 2, 42]).await;
    assert_student_isolated(&app, &student_one, "student-one", &first, &second).await;
    assert_student_isolated(&app, &student_two, "student-two", &second, &first).await;

    // Real WebSocket upgrades: own notifications open, the other lab does not.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server_app = app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, server_app).await.unwrap();
    });
    let own_path = format!("/v3/projects/{}/notifications/ws", first.project_id);
    let (status, ticket) = json_route(
        &app,
        "POST",
        "/v3/access/ws-ticket",
        &student_one,
        json!({"path":own_path}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (mut socket, _) = connect_async(format!(
        "ws://{address}{own_path}?ticket={}",
        ticket["ticket"].as_str().unwrap()
    ))
    .await
    .unwrap();
    socket.close(None).await.unwrap();
    let foreign_path = format!("/v3/projects/{}/notifications/ws", second.project_id);
    let rejected = connect_async(format!("ws://{address}{foreign_path}?token={student_one}"))
        .await
        .unwrap_err();
    assert!(
        matches!(rejected, WsError::Http(response) if response.status() == StatusCode::FORBIDDEN)
    );
    let console_path = format!(
        "/v3/compute/projects/{}/vpcs/nodes/{}/console/ws",
        second.project_id, second.node_id
    );
    let rejected = connect_async(format!("ws://{address}{console_path}?token={student_one}"))
        .await
        .unwrap_err();
    assert!(
        matches!(rejected, WsError::Http(response) if response.status() == StatusCode::FORBIDDEN)
    );
    server.abort();

    // Deleting a lab removes its ACEs and pool membership, so a later project
    // with the same ID does not inherit the old grants. The seeded link rows
    // have no runtime endpoint metadata for the delete path to tear down.
    sqlx::query("DELETE FROM topology_objects WHERE kind = 'link'")
        .execute(&pool)
        .await
        .unwrap();
    admin_no_content(
        &app,
        &admin,
        "DELETE",
        &format!("/v3/projects/{}", second.project_id),
    )
    .await;
    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM resources WHERE resource_id = ?")
        .bind(&second.project_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(remaining, 0);
    admin_no_content(
        &app,
        &admin,
        "DELETE",
        &format!("/v3/projects/{}", first.project_id),
    )
    .await;
    let stale: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_acl WHERE path = ?")
        .bind(format!("/projects/{}", first.project_id))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stale, 0);
    let pool_ace: i64 = sqlx::query_scalar("SELECT count(*) FROM auth_acl WHERE path = ?")
        .bind(format!("/pools/{pool_id}"))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(pool_ace, 1, "the pool root ACE is not project-owned");
    admin_create(
        &app,
        &admin,
        "/v3/projects",
        json!({"name":"Reused lab","project_id":first.project_id}),
    )
    .await;
    let (status, projects) =
        json_route(&app, "GET", "/v3/projects", &student_one, Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(projects, json!([]));
}
