use super::*;

impl super::AppState {
    /// Kill runtime processes a previous controller run left behind.
    pub fn reap_orphaned_runtimes(&self) {
        let killed = self.vpcs.reap_orphans();
        if killed > 0 {
            tracing::warn!(killed, "killed orphaned VPCS processes from an earlier run");
        }
        let killed = self.dynamips.reap_orphans();
        if killed > 0 {
            tracing::warn!(
                killed,
                "killed orphaned Dynamips hypervisors from an earlier run"
            );
        }
    }

    /// Terminate processes this controller owns that cannot outlive it.
    pub async fn shutdown_runtimes(&self) {
        let rows = match sqlx::query(
            "SELECT project_id, object_id, object_json FROM topology_objects
             WHERE kind = 'node' AND json_extract(object_json, '$.status') IN ('started', 'suspended')",
        )
        .fetch_all(self.store.pool())
        .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::error!(%error, "could not load running nodes for graceful shutdown");
                Vec::new()
            }
        };
        for row in rows {
            let project_id: String = row.get("project_id");
            let node_id: String = row.get("object_id");
            let node = match decode_json(row.get::<String, _>("object_json")) {
                Ok(node) => node,
                Err(error) => {
                    tracing::warn!(error = %error.message, %project_id, %node_id, "skipping invalid node during graceful shutdown");
                    continue;
                }
            };
            if !controller_routes::bulk_node_action_applies(&node, NodeAction::Stop) {
                continue;
            }
            let (Ok(project_uuid), Ok(node_uuid)) =
                (Uuid::parse_str(&project_id), Uuid::parse_str(&node_id))
            else {
                tracing::warn!(%project_id, %node_id, "skipping node with an invalid ID during graceful shutdown");
                continue;
            };
            if let Err(error) =
                change_node_runtime_state(self.clone(), project_uuid, node_uuid, NodeAction::Stop)
                    .await
            {
                tracing::error!(error = %error.message, %project_id, %node_id, "could not stop node during graceful shutdown");
            }
        }
        self.dynamips.kill_all().await;
        self.ubridge.shutdown().await;
        self.vpcs.kill_all().await;
    }

    pub(crate) fn start_idle_stop_worker(&self) {
        let Some(minutes) = self.idle_stop_minutes.filter(|minutes| *minutes > 0) else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::warn!("idle stop is configured but no Tokio runtime is active");
            return;
        };
        let state = self.clone();
        handle.spawn(async move {
            let interval = Duration::from_secs(minutes.saturating_mul(60));
            let mut ticker = tokio::time::interval(interval.min(Duration::from_secs(60)));
            ticker.tick().await;
            loop {
                ticker.tick().await;
                state.stop_idle_projects(interval).await;
            }
        });
        tracing::info!(idle_stop_minutes = minutes, "project idle stop enabled");
    }

    pub(crate) async fn stop_idle_projects(&self, idle_for: Duration) {
        let rows = match sqlx::query("SELECT project_id FROM projects ORDER BY project_id")
            .fetch_all(self.store.pool())
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::error!(%error, "could not list projects for idle stop");
                return;
            }
        };
        for row in rows {
            let project_id: String = row.get("project_id");
            if self
                .project_activity
                .is_idle(&project_id, self.started_at, idle_for)
            {
                self.stop_idle_project(&project_id, idle_for).await;
            }
        }
    }

    pub(crate) async fn stop_idle_project(&self, project_id: &str, idle_for: Duration) {
        let _project_lock = self.lock_project_operations(project_id).await;
        if !self
            .project_activity
            .is_idle(project_id, self.started_at, idle_for)
        {
            return;
        }
        let rows = match sqlx::query(
            "SELECT object_id, object_json FROM topology_objects
             WHERE project_id = ? AND kind = 'node' ORDER BY object_id",
        )
        .bind(project_id)
        .fetch_all(self.store.pool())
        .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::error!(%error, %project_id, "could not load nodes for idle stop");
                return;
            }
        };
        for row in rows {
            if !self
                .project_activity
                .is_idle(project_id, self.started_at, idle_for)
            {
                return;
            }
            let node_id: String = row.get("object_id");
            let mut node = match decode_json(row.get::<String, _>("object_json")) {
                Ok(node) => node,
                Err(error) => {
                    tracing::warn!(error = %error.message, %project_id, %node_id, "skipping invalid node during idle stop");
                    continue;
                }
            };
            if !controller_routes::bulk_node_action_applies(&node, NodeAction::Stop) {
                continue;
            }
            match apply_node_lifecycle(self, project_id, &mut node, NodeAction::Stop).await {
                Ok(status) => {
                    node["status"] = json!(status);
                    let result = async {
                        let mut tx = self.store.begin_write().await?;
                        sqlx::query(
                            "UPDATE topology_objects SET object_json = ?
                             WHERE project_id = ? AND kind = 'node' AND object_id = ?",
                        )
                        .bind(serde_json::to_string(&node).map_err(internal_json_error)?)
                        .bind(project_id)
                        .bind(&node_id)
                        .execute(&mut *tx)
                        .await?;
                        let event =
                            Store::append_event(&mut tx, project_id, "node.updated", &node).await?;
                        tx.commit().await?;
                        Ok::<_, ApiError>(event)
                    }
                    .await;
                    match result {
                        Ok(event) => self.store.publish(event),
                        Err(error) => {
                            tracing::error!(error = %error.message, %project_id, %node_id, "could not persist idle-stopped node")
                        }
                    }
                }
                Err(error) => {
                    tracing::error!(error = %error.message, %project_id, %node_id, "could not stop idle node")
                }
            }
        }
    }
}
