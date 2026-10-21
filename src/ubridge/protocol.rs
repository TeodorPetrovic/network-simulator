use super::*;

impl UbridgeRuntime {
    pub async fn version(&self) -> Result<String, RuntimeError> {
        let reply = self.command("hypervisor version").await?;
        Ok(reply
            .lines()
            .find_map(|line| line.strip_prefix("100-"))
            .unwrap_or("unknown")
            .trim()
            .to_owned())
    }

    pub async fn info(&self) -> UbridgeRuntimeInfo {
        match self.version().await {
            Ok(version) => {
                let probe = uuid::Uuid::new_v4().to_string();
                match endpoint_name(&probe) {
                    Ok(bridge) => match self.command(&format!("bridge create {bridge}")).await {
                        Ok(_) => match self.command(&format!("bridge delete {bridge}")).await {
                            Ok(_) => UbridgeRuntimeInfo {
                                available: true,
                                binary: self.binary.display().to_string(),
                                version: Some(version),
                                error: None,
                            },
                            Err(error) => UbridgeRuntimeInfo {
                                available: false,
                                binary: self.binary.display().to_string(),
                                version: Some(version),
                                error: Some(format!(
                                    "Could not remove uBridge capability probe: {}",
                                    error.message()
                                )),
                            },
                        },
                        Err(error) => UbridgeRuntimeInfo {
                            available: false,
                            binary: self.binary.display().to_string(),
                            version: Some(version),
                            error: Some(format!(
                                "Could not create a uBridge NIO relay: {}",
                                error.message()
                            )),
                        },
                    },
                    Err(error) => UbridgeRuntimeInfo {
                        available: false,
                        binary: self.binary.display().to_string(),
                        version: Some(version),
                        error: Some(error.message().to_owned()),
                    },
                }
            }
            Err(error) => UbridgeRuntimeInfo {
                available: false,
                binary: self.binary.display().to_string(),
                version: None,
                error: Some(error.message().to_owned()),
            },
        }
    }

    pub async fn create_veth_pair(&self, host: &str, guest: &str) -> Result<(), RuntimeError> {
        validate_interface_name(host)?;
        validate_interface_name(guest)?;
        self.command(&format!("docker create_veth {host} {guest}"))
            .await?;
        Ok(())
    }

    pub async fn move_veth_to_container(
        &self,
        guest_if: &str,
        container_pid: u32,
        guest_name: &str,
    ) -> Result<(), RuntimeError> {
        validate_interface_name(guest_if)?;
        validate_interface_name(guest_name)?;
        if container_pid == 0 {
            return Err(RuntimeError::Failed(
                "Docker did not report a running container network namespace".to_owned(),
            ));
        }
        self.command(&format!(
            "docker move_to_ns {guest_if} {container_pid} {guest_name}"
        ))
        .await?;
        Ok(())
    }

    pub async fn set_docker_mac_address(
        &self,
        interface: &str,
        mac_address: &str,
    ) -> Result<(), RuntimeError> {
        validate_interface_name(interface)?;
        let octets = mac_address.split(':').collect::<Vec<_>>();
        if octets.len() != 6
            || octets.iter().any(|octet| {
                octet.len() != 2 || !octet.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
            return Err(RuntimeError::Failed(
                "Docker interface MAC address must contain six hexadecimal octets".to_owned(),
            ));
        }
        self.command(&format!("docker set_mac_addr {interface} {mac_address}"))
            .await?;
        Ok(())
    }

    pub async fn set_interface_up(&self, interface: &str) -> Result<(), RuntimeError> {
        validate_interface_name(interface)?;
        self.command(&format!("link set {interface} up")).await?;
        Ok(())
    }

    pub async fn reserve_udp_port(&self) -> Result<u16, RuntimeError> {
        self.reserve_udp_port_for(&uuid::Uuid::new_v4().to_string())
            .await
    }

    pub async fn reserve_udp_port_for(&self, owner: &str) -> Result<u16, RuntimeError> {
        let mut allocated = self.allocated_udp_ports.lock().await;
        for _ in 0..128 {
            let port = available_udp_port().await?;
            if !allocated.contains_key(&port) {
                allocated.insert(port, owner.to_owned());
                return Ok(port);
            }
        }
        Err(RuntimeError::Unavailable(
            "Could not reserve a unique UDP port".to_owned(),
        ))
    }

    /// Claim a persisted port during startup reconciliation so new links do
    /// not reuse ports that belong to links created by an earlier process.
    pub async fn claim_udp_port(&self, owner: &str, port: u16) -> Result<(), RuntimeError> {
        if port == 0 {
            return Err(RuntimeError::Failed(
                "Cannot reserve UDP port zero".to_owned(),
            ));
        }
        let mut allocated = self.allocated_udp_ports.lock().await;
        match allocated.get(&port) {
            Some(current) if current == owner => Ok(()),
            Some(_) => Err(RuntimeError::Failed(format!(
                "UDP port {port} is already reserved by another link"
            ))),
            None => {
                allocated.insert(port, owner.to_owned());
                Ok(())
            }
        }
    }

    pub async fn release_udp_ports(&self, owner: &str, ports: impl IntoIterator<Item = u16>) {
        let mut allocated = self.allocated_udp_ports.lock().await;
        for port in ports {
            if allocated.get(&port).is_some_and(|current| current == owner) {
                allocated.remove(&port);
            }
        }
    }
}
