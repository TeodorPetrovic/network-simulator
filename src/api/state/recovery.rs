use super::*;

impl super::AppState {
    /// Reattach links for runtimes that survived a controller restart. Nodes
    /// that were stopped with the controller are attached later by their
    /// normal start lifecycle path.
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

    /// Reconcile persisted runtime statuses after startup and periodically while
    /// the controller is running. Only runtimes with an inspection hook are
    /// considered; inspection failures leave the stored state untouched.
    /// Restores the persisted per-adapter UDP ports of QEMU links into the
    /// in-memory reservation map after a controller restart.
    pub async fn restore_qemu_adapter_ports(&self) {
        let qemu_nodes: std::collections::HashSet<String> = match sqlx::query_scalar(
            "SELECT object_id FROM topology_objects WHERE kind = 'node' AND json_extract(object_json, '$.node_type') = 'qemu'",
        )
        .fetch_all(self.store.pool())
        .await
        {
            Ok(ids) => ids.into_iter().collect(),
            Err(error) => {
                tracing::error!(%error, "could not load QEMU nodes for adapter port restore");
                return;
            }
        };
        if qemu_nodes.is_empty() {
            return;
        }
        let rows = match sqlx::query(
            "SELECT object_json FROM topology_objects WHERE kind = 'link'",
        )
        .fetch_all(self.store.pool())
        .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::error!(%error, "could not load links for QEMU adapter port restore");
                return;
            }
        };
        for row in rows {
            let Ok(link) = decode_json(row.get::<String, _>("object_json")) else {
                continue;
            };
            for endpoint in link["nodes"].as_array().into_iter().flatten() {
                let Some(node_id) = endpoint["node_id"].as_str() else {
                    continue;
                };
                if !qemu_nodes.contains(node_id) {
                    continue;
                }
                let (Some(adapter), Some(local), Some(remote)) = (
                    endpoint["adapter_number"].as_u64(),
                    endpoint["runtime_udp_port"].as_u64(),
                    endpoint["runtime_adapter_port"].as_u64(),
                ) else {
                    continue;
                };
                let (Ok(adapter), Ok(local), Ok(remote)) = (
                    u32::try_from(adapter),
                    u16::try_from(local),
                    u16::try_from(remote),
                ) else {
                    continue;
                };
                if let Err(error) =
                    self.qemu
                        .restore_adapter_ports(node_id, adapter, (local, remote))
                {
                    tracing::warn!(error = %error.message(), %node_id, adapter, "could not restore QEMU adapter ports");
                }
            }
        }
    }

    /// Clears Web Wireshark link flags that cannot be valid after a controller
    /// restart because the sessions and feeders were in-memory only.
    pub async fn reset_stale_web_wireshark(&self) {
        self.web_wireshark.remove_orphan_containers().await;
        let rows = match sqlx::query(
            "SELECT project_id, object_id, object_json FROM topology_objects
             WHERE kind = 'link' AND json_extract(object_json, '$.wireshark') = 1",
        )
        .fetch_all(self.store.pool())
        .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::error!(%error, "could not load links for Web Wireshark reset");
                return;
            }
        };
        for row in rows {
            let project_id: String = row.get("project_id");
            let link_id: String = row.get("object_id");
            let Ok(mut link) = decode_json(row.get::<String, _>("object_json")) else {
                continue;
            };
            link["wireshark"] = serde_json::json!(false);
            link["wireshark_ws_path"] = serde_json::Value::Null;
            link["ws_path"] = serde_json::Value::Null;
            let result = async {
                let mut tx = self.store.pool().begin().await?;
                sqlx::query(
                    "UPDATE topology_objects SET object_json = ? WHERE project_id = ? AND kind = 'link' AND object_id = ?",
                )
                .bind(link.to_string())
                .bind(&project_id)
                .bind(&link_id)
                .execute(&mut *tx)
                .await?;
                Store::append_event(&mut tx, &project_id, "link.updated", &link).await?;
                tx.commit().await
            }
            .await;
            if let Err(error) = result {
                tracing::warn!(%error, %project_id, %link_id, "could not reset stale Web Wireshark flag");
            }
        }
    }

    pub async fn reconcile_node_statuses(&self) {
        let rows = match sqlx::query(
            "SELECT project_id, object_id, object_json FROM topology_objects
             WHERE kind = 'node' AND json_extract(object_json, '$.status') != 'stopped'",
        )
        .fetch_all(self.store.pool())
        .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::error!(%error, "could not load nodes for runtime status reconciliation");
                return;
            }
        };
        for row in rows {
            let project_id: String = row.get("project_id");
            let node_id: String = row.get("object_id");
            let mut node = match decode_json(row.get::<String, _>("object_json")) {
                Ok(node) => node,
                Err(error) => {
                    tracing::warn!(error = %error.message, %project_id, %node_id, "skipping invalid node during runtime status reconciliation");
                    continue;
                }
            };
            let status = match inspected_node_status(self, &node).await {
                Some(Ok(status)) => status,
                Some(Err(error)) => {
                    tracing::debug!(error = %error.message(), %project_id, %node_id, "runtime status inspection failed");
                    continue;
                }
                None => continue,
            };
            if status == "started" || status == "suspended" {
                continue;
            }
            if node["status"].as_str() == Some("stopped") {
                continue;
            }
            node["status"] = json!("stopped");
            let result = async {
                let mut tx = self.store.begin_write().await?;
                sqlx::query("UPDATE topology_objects SET object_json = ? WHERE project_id = ? AND kind = 'node' AND object_id = ?")
                    .bind(serde_json::to_string(&node).map_err(internal_json_error)?).bind(&project_id).bind(&node_id).execute(&mut *tx).await?;
                let event = Store::append_event(&mut tx, &project_id, "node.updated", &node).await?;
                tx.commit().await?;
                Ok::<_, ApiError>(event)
            }.await;
            match result {
                Ok(event) => self.store.publish(event),
                Err(error) => {
                    tracing::error!(error = %error.message, %project_id, %node_id, "could not persist reconciled node status")
                }
            }
        }
    }

    pub async fn monitor_node_liveness(&self) {
        loop {
            sleep(Duration::from_secs(5)).await;
            self.reconcile_node_statuses().await;
        }
    }
}


pub(crate) async fn inspected_node_status(
    state: &AppState,
    node: &Value,
) -> Option<Result<String, RuntimeError>> {
    let id = node["node_id"].as_str().unwrap_or_default();
    match node["node_type"].as_str()? {
        "docker" => Some(
            state
                .runtime
                .container_status(node["container_id"].as_str()?)
                .await,
        ),
        "qemu" => Some(match state.qemu.vm_from_node(node) {
            Ok(vm) => state.qemu.status(&vm).await,
            Err(error) => return Some(Err(error)),
        }),
        "vpcs" => Some(state.vpcs.status(id).await),
        "dynamips" => Some(match state.dynamips.vm_from_node(node) {
            Ok(vm) => state.dynamips.status(&vm).await,
            Err(error) => return Some(Err(error)),
        }),
        _ => None,
    }
}

pub(crate) fn configured_auto_close_delay() -> Duration {
    let millis = std::env::var("NETWORK_SIMULATOR_AUTO_CLOSE_DELAY_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|millis| *millis <= 60_000)
        .unwrap_or(1_000);
    Duration::from_millis(millis)
}
