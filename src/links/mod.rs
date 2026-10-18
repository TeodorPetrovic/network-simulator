use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    dynamips::DynamipsRuntime,
    qemu::QemuRuntime,
    runtime::{DockerRuntime, RuntimeError},
    ubridge::UbridgeRuntime,
    vpcs::VpcsRuntime,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdapterNio {
    Ethernet {
        host_interface: String,
        guest_interface: String,
    },
    /// Linux Cloud/NAT host interfaces use raw Ethernet, or a TAP enslaved to
    /// a Linux bridge when the selected interface is itself a bridge.
    HostEthernet {
        interface: String,
        tap_interface: String,
    },
    Tap {
        interface: String,
    },
    ExternalUdp {
        local_port: u16,
        remote_host: String,
        remote_port: u16,
    },
    Udp {
        adapter_port: u16,
        runtime_port: u16,
    },
}

/// Runtime-independent adapter description used by the link engine. Runtime
/// implementations translate a topology endpoint into the NIO their guest
/// can consume; all links then share the same reciprocal UDP tunnel format.
pub trait NioAdapter {
    fn adapter_nio(&self, endpoint: &Value) -> Result<AdapterNio, RuntimeError>;
}

impl NioAdapter for DockerRuntime {
    fn adapter_nio(&self, endpoint: &Value) -> Result<AdapterNio, RuntimeError> {
        let endpoint_id = required_text(endpoint, "runtime_endpoint_id")?;
        let port = endpoint["port_number"].as_u64().unwrap_or_default();
        Ok(AdapterNio::Ethernet {
            host_interface: crate::ubridge::host_veth_name(&endpoint_id)?,
            guest_interface: format!("eth{port}"),
        })
    }
}

impl NioAdapter for QemuRuntime {
    fn adapter_nio(&self, endpoint: &Value) -> Result<AdapterNio, RuntimeError> {
        udp_adapter_nio(endpoint)
    }
}

impl NioAdapter for VpcsRuntime {
    fn adapter_nio(&self, endpoint: &Value) -> Result<AdapterNio, RuntimeError> {
        udp_adapter_nio(endpoint)
    }
}

impl NioAdapter for DynamipsRuntime {
    fn adapter_nio(&self, endpoint: &Value) -> Result<AdapterNio, RuntimeError> {
        udp_adapter_nio(endpoint)
    }
}

fn udp_adapter_nio(endpoint: &Value) -> Result<AdapterNio, RuntimeError> {
    Ok(AdapterNio::Udp {
        adapter_port: required_port(endpoint, "runtime_adapter_port")?,
        runtime_port: required_port(endpoint, "runtime_udp_port")?,
    })
}

fn required_text(endpoint: &Value, key: &str) -> Result<String, RuntimeError> {
    endpoint
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| RuntimeError::Failed(format!("Link endpoint is missing {key}")))
}

fn required_port(endpoint: &Value, key: &str) -> Result<u16, RuntimeError> {
    endpoint
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port != 0)
        .ok_or_else(|| RuntimeError::Failed(format!("Link endpoint is missing valid {key}")))
}

#[derive(Clone)]
pub struct LinkEngine {
    ubridge: UbridgeRuntime,
}

impl LinkEngine {
    pub fn new(ubridge: UbridgeRuntime) -> Self {
        Self { ubridge }
    }

    pub async fn reserve_pair(&self, link: &mut Value) -> Result<(), RuntimeError> {
        let link_id = required_text(link, "link_id")?;
        let endpoints = link
            .get_mut("nodes")
            .and_then(Value::as_array_mut)
            .filter(|endpoints| endpoints.len() == 2)
            .ok_or_else(|| {
                RuntimeError::Failed("Ethernet links require exactly two endpoints".to_owned())
            })?;
        let mut ports = Vec::with_capacity(2);
        for _ in 0..2 {
            ports.push(self.ubridge.reserve_udp_port().await?);
        }
        for (index, endpoint) in endpoints.iter_mut().enumerate() {
            let endpoint_object = endpoint.as_object_mut().ok_or_else(|| {
                RuntimeError::Failed("Link endpoints must be JSON objects".to_owned())
            })?;
            let node_id = endpoint_object
                .get("node_id")
                .and_then(Value::as_str)
                .ok_or_else(|| RuntimeError::Failed("Link endpoint has no node_id".to_owned()))?;
            let adapter = endpoint_object
                .get("adapter_number")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            let port = endpoint_object
                .get("port_number")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            let endpoint_key = format!("{link_id}/{node_id}/{adapter}/{port}");
            let endpoint_id =
                Uuid::new_v5(&Uuid::NAMESPACE_URL, endpoint_key.as_bytes()).to_string();
            endpoint_object.insert("runtime_endpoint_id".into(), json!(endpoint_id));
            endpoint_object.insert("runtime_link_port".into(), json!(ports[index]));
            endpoint_object.insert("runtime_peer_link_port".into(), json!(ports[1 - index]));
            if endpoint_object.get("node_type").and_then(Value::as_str) != Some("docker") {
                let adapter_port = self.ubridge.reserve_udp_port().await?;
                let runtime_port = self.ubridge.reserve_udp_port().await?;
                endpoint_object.insert("runtime_adapter_port".into(), json!(adapter_port));
                endpoint_object.insert("runtime_udp_port".into(), json!(runtime_port));
            }
        }
        Ok(())
    }

    pub async fn attach_endpoint(
        &self,
        endpoint: &Value,
        adapter: AdapterNio,
        filters: &Value,
        suspended: bool,
        capture_file: Option<&str>,
    ) -> Result<(), RuntimeError> {
        let endpoint_id = required_text(endpoint, "runtime_endpoint_id")?;
        let link_port = required_port(endpoint, "runtime_link_port")?;
        let peer_link_port = required_port(endpoint, "runtime_peer_link_port")?;
        let (runtime_adapter_port, runtime_port, ethernet, tap, external_udp) = match adapter {
            AdapterNio::Ethernet { host_interface, .. } => {
                (None, None, Some(host_interface), None, None)
            }
            AdapterNio::HostEthernet {
                interface,
                tap_interface,
            } => {
                self.ubridge
                    .attach_link_endpoint_with_host_ethernet(
                        &endpoint_id,
                        link_port,
                        peer_link_port,
                        &interface,
                        &tap_interface,
                        filters,
                        suspended,
                        capture_file,
                    )
                    .await?;
                return Ok(());
            }
            AdapterNio::Tap { interface } => (None, None, None, Some(interface), None),
            AdapterNio::ExternalUdp {
                local_port,
                remote_host,
                remote_port,
            } => (
                None,
                None,
                None,
                None,
                Some((local_port, remote_host, remote_port)),
            ),
            AdapterNio::Udp {
                adapter_port,
                runtime_port,
            } => (Some(adapter_port), Some(runtime_port), None, None, None),
        };
        self.ubridge
            .attach_link_endpoint_with_peer(
                &endpoint_id,
                link_port,
                peer_link_port,
                runtime_adapter_port,
                runtime_port,
                ethernet.as_deref(),
                tap.as_deref(),
                external_udp
                    .as_ref()
                    .map(|(port, host, remote)| (*port, host.as_str(), *remote)),
                filters,
                suspended,
                capture_file,
            )
            .await
    }

    pub async fn update_endpoint(
        &self,
        endpoint: &Value,
        filters: &Value,
        suspended: bool,
    ) -> Result<(), RuntimeError> {
        let endpoint_id = required_text(endpoint, "runtime_endpoint_id")?;
        self.ubridge
            .update_link_endpoint(&endpoint_id, filters, suspended)
            .await
    }

    pub async fn start_capture(
        &self,
        endpoint: &Value,
        capture_file: &str,
    ) -> Result<(), RuntimeError> {
        let endpoint_id = required_text(endpoint, "runtime_endpoint_id")?;
        self.ubridge
            .start_link_capture(&endpoint_id, capture_file)
            .await
    }

    pub async fn stop_capture(&self, endpoint: &Value) -> Result<(), RuntimeError> {
        let endpoint_id = required_text(endpoint, "runtime_endpoint_id")?;
        self.ubridge.stop_link_capture(&endpoint_id).await
    }

    pub async fn detach_endpoint(&self, endpoint: &Value) -> Result<(), RuntimeError> {
        let endpoint_id = required_text(endpoint, "runtime_endpoint_id")?;
        self.ubridge.delete_link_endpoint(&endpoint_id).await
    }
}
