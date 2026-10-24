use super::*;

use std::time::{Duration, Instant};

use super::{ProjectActivityTracker, project_id_from_path};

#[test]
fn project_scoped_api_activity_resets_the_idle_window() {
    let project_id = "63fe90f9-485f-4379-b78c-8d8f984a62b2";
    let tracker = ProjectActivityTracker::default();
    let fallback = Instant::now() - Duration::from_secs(600);
    let idle_for = Duration::from_secs(300);

    assert!(tracker.is_idle(project_id, fallback, idle_for));
    tracker.record_request(project_id);
    assert!(!tracker.is_idle(project_id, fallback, idle_for));
    assert!(tracker.is_idle_at(
        project_id,
        fallback,
        idle_for,
        Instant::now() + Duration::from_secs(301)
    ));
}

#[test]
fn an_open_project_websocket_prevents_idle_stop_and_disconnect_resets_timer() {
    let project_id = "63fe90f9-485f-4379-b78c-8d8f984a62b2";
    let tracker = ProjectActivityTracker::default();
    let fallback = Instant::now() - Duration::from_secs(600);
    let idle_for = Duration::from_secs(300);
    let connection = tracker.open_websocket(project_id.to_owned());

    assert!(!tracker.is_idle_at(
        project_id,
        fallback,
        idle_for,
        Instant::now() + Duration::from_secs(3600)
    ));
    drop(connection);
    assert!(!tracker.is_idle(project_id, fallback, idle_for));
    assert!(tracker.is_idle_at(
        project_id,
        fallback,
        idle_for,
        Instant::now() + Duration::from_secs(301)
    ));
}

#[test]
fn activity_routes_extract_ids_for_controller_and_compute_websockets() {
    let project_id = "63fe90f9-485f-4379-b78c-8d8f984a62b2";
    assert_eq!(
        project_id_from_path(&format!("/v3/projects/{project_id}/nodes")),
        Some(project_id.to_owned())
    );
    assert_eq!(
        project_id_from_path(&format!(
            "/v3/compute/projects/{project_id}/vpcs/nodes/00000000-0000-0000-0000-000000000001/console/ws"
        )),
        Some(project_id.to_owned())
    );
    assert_eq!(project_id_from_path("/v3/projects/not-a-uuid/nodes"), None);
    assert_eq!(project_id_from_path("/v3/projects"), None);
}
