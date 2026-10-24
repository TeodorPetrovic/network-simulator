use super::*;

#[derive(Clone, Default)]
pub(super) struct ProjectActivityTracker {
    projects: Arc<StdMutex<HashMap<String, ProjectActivity>>>,
}

#[derive(Clone, Copy)]
struct ProjectActivity {
    last_activity: Instant,
    open_websockets: usize,
}

impl ProjectActivityTracker {
    pub(super) fn record_request(&self, project_id: &str) {
        let mut projects = self
            .projects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let activity = projects
            .entry(project_id.to_owned())
            .or_insert(ProjectActivity {
                last_activity: Instant::now(),
                open_websockets: 0,
            });
        activity.last_activity = Instant::now();
    }

    pub(super) fn open_websocket(&self, project_id: String) -> ProjectWebSocketGuard {
        let mut projects = self
            .projects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let activity = projects
            .entry(project_id.clone())
            .or_insert(ProjectActivity {
                last_activity: Instant::now(),
                open_websockets: 0,
            });
        activity.last_activity = Instant::now();
        activity.open_websockets = activity.open_websockets.saturating_add(1);
        ProjectWebSocketGuard {
            tracker: self.clone(),
            project_id,
        }
    }

    pub(super) fn is_idle(
        &self,
        project_id: &str,
        fallback_since: Instant,
        idle_for: Duration,
    ) -> bool {
        self.is_idle_at(project_id, fallback_since, idle_for, Instant::now())
    }

    pub(super) fn is_idle_at(
        &self,
        project_id: &str,
        fallback_since: Instant,
        idle_for: Duration,
        now: Instant,
    ) -> bool {
        let projects = self
            .projects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(activity) = projects.get(project_id) else {
            return now.saturating_duration_since(fallback_since) >= idle_for;
        };
        activity.open_websockets == 0
            && now.saturating_duration_since(activity.last_activity) >= idle_for
    }
}

pub(super) struct ProjectWebSocketGuard {
    tracker: ProjectActivityTracker,
    project_id: String,
}

impl Drop for ProjectWebSocketGuard {
    fn drop(&mut self) {
        let mut projects = self
            .tracker
            .projects
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(activity) = projects.get_mut(&self.project_id) {
            activity.open_websockets = activity.open_websockets.saturating_sub(1);
            if activity.open_websockets == 0 {
                activity.last_activity = Instant::now();
            }
        }
    }
}

pub(super) fn configured_idle_stop_minutes() -> Option<u64> {
    let configured = std::env::var("NETWORK_SIMULATOR_IDLE_STOP_MINUTES").ok()?;
    match configured.trim().parse::<u64>() {
        Ok(minutes) if (1..=525_600).contains(&minutes) => Some(minutes),
        _ => {
            tracing::warn!(value = %configured, "ignoring invalid NETWORK_SIMULATOR_IDLE_STOP_MINUTES; expected an integer from 1 to 525600");
            None
        }
    }
}

pub(super) fn project_id_from_path(path: &str) -> Option<String> {
    let segments = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let project_segment = match segments.as_slice() {
        ["v3", "projects", project_id, ..] => *project_id,
        ["v3", "compute", "projects", project_id, ..] => *project_id,
        _ => return None,
    };
    Uuid::parse_str(project_segment)
        .ok()
        .map(|project_id| project_id.to_string())
}

pub(super) async fn track_project_activity(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let project_id = project_id_from_path(request.uri().path());
    let response = next.run(request).await;
    if response.status().is_success() {
        if let Some(project_id) = project_id {
            state.project_activity.record_request(&project_id);
        }
    }
    response
}
