use super::*;

impl DockerRuntime {
    pub async fn info(&self) -> RuntimeInfo {
        match self
            .command(["info", "--format", "{{.ServerVersion}}"], DOCKER_TIMEOUT)
            .await
        {
            Ok(version) => RuntimeInfo {
                available: true,
                version: Some(version.trim().to_owned()),
                error: None,
            },
            Err(error) => RuntimeInfo {
                available: false,
                version: None,
                error: Some(error.message().to_owned()),
            },
        }
    }

    pub async fn templates(&self) -> Result<Vec<Value>, RuntimeError> {
        let output = self
            .command(
                [
                    "image",
                    "ls",
                    "--digests",
                    "--no-trunc",
                    "--format",
                    "{{json .}}",
                ],
                DOCKER_TIMEOUT,
            )
            .await?;
        let mut templates = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for line in output.lines().filter(|line| !line.trim().is_empty()) {
            let image: ImageRow = serde_json::from_str(line).map_err(|error| {
                RuntimeError::Failed(format!("Docker returned an invalid image row: {error}"))
            })?;
            if image.repository == "<none>" || image.tag == "<none>" || image.id.is_empty() {
                continue;
            }
            let reference = format!("{}:{}", image.repository, image.tag);
            if !seen.insert(reference.clone()) {
                continue;
            }
            let template_id = Uuid::new_v5(&Uuid::NAMESPACE_URL, reference.as_bytes());
            templates.push(serde_json::json!({
                "template_id": template_id,
                "name": reference,
                "template_type": "docker",
                "compute_id": "local",
                "category": "guest",
                "symbol": "docker_guest",
                "builtin": false,
                "image": reference,
                "image_digest": image.id,
                "start_command": "",
                "environment": "",
                "extra_hosts": "",
                "extra_volumes": [],
                "console_type": "telnet",
                "console_resolution": "1024x768",
                "console_http_port": 80,
                "console_http_path": "/",
                "aux_type": "none",
                "custom_adapters": [],
                "adapters": 1,
                "memory": 0,
                "cpus": 0.0
            }));
        }
        templates.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
        Ok(templates)
    }

    pub async fn lifecycle(
        &self,
        container_id: &str,
        action: NodeAction,
    ) -> Result<String, RuntimeError> {
        let status = self.container_status(container_id).await?;
        match action {
            NodeAction::Start if status == "suspended" => {
                self.command(["unpause", container_id], DOCKER_TIMEOUT)
                    .await?;
            }
            NodeAction::Start if status != "started" => {
                self.command(["start", container_id], DOCKER_TIMEOUT)
                    .await?;
            }
            NodeAction::Start => {}
            NodeAction::Stop if status == "started" || status == "suspended" => {
                if status == "suspended" {
                    self.command(["unpause", container_id], DOCKER_TIMEOUT)
                        .await?;
                }
                self.command(["stop", "--time", "10", container_id], DOCKER_TIMEOUT)
                    .await?;
            }
            NodeAction::Stop => {}
            NodeAction::Suspend if status == "started" => {
                self.command(["pause", container_id], DOCKER_TIMEOUT)
                    .await?;
            }
            NodeAction::Suspend if status == "suspended" => {}
            NodeAction::Suspend => {
                return Err(RuntimeError::Failed(
                    "Only a running Docker node can be suspended".to_owned(),
                ));
            }
            NodeAction::Reload => {
                self.command(["restart", "--time", "10", container_id], DOCKER_TIMEOUT)
                    .await?;
            }
        }
        let status = self.container_status(container_id).await?;
        if status == "started" {
            self.start_http_console(container_id).await?;
        } else {
            self.stop_http_console(container_id).await;
        }
        Ok(status)
    }

    pub async fn container_status(&self, container_id: &str) -> Result<String, RuntimeError> {
        self.ensure_managed_container(container_id)?;
        let status = self
            .command(
                ["inspect", "--format", "{{.State.Status}}", container_id],
                DOCKER_TIMEOUT,
            )
            .await?;
        Ok(normalize_docker_state(status.trim()))
    }

    pub async fn container_pid(&self, container_id: &str) -> Result<u32, RuntimeError> {
        self.ensure_managed_container(container_id)?;
        let output = self
            .command(
                ["inspect", "--format", "{{.State.Pid}}", container_id],
                DOCKER_TIMEOUT,
            )
            .await?;
        output.trim().parse::<u32>().map_err(|_| {
            RuntimeError::Failed(format!(
                "Docker returned an invalid network namespace PID for container {container_id}"
            ))
        })
    }

    /// Resize the TTY attached to a running Docker container (height, width).
    /// The console WebSocket route should call this when it receives a terminal
    /// resize control frame.
    pub async fn project_node_states(
        &self,
        project_id: &str,
    ) -> Result<HashMap<String, String>, RuntimeError> {
        let output = self
            .command(
                [
                    "ps",
                    "--all",
                    "--filter",
                    &format!("label=network-simulator.project-id={project_id}"),
                    "--format",
                    "{{.Label \"network-simulator.node-id\"}} {{.State}}",
                ],
                DOCKER_TIMEOUT,
            )
            .await?;
        Ok(output
            .lines()
            .filter_map(|line| {
                let (node_id, state) = line.trim().split_once(' ')?;
                if node_id.is_empty() {
                    return None;
                }
                Some((node_id.to_owned(), normalize_docker_state(state)))
            })
            .collect())
    }
}
