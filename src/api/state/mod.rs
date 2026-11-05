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
            auto_close_delay: configured_auto_close_delay(),
            web_wireshark: Arc::new(crate::web_wireshark::WebWiresharkManager::default()),
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
            auto_close_delay: configured_auto_close_delay(),
            web_wireshark: Arc::new(crate::web_wireshark::WebWiresharkManager::default()),
        }
    }

    pub fn with_bind_address(mut self, bind_address: SocketAddr) -> Self {
        self.bind_address = bind_address;
        self
    }

    pub(super) async fn auto_close_after_disconnect(&self, project_id: &str) {
        sleep(self.auto_close_delay).await;
        if self.project_activity.has_open_websockets(project_id) {
            return;
        }
        let project = match fetch_project(self.store.pool(), project_id).await {
            Ok(project) => project,
            Err(error) => {
                tracing::debug!(error = %error.message, %project_id, "project disappeared before auto-close");
                return;
            }
        };
        if !project["auto_close"].as_bool().unwrap_or(false)
            || project["status"].as_str() != Some("opened")
        {
            return;
        }
        let Ok(project_uuid) = Uuid::parse_str(project_id) else {
            return;
        };
        if let Err(error) = projects::set_project_status(self.clone(), project_uuid, "closed").await
        {
            tracing::error!(error = %error.message, %project_id, "could not auto-close project after notifications disconnected");
        }
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

}

mod lifecycle;
mod recovery;
pub(crate) use recovery::*;
pub(crate) use lifecycle::*;
