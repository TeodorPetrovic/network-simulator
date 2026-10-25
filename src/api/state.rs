use super::*;

impl super::AppState {
    pub async fn open_auto_open_projects(&self) {
        let rows = match sqlx::query(
            "SELECT project_id FROM projects WHERE json_extract(project_json, '$.auto_open') = 1 ORDER BY project_id",
        )
        .fetch_all(self.store.pool())
        .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::error!(%error, "could not load projects configured to open at startup");
                return;
            }
        };
        for row in rows {
            let id: String = row.get("project_id");
            let Ok(id) = Uuid::parse_str(&id) else {
                tracing::warn!(project_id = %id, "skipping project with invalid ID during startup open");
                continue;
            };
            if let Err(error) = super::projects::open_project_lifecycle(self.clone(), id).await {
                tracing::error!(project_id = %id, error = %error.message, "could not open project configured for startup");
            }
        }
    }

    pub fn new(store: Store) -> Self {
        Self {
            store,
            runtime: DockerRuntime::new(),
            qemu: QemuRuntime::new(),
            vpcs: VpcsRuntime::new(),
            dynamips: DynamipsRuntime::new(),
            builtin: BuiltinRuntime::default(),
            ubridge: UbridgeRuntime::new(),
            marketplace: MarketplaceCatalog::new(),
            bind_address: std::env::var("NETWORK_SIMULATOR_BIND")
                .ok()
                .and_then(|address| address.parse().ok())
                .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 3080))),
            started_at: Instant::now(),
            project_operation_locks: Arc::new(StdMutex::new(HashMap::new())),
            project_activity: ProjectActivityTracker::default(),
            idle_stop_minutes: configured_idle_stop_minutes(),
        }
    }

    #[cfg(test)]
    pub(super) fn with_runtimes(
        store: Store,
        runtime: DockerRuntime,
        qemu: QemuRuntime,
        vpcs: VpcsRuntime,
        dynamips: DynamipsRuntime,
        marketplace: MarketplaceCatalog,
    ) -> Self {
        Self::with_runtimes_and_ubridge(
            store,
            runtime,
            qemu,
            vpcs,
            dynamips,
            UbridgeRuntime::new(),
            marketplace,
        )
    }

    #[cfg(test)]
    pub(super) fn with_runtimes_and_ubridge(
        store: Store,
        runtime: DockerRuntime,
        qemu: QemuRuntime,
        vpcs: VpcsRuntime,
        dynamips: DynamipsRuntime,
        ubridge: UbridgeRuntime,
        marketplace: MarketplaceCatalog,
    ) -> Self {
        Self {
            store,
            runtime,
            qemu,
            vpcs,
            dynamips,
            builtin: BuiltinRuntime::default(),
            ubridge,
            marketplace,
            bind_address: SocketAddr::from(([127, 0, 0, 1], 3080)),
            started_at: Instant::now(),
            project_operation_locks: Arc::new(StdMutex::new(HashMap::new())),
            project_activity: ProjectActivityTracker::default(),
            idle_stop_minutes: configured_idle_stop_minutes(),
        }
    }

    pub fn with_bind_address(mut self, bind_address: SocketAddr) -> Self {
        self.bind_address = bind_address;
        self
    }

    pub(super) async fn lock_project_operations(&self, project_id: &str) -> OwnedMutexGuard<()> {
        let lock = self
            .project_operation_locks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(project_id.to_owned())
            .or_insert_with(|| Arc::new(AsyncMutex::new(())))
            .clone();
        lock.lock_owned().await
    }

    /// Reattach links for runtimes that survived a controller restart. Nodes
    /// that were stopped with the controller are attached later by their
    /// normal start lifecycle path.
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

    pub(super) fn start_idle_stop_worker(&self) {
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

    async fn stop_idle_projects(&self, idle_for: Duration) {
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

    async fn stop_idle_project(&self, project_id: &str, idle_for: Duration) {
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
                        let mut tx = self.store.pool().begin().await?;
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

    pub async fn reattach_running_links(&self) {
        let rows = match sqlx::query(
            "SELECT project_id, object_json FROM topology_objects WHERE kind = 'link'",
        )
        .fetch_all(self.store.pool())
        .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::error!(%error, "could not load links for runtime reconciliation");
                return;
            }
        };
        for row in rows {
            let project_id = row.get::<String, _>("project_id");
            let link = match decode_json(row.get::<String, _>("object_json")) {
                Ok(link) => link,
                Err(error) => {
                    tracing::warn!(error = %error.message, %project_id, "skipping invalid link during runtime reconciliation");
                    continue;
                }
            };
            let _project_lock = self.lock_project_operations(&project_id).await;
            if let Err(error) = LinkEngine::new(self.ubridge.clone())
                .claim_link_ports(&link)
                .await
            {
                tracing::warn!(error = error.message(), %project_id, link_id = %link["link_id"], "could not reserve persisted UDP ports during runtime reconciliation");
                continue;
            }
            let endpoints = match link_runtime_endpoints(self.store.pool(), &project_id, &link)
                .await
            {
                Ok(endpoints) => endpoints,
                Err(error) => {
                    tracing::warn!(error = %error.message, %project_id, link_id = %link["link_id"], "could not resolve link endpoints during runtime reconciliation");
                    continue;
                }
            };
            for endpoint in link["nodes"].as_array().into_iter().flatten() {
                let node_id = endpoint["node_id"].as_str().unwrap_or_default();
                let Some(runtime_endpoint) = endpoints
                    .iter()
                    .find(|candidate| candidate["node_id"].as_str() == Some(node_id))
                else {
                    continue;
                };
                match runtime_endpoint_status(self, &project_id, runtime_endpoint).await {
                    Ok(status) if status != "stopped" => {
                        if let Err(error) = attach_runtime_link_endpoint(
                            self,
                            &project_id,
                            &link,
                            endpoint,
                            runtime_endpoint,
                        )
                        .await
                        {
                            tracing::warn!(error = %error.message, %project_id, %node_id, link_id = %link["link_id"], "could not reattach link endpoint after controller restart");
                        }
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::warn!(error = %error.message, %project_id, %node_id, "could not inspect node during link reconciliation")
                    }
                }
            }
        }
    }
}
