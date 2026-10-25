use super::*;

#[tokio::test]
async fn refresh_tokens_rotate_once_and_reuse_revokes_the_family() {
    let (_directory, app, pool) = fixture().await;
    let login = response(
        &app,
        from_peer(
            Request::builder()
                .method("POST")
                .uri("/v3/access/users/login")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("username=admin&password=correct-password"))
                .unwrap(),
            [192, 0, 2, 22],
        ),
    )
    .await;
    assert_eq!(login.status(), StatusCode::OK);
    let login: Value =
        serde_json::from_slice(&to_bytes(login.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(login["expires_in"], ACCESS_TTL);
    assert_eq!(login["refresh_expires_in"], REFRESH_TTL);
    let original = login["refresh_token"].as_str().unwrap().to_owned();
    let claims = decode_token(&original, "test-signing-key").unwrap();
    assert_eq!(claims.exp - claims.iat, REFRESH_TTL);
    let rotate = |refresh_token: &str| {
        Request::builder()
            .method("POST")
            .uri("/v3/access/users/refresh")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({"refresh_token":refresh_token}).to_string(),
            ))
            .unwrap()
    };
    let rotated = response(&app, rotate(&original)).await;
    assert_eq!(rotated.status(), StatusCode::OK);
    let rotated: Value =
        serde_json::from_slice(&to_bytes(rotated.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(rotated["expires_in"], ACCESS_TTL);
    assert_eq!(rotated["refresh_expires_in"], REFRESH_TTL);
    let replacement = rotated["refresh_token"].as_str().unwrap().to_owned();
    assert_ne!(replacement, original);

    let reuse = response(&app, rotate(&original)).await;
    assert_eq!(reuse.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        response(&app, rotate(&replacement)).await.status(),
        StatusCode::UNAUTHORIZED
    );
    let revoked: bool = sqlx::query_scalar("SELECT revoked FROM auth_refresh_families")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(revoked);
}

#[tokio::test]
async fn ws_ticket_is_short_lived_path_bound_and_single_use() {
    let (_directory, app, pool) = fixture().await;
    let project_id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO projects (project_id,name,status,project_json) VALUES (?, 'Ticket project','opened','{}')")
        .bind(&project_id).execute(&pool).await.unwrap();
    let admin_token = token(&pool, "admin", 0).await;
    let path = format!("/v3/projects/{project_id}/notifications/ws");
    let ticket_response = response(
        &app,
        Request::builder()
            .method("POST")
            .uri("/v3/access/ws-ticket")
            .header(header::AUTHORIZATION, format!("Bearer {admin_token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({"path":path}).to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(ticket_response.status(), StatusCode::OK);
    let ticket_body: Value = serde_json::from_slice(
        &to_bytes(ticket_response.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(ticket_body["expires_in"], WS_TICKET_TTL);
    let ticket = ticket_body["ticket"].as_str().unwrap();
    let app = app.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let endpoint = format!("ws://{address}{path}?ticket={ticket}");
    let (mut socket, _) = connect_async(&endpoint).await.unwrap();
    socket.close(None).await.unwrap();
    let reused = connect_async(&endpoint).await.unwrap_err();
    assert!(
        matches!(reused, WsError::Http(response) if response.status() == StatusCode::UNAUTHORIZED)
    );
    server.abort();
}

#[tokio::test]
async fn missing_auth_config_fails_closed_and_bootstrap_recovers_admin() {
    let (directory, app, pool) = fixture().await;
    let admin_token = token(&pool, "admin", 0).await;
    sqlx::query("DELETE FROM auth_config WHERE singleton = 1")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        response(&app, bearer("/v3/runtimes", &admin_token))
            .await
            .status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    sqlx::query("INSERT INTO auth_config (singleton, auth_enabled, jwt_signing_key) VALUES (1, 1, 'test-signing-key')")
        .execute(&pool).await.unwrap();
    sqlx::query("UPDATE auth_users SET is_active = 0 WHERE username = 'admin'")
        .execute(&pool)
        .await
        .unwrap();
    let password_file = directory.path().join("initial-admin-password.txt");
    initialize(&pool, true, &password_file).await.unwrap();
    let active_admin: bool =
        sqlx::query_scalar("SELECT is_active FROM auth_users WHERE username = 'admin'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let must_change: bool =
        sqlx::query_scalar("SELECT must_change_password FROM auth_users WHERE username = 'admin'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(active_admin && must_change);
    assert!(
        !std::fs::read_to_string(&password_file)
            .unwrap()
            .trim()
            .is_empty()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&password_file)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[tokio::test]
async fn last_active_superadmin_cannot_be_disabled_and_initial_password_is_required() {
    let (_directory, app, pool) = fixture().await;
    let admin_id: String =
        sqlx::query_scalar("SELECT user_id FROM auth_users WHERE username = 'admin'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let admin_token = token(&pool, "admin", 0).await;
    let disable = Request::builder()
        .method("PUT")
        .uri(format!("/v3/access/users/{admin_id}"))
        .header(header::AUTHORIZATION, format!("Bearer {admin_token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"is_active":false}"#))
        .unwrap();
    assert_eq!(
        response(&app, disable).await.status(),
        StatusCode::FORBIDDEN
    );

    let alice_id: String =
        sqlx::query_scalar("SELECT user_id FROM auth_users WHERE username = 'alice'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("UPDATE auth_users SET must_change_password = 1 WHERE user_id = ?")
        .bind(&alice_id)
        .execute(&pool)
        .await
        .unwrap();
    let alice_token = token(&pool, "alice", 0).await;
    assert_eq!(
        response(&app, bearer("/v3/runtimes", &alice_token))
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let change = Request::builder()
        .method("PUT")
        .uri("/v3/access/users/me")
        .header(header::AUTHORIZATION, format!("Bearer {alice_token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"password":"new-correct-password"}"#))
        .unwrap();
    assert_eq!(response(&app, change).await.status(), StatusCode::OK);
    let must_change: bool =
        sqlx::query_scalar("SELECT must_change_password FROM auth_users WHERE user_id = ?")
            .bind(alice_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!must_change);
}
