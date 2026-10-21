use super::*;
use crate::ubridge::bridge_forwarding;

impl UbridgeRuntime {
    /// Attach one endpoint of a point-to-point link to a runtime adapter.
    /// Each endpoint owns a uBridge bridge joining the runtime NIO to the
    /// reciprocal link UDP tunnel NIO. Docker adapters use an Ethernet NIO;
    /// the emulators use a local UDP NIO.
    pub async fn attach_link_endpoint(
        &self,
        endpoint_id: &str,
        link_local_port: u16,
        peer_link_port: u16,
        runtime_local_port: Option<u16>,
        runtime_peer_port: Option<u16>,
        ethernet_interface: Option<&str>,
        filters: &serde_json::Value,
        suspended: bool,
        capture_file: Option<&str>,
    ) -> Result<(), RuntimeError> {
        self.attach_link_endpoint_with_peer(
            endpoint_id,
            link_local_port,
            peer_link_port,
            runtime_local_port,
            runtime_peer_port,
            ethernet_interface,
            None,
            None,
            filters,
            suspended,
            capture_file,
        )
        .await
    }

    /// Attach an endpoint to either a runtime UDP NIO or a host-side Cloud
    /// NIO. Cloud ports use Ethernet, TAP, or a separately configured UDP
    /// peer; ordinary VM ports keep the reciprocal runtime UDP tunnel.
    pub async fn attach_link_endpoint_with_peer(
        &self,
        endpoint_id: &str,
        link_local_port: u16,
        peer_link_port: u16,
        runtime_local_port: Option<u16>,
        runtime_peer_port: Option<u16>,
        ethernet_interface: Option<&str>,
        tap_interface: Option<&str>,
        external_udp: Option<(u16, &str, u16)>,
        filters: &serde_json::Value,
        suspended: bool,
        capture_file: Option<&str>,
    ) -> Result<(), RuntimeError> {
        self.attach_link_endpoint_internal(
            endpoint_id,
            link_local_port,
            peer_link_port,
            runtime_local_port,
            runtime_peer_port,
            ethernet_interface,
            tap_interface,
            external_udp,
            None,
            filters,
            suspended,
            capture_file,
        )
        .await
    }

    /// Attach a host Ethernet interface through Linux's appropriate NIO.
    /// Physical interfaces use uBridge raw Ethernet; bridge interfaces use a
    /// temporary TAP enslaved to the selected bridge, matching Cloud/NAT.
    pub async fn attach_link_endpoint_with_host_ethernet(
        &self,
        endpoint_id: &str,
        link_local_port: u16,
        peer_link_port: u16,
        host_interface: &str,
        tap_interface: &str,
        filters: &serde_json::Value,
        suspended: bool,
        capture_file: Option<&str>,
    ) -> Result<(), RuntimeError> {
        self.attach_link_endpoint_internal(
            endpoint_id,
            link_local_port,
            peer_link_port,
            None,
            None,
            None,
            None,
            None,
            Some((host_interface, tap_interface)),
            filters,
            suspended,
            capture_file,
        )
        .await
    }

    async fn attach_link_endpoint_internal(
        &self,
        endpoint_id: &str,
        link_local_port: u16,
        peer_link_port: u16,
        runtime_local_port: Option<u16>,
        runtime_peer_port: Option<u16>,
        ethernet_interface: Option<&str>,
        tap_interface: Option<&str>,
        external_udp: Option<(u16, &str, u16)>,
        host_ethernet: Option<(&str, &str)>,
        filters: &serde_json::Value,
        suspended: bool,
        capture_file: Option<&str>,
    ) -> Result<(), RuntimeError> {
        if capture_file.is_some_and(|path| path.chars().any(char::is_control)) {
            return Err(RuntimeError::Failed(
                "uBridge capture paths cannot contain control characters".to_owned(),
            ));
        }
        let bridge = endpoint_name(endpoint_id)?;
        if self.link_bridges.lock().await.get(endpoint_id) == Some(&bridge) {
            self.update_link_endpoint(endpoint_id, filters, suspended)
                .await?;
            if let Some(path) = capture_file {
                self.start_link_capture(endpoint_id, path).await?;
            }
            return Ok(());
        }

        let mut created = false;
        let mut host_bridge_tap = None;
        let setup = async {
            self.command(&format!("bridge create {bridge}")).await?;
            created = true;
            if let Some((interface, tap)) = host_ethernet {
                validate_interface_name(interface)?;
                validate_interface_name(tap)?;
                if is_linux_bridge_interface(interface) {
                    self.command(&format!("bridge add_nio_tap {bridge} \"{tap}\""))
                        .await?;
                    self.command(&format!("brctl addif \"{interface}\" \"{tap}\""))
                        .await?;
                    host_bridge_tap = Some((interface.to_owned(), tap.to_owned()));
                } else {
                    self.command(&format!(
                        "bridge add_nio_linux_raw {bridge} \"{interface}\""
                    ))
                    .await?;
                }
            } else if let Some(interface) = ethernet_interface {
                validate_interface_name(interface)?;
                self.command(&format!("bridge add_nio_ethernet {bridge} {interface}"))
                    .await?;
            } else if let Some(interface) = tap_interface {
                validate_interface_name(interface)?;
                self.command(&format!("bridge add_nio_tap {bridge} {interface}"))
                    .await?;
            } else if let Some((local_port, remote_host, remote_port)) = external_udp {
                validate_remote_host(remote_host)?;
                if local_port == 0 || remote_port == 0 {
                    return Err(RuntimeError::Failed(
                        "Cloud UDP ports must be between 1 and 65535".to_owned(),
                    ));
                }
                self.command(&format!(
                    "bridge add_nio_udp {bridge} {local_port} {remote_host} {remote_port}"
                ))
                .await?;
            } else {
                let local = runtime_local_port.ok_or_else(|| {
                    RuntimeError::Failed("runtime UDP NIO has no local port".to_owned())
                })?;
                let peer = runtime_peer_port.ok_or_else(|| {
                    RuntimeError::Failed("runtime UDP NIO has no peer port".to_owned())
                })?;
                self.command(&format!(
                    "bridge add_nio_udp {bridge} {local} 127.0.0.1 {peer}"
                ))
                .await?;
            }
            self.command(&format!(
                "bridge add_nio_udp {bridge} {link_local_port} 127.0.0.1 {peer_link_port}"
            ))
            .await?;
            self.command(&format!("bridge start {bridge}")).await?;
            bridge_forwarding::enable_edge_forwarding(
                host_bridge_tap.as_ref().map(|(_, tap)| tap.as_str()),
            )
            .await?;
            self.apply_link_filters(&bridge, filters, suspended).await?;
            if let Some(path) = capture_file {
                self.command(&format!(
                    "bridge start_capture {bridge} \"{}\"",
                    escape_ubridge_arg(path)
                ))
                .await?;
            }
            Ok::<(), RuntimeError>(())
        }
        .await;
        if let Err(error) = setup {
            if let Some((interface, tap)) = host_bridge_tap.as_ref() {
                let _ = self
                    .command(&format!("brctl delif \"{interface}\" \"{tap}\""))
                    .await;
            }
            if created {
                let _ = self.command(&format!("bridge delete {bridge}")).await;
            }
            return Err(error);
        }
        if let Some(host_bridge_tap) = host_bridge_tap {
            self.host_bridge_taps
                .lock()
                .await
                .insert(endpoint_id.to_owned(), host_bridge_tap);
        }
        self.link_bridges
            .lock()
            .await
            .insert(endpoint_id.to_owned(), bridge);
        Ok(())
    }

    pub async fn update_link_endpoint(
        &self,
        endpoint_id: &str,
        filters: &serde_json::Value,
        suspended: bool,
    ) -> Result<(), RuntimeError> {
        let bridge = endpoint_name(endpoint_id)?;
        if !self.link_bridges.lock().await.contains_key(endpoint_id) {
            return Ok(());
        }
        self.apply_link_filters(&bridge, filters, suspended).await
    }

    async fn apply_link_filters(
        &self,
        bridge: &str,
        filters: &serde_json::Value,
        suspended: bool,
    ) -> Result<(), RuntimeError> {
        let mut commands = Vec::new();
        if suspended {
            commands.push(format!(
                "bridge add_packet_filter {bridge} suspended frequency_drop -1"
            ));
        } else if let Some(filters) = filters.as_object() {
            let mut index = 0usize;
            for (kind, values) in filters {
                if !matches!(
                    kind.as_str(),
                    "delay" | "frequency_drop" | "packet_loss" | "corrupt" | "bpf"
                ) {
                    return Err(RuntimeError::Failed(format!(
                        "Unsupported uBridge link filter '{kind}'"
                    )));
                }
                let Some(values) = values.as_array() else {
                    return Err(RuntimeError::Failed(format!(
                        "Link filter '{kind}' must contain an array of values"
                    )));
                };
                validate_filter_values(kind, values)?;
                if kind == "bpf" {
                    let Some(expression) = values.first().and_then(serde_json::Value::as_str)
                    else {
                        return Err(RuntimeError::Failed(
                            "BPF link filter must begin with a text expression".to_owned(),
                        ));
                    };
                    if expression.chars().any(char::is_control) {
                        return Err(RuntimeError::Failed(
                            "BPF link filters cannot contain control characters".to_owned(),
                        ));
                    }
                    let expression = expression.trim();
                    if !expression.is_empty() {
                        commands.push(format!(
                            "bridge add_packet_filter {bridge} filter{index} bpf \"{}\"",
                            escape_ubridge_arg(expression)
                        ));
                    }
                    index += 1;
                } else {
                    let arguments = values
                        .iter()
                        .map(filter_value)
                        .collect::<Result<Vec<_>, _>>()?
                        .join(" ");
                    commands.push(format!(
                        "bridge add_packet_filter {bridge} filter{index} {kind} {arguments}"
                    ));
                    index += 1;
                }
            }
        } else {
            return Err(RuntimeError::Failed(
                "Link filters must be an object".to_owned(),
            ));
        }
        // Validate the complete replacement before clearing the currently
        // working filters. uBridge applies rules one command at a time, so
        // keeping preparation separate prevents invalid updates from losing
        // the active set.
        self.command(&format!("bridge reset_packet_filters {bridge}"))
            .await?;
        for command in commands {
            self.command(&command).await?;
        }
        Ok(())
    }

    pub async fn start_link_capture(
        &self,
        endpoint_id: &str,
        capture_file: &str,
    ) -> Result<(), RuntimeError> {
        if capture_file.chars().any(char::is_control) {
            return Err(RuntimeError::Failed(
                "uBridge capture paths cannot contain control characters".to_owned(),
            ));
        }
        if !self.link_bridges.lock().await.contains_key(endpoint_id) {
            return Err(RuntimeError::Failed(
                "Link capture requires a running endpoint".to_owned(),
            ));
        }
        let bridge = endpoint_name(endpoint_id)?;
        self.command(&format!(
            "bridge start_capture {bridge} \"{}\"",
            escape_ubridge_arg(capture_file)
        ))
        .await?;
        Ok(())
    }

    pub async fn stop_link_capture(&self, endpoint_id: &str) -> Result<(), RuntimeError> {
        if !self.link_bridges.lock().await.contains_key(endpoint_id) {
            return Ok(());
        }
        let bridge = endpoint_name(endpoint_id)?;
        self.command(&format!("bridge stop_capture {bridge}"))
            .await?;
        Ok(())
    }

    pub async fn delete_link_endpoint(&self, endpoint_id: &str) -> Result<(), RuntimeError> {
        if !self.link_bridges.lock().await.contains_key(endpoint_id) {
            return Ok(());
        }
        let bridge = endpoint_name(endpoint_id)?;
        if let Some((interface, tap)) = self.host_bridge_taps.lock().await.get(endpoint_id).cloned()
        {
            let _ = self
                .command(&format!("brctl delif \"{interface}\" \"{tap}\""))
                .await;
        }
        self.command(&format!("bridge delete {bridge}")).await?;
        self.link_bridges.lock().await.remove(endpoint_id);
        self.host_bridge_taps.lock().await.remove(endpoint_id);
        Ok(())
    }

    pub async fn has_link_endpoint(&self, endpoint_id: &str) -> bool {
        self.link_bridges.lock().await.contains_key(endpoint_id)
    }

    pub async fn delete_veth(&self, host: &str) -> Result<(), RuntimeError> {
        validate_interface_name(host)?;
        self.command(&format!("docker delete_veth {host}")).await?;
        Ok(())
    }
}
