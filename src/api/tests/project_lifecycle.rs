use super::{AppState, router};
use crate::storage::Store;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use tempfile::TempDir;
use tower::ServiceExt;

#[tokio::test]
async fn startup_reopens_projects_marked_auto_open() {
    let directory = TempDir::new().expect("temporary directory");
    let store = Store::connect(directory.path().join("lifecycle.sqlite").to_str().unwrap())
        .await
        .expect("database connects");
    let state = AppState::new(store.clone());
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO projects(project_id,name,status,project_json) VALUES(?,?,'closed',?)")
        .bind(&id)
        .bind("Auto open")
        .bind(serde_json::json!({"project_id":id,"name":"Auto open","status":"closed","auto_open":true,"auto_start":false}).to_string())
        .execute(store.pool())
        .await
        .expect("project inserted");

    state.open_auto_open_projects().await;

    let status: String = sqlx::query_scalar("SELECT status FROM projects WHERE project_id=?")
        .bind(&id)
        .fetch_one(store.pool())
        .await
        .expect("project status loads");
    assert_eq!(status, "opened");
}

#[tokio::test]
async fn open_project_auto_start_path_is_used() {
    let directory = TempDir::new().expect("temporary directory");
    let store = Store::connect(directory.path().join("auto-start.sqlite").to_str().unwrap())
        .await
        .expect("database connects");
    let project_id = uuid::Uuid::new_v4();
    let node_id = uuid::Uuid::new_v4();
    let project = serde_json::json!({"project_id":project_id,"name":"Auto start","status":"closed","auto_start":true});
    sqlx::query("INSERT INTO projects(project_id,name,status,project_json) VALUES(?,?,'closed',?)")
        .bind(project_id.to_string())
        .bind("Auto start")
        .bind(project.to_string())
        .execute(store.pool())
        .await
        .expect("project inserted");
    let node = serde_json::json!({"node_id":node_id,"project_id":project_id,"name":"Cloud","node_type":"cloud","status":"stopped","ports_mapping":[]});
    sqlx::query(
        "INSERT INTO topology_objects(project_id,kind,object_id,object_json) VALUES(?,'node',?,?)",
    )
    .bind(project_id.to_string())
    .bind(node_id.to_string())
    .bind(node.to_string())
    .execute(store.pool())
    .await
    .expect("node inserted");
    let app = router(AppState::new(store.clone()));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v3/projects/{project_id}/open"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let node_json: String = sqlx::query_scalar(
        "SELECT object_json FROM topology_objects WHERE project_id=? AND object_id=?",
    )
    .bind(project_id.to_string())
    .bind(node_id.to_string())
    .fetch_one(store.pool())
    .await
    .expect("node status loads");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&node_json).unwrap()["status"],
        "started"
    );
}

#[tokio::test]
async fn auto_close_closes_project_after_last_notification_disconnects() {
    let directory = TempDir::new().expect("temporary directory");
    let store = Store::connect(directory.path().join("auto-close.sqlite").to_str().unwrap())
        .await
        .expect("database connects");
    let mut state = AppState::new(store.clone());
    state.auto_close_delay = std::time::Duration::from_millis(1);
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO projects(project_id,name,status,project_json) VALUES(?,?,'opened',?)")
        .bind(&id)
        .bind("Auto close")
        .bind(serde_json::json!({"project_id":id,"name":"Auto close","status":"opened","auto_close":true}).to_string())
        .execute(store.pool())
        .await
        .expect("project inserted");

    state.auto_close_after_disconnect(&id).await;

    let status: String = sqlx::query_scalar("SELECT status FROM projects WHERE project_id=?")
        .bind(id)
        .fetch_one(store.pool())
        .await
        .expect("project status loads");
    assert_eq!(status, "closed");
}
