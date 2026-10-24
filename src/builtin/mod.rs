//! Builtin Ethernet devices that plug into the point-to-point link engine.
//!
//! Ethernet switches and hubs use one local UDP NIO per configured port. A
//! node-scoped forwarding task joins those NIOs, while cloud and NAT nodes
//! attach their selected host NIO directly to the uBridge endpoint.

use std::{
    collections::{HashMap, HashSet},
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
    time::{Duration, Instant},
};

use serde_json::{Value, json};
use tokio::{net::UdpSocket, sync::Mutex};
use uuid::Uuid;

use crate::{links::AdapterNio, runtime::RuntimeError};

const DEFAULT_PORTS: usize = 8;
const MAX_PORTS: usize = 256;
const MAX_FDB_ENTRIES: usize = 8192;
const FDB_AGE: Duration = Duration::from_secs(300);
const FDB_AGE_INTERVAL: Duration = Duration::from_secs(60);

pub use types::BuiltinRuntime;
use types::{ActivePort, ForwardingNode, LearnedAddress, RuntimePort, SwitchPortMode};

mod cloud;
mod switch;
#[path = "types.rs"]
mod types;
#[cfg(test)]
use cloud::validate_host_interface_policy;
pub use cloud::validate_host_interfaces;
use cloud::{
    default_nat_interface, host_ethernet_tap_name, normalize_cloud_console, normalize_cloud_ports,
    normalize_host_interface_allowlist, normalize_hub_ports, normalize_nat_ports,
    normalize_switch_ports, port_from_endpoint, port_from_mapping, required_mapping_text,
    safe_remote_host,
};
#[cfg(test)]
use switch::{
    age_forwarding_node, classify_egress, classify_ingress, forward_frame, learn_address,
    switch_mode, vlan_allowed,
};

impl BuiltinRuntime {
    pub fn validate_host_interfaces(node: &Value) -> Result<(), String> {
        crate::builtin::validate_host_interfaces(node)
    }

    pub fn templates() -> Vec<Value> {
        let switch_id = builtin_template_id("ethernet_switch");
        let hub_id = builtin_template_id("ethernet_hub");
        let cloud_id = builtin_template_id("cloud");
        let nat_id = builtin_template_id("nat");
        vec![
            json!({
                "template_id": switch_id,
                "name": "Ethernet switch",
                "template_type": "ethernet_switch",
                "compute_id": "local",
                "category": "switch",
                "default_name_format": "Switch{0}",
                "symbol": "ethernet_switch",
                "builtin": true,
                "console_type": "none",
                "ports_mapping": default_switch_ports()
            }),
            json!({
                "template_id": hub_id,
                "name": "Ethernet hub",
                "template_type": "ethernet_hub",
                "compute_id": "local",
                "category": "switch",
                "default_name_format": "Hub{0}",
                "symbol": "hub",
                "builtin": true,
                "ports_mapping": default_hub_ports()
            }),
            json!({
                "template_id": cloud_id,
                "name": "Cloud",
                "template_type": "cloud",
                "compute_id": "local",
                "category": "guest",
                "default_name_format": "Cloud{0}",
                "symbol": "cloud",
                "builtin": true,
                "ports_mapping": [],
                "remote_console_host": "127.0.0.1",
                "remote_console_port": 23,
                "remote_console_type": "none",
                "remote_console_http_path": "/"
            }),
            json!({
                "template_id": nat_id,
                "name": "NAT",
                "template_type": "nat",
                "compute_id": "local",
                "category": "guest",
                "default_name_format": "NAT{0}",
                "symbol": "nat",
                "builtin": true,
                "ports_mapping": [{
                    "name": "nat0", "port_number": 0, "type": "ethernet",
                    "interface": default_nat_interface()
                }],
                "nat_interface": default_nat_interface(),
                "usage": "Linux NAT requires an active libvirt default network on virbr0."
            }),
        ]
    }

    pub fn is_type(node_type: &str) -> bool {
        matches!(
            node_type,
            "ethernet_switch" | "ethernet_hub" | "cloud" | "nat"
        )
    }

    pub fn normalize_template_ports(node_type: &str, mapping: Value) -> Result<Value, String> {
        let ports = match node_type {
            "ethernet_switch" => normalize_switch_ports(Some(mapping))?,
            "ethernet_hub" => normalize_hub_ports(Some(mapping))?,
            "cloud" => normalize_cloud_ports(Some(mapping))?,
            "nat" => {
                let mut object = serde_json::Map::new();
                normalize_nat_ports(&mut object, Some(mapping))?
            }
            _ => return Err(format!("Unsupported builtin node type '{node_type}'")),
        };
        Ok(json!(ports))
    }

    /// Normalize a builtin node or template-derived node into the persisted
    /// GNS3-shaped representation. Adapter numbering is always adapter 0,
    /// port N for these devices.
    pub fn normalize_node(mut node: Value) -> Result<Value, String> {
        let object = node
            .as_object_mut()
            .ok_or_else(|| "Builtin node body must be a JSON object".to_owned())?;
        let node_type = object
            .get("node_type")
            .and_then(Value::as_str)
            .ok_or_else(|| "node_type is required".to_owned())?
            .to_owned();
        if !Self::is_type(&node_type) {
            return Err(format!("Unsupported builtin node type '{node_type}'"));
        }
        if object
            .get("compute_id")
            .and_then(Value::as_str)
            .is_some_and(|compute_id| compute_id != "local")
        {
            return Err("Builtin nodes can run only on the local compute".to_owned());
        }
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty() && name.len() <= 128)
            .ok_or_else(|| "name must contain 1 to 128 characters".to_owned())?
            .to_owned();
        object.insert("name".into(), json!(name));
        object.insert("node_type".into(), json!(node_type));
        object.insert("compute_id".into(), json!("local"));
        object.insert("status".into(), json!("started"));
        object.entry("x").or_insert(json!(0));
        object.entry("y").or_insert(json!(0));
        object.entry("z").or_insert(json!(1));
        object.entry("locked").or_insert(json!(false));
        object.entry("properties").or_insert(json!({}));

        if matches!(node_type.as_str(), "cloud" | "nat") {
            let allowed = normalize_host_interface_allowlist(object.get("allow_host_interfaces"))?;
            object.insert("allow_host_interfaces".into(), json!(allowed));
        }

        let mapping = object.get("ports_mapping").cloned();
        let ports_mapping = match node_type.as_str() {
            "ethernet_switch" => normalize_switch_ports(mapping)?,
            "ethernet_hub" => normalize_hub_ports(mapping)?,
            "cloud" => normalize_cloud_ports(mapping)?,
            "nat" => normalize_nat_ports(object, mapping)?,
            _ => unreachable!(),
        };
        object.insert("ports_count".into(), json!(ports_mapping.len()));
        object.insert("ports_mapping".into(), json!(ports_mapping));
        if node_type == "ethernet_switch" {
            object.insert(
                "console_type".into(),
                json!(
                    object
                        .get("console_type")
                        .and_then(Value::as_str)
                        .unwrap_or("none")
                ),
            );
        }
        if node_type == "cloud" {
            normalize_cloud_console(object)?;
        }
        if node_type == "nat" {
            let interface = object["ports_mapping"][0]["interface"].clone();
            object.insert("nat_interface".into(), interface);
        }
        Ok(node)
    }

    /// Normalize the flat per-node configuration contract. `properties` is
    /// accepted as a legacy input wrapper for clients that used the GNS3
    /// compute-node shape; normalized values are always stored at top level.
    pub fn normalize_configuration_patch(node: &Value, patch: Value) -> Result<Value, String> {
        let input = patch
            .as_object()
            .ok_or_else(|| "device configuration must be an object".to_owned())?;
        if input.is_empty() {
            return Err("device configuration is empty".to_owned());
        }
        let node_type = node["node_type"]
            .as_str()
            .ok_or_else(|| "node_type is required".to_owned())?;
        let allowed: &[&str] = match node_type {
            "ethernet_switch" => &["name", "usage", "ports_mapping", "console_type"],
            "ethernet_hub" => &["name", "usage", "ports_mapping"],
            "cloud" => &[
                "name",
                "usage",
                "ports_mapping",
                "allow_host_interfaces",
                "remote_console_host",
                "remote_console_port",
                "remote_console_type",
                "remote_console_http_path",
            ],
            "nat" => &["name", "usage", "allow_host_interfaces"],
            _ => return Err(format!("Unsupported builtin node type '{node_type}'")),
        };

        let mut requested = serde_json::Map::new();
        for (key, value) in input {
            if key == "properties" {
                let properties = value
                    .as_object()
                    .ok_or_else(|| "properties must be an object".to_owned())?;
                for (property, value) in properties {
                    if !allowed.contains(&property.as_str()) {
                        return Err(format!(
                            "'{property}' cannot be changed in this {node_type} configuration"
                        ));
                    }
                    requested
                        .entry(property.clone())
                        .or_insert_with(|| value.clone());
                }
                continue;
            }
            if !allowed.contains(&key.as_str()) {
                return Err(format!(
                    "'{key}' cannot be changed in this {node_type} configuration"
                ));
            }
            requested.insert(key.clone(), value.clone());
        }
        if requested.is_empty() {
            return Err("device configuration is empty".to_owned());
        }
        if let Some(console_type) = requested.get("console_type")
            && !matches!(console_type.as_str(), Some("none" | "telnet"))
        {
            return Err("unsupported switch console_type".to_owned());
        }
        if let Some(usage) = requested.get("usage")
            && usage.as_str().is_none_or(|text| text.len() > 65_536)
        {
            return Err("usage must be text under 64 KiB".to_owned());
        }

        let mut candidate = node.clone();
        for (key, value) in &requested {
            candidate[key] = value.clone();
        }
        let normalized = Self::normalize_node(candidate)?;
        let mut result = serde_json::Map::new();
        for key in requested.keys() {
            result.insert(key.clone(), normalized[key].clone());
        }
        if requested.contains_key("ports_mapping") {
            result.insert("ports_count".to_owned(), normalized["ports_count"].clone());
            if node_type == "nat" {
                result.insert(
                    "nat_interface".to_owned(),
                    normalized["nat_interface"].clone(),
                );
            }
        }
        Ok(Value::Object(result))
    }

    /// Resolve the adapter NIO described by a builtin node's port mapping.
    pub fn adapter_nio(node: &Value, endpoint: &Value) -> Result<AdapterNio, RuntimeError> {
        let node_type = node["node_type"].as_str().unwrap_or_default();
        let port_number = endpoint["port_number"].as_u64().unwrap_or_default() as usize;
        let mapping = node["ports_mapping"]
            .as_array()
            .and_then(|ports| ports.get(port_number))
            .ok_or_else(|| {
                RuntimeError::Failed(format!("Port {port_number} does not exist on {node_type}"))
            })?;
        match node_type {
            "ethernet_switch" | "ethernet_hub" => Ok(AdapterNio::Udp {
                adapter_port: port_from_endpoint(endpoint, "runtime_adapter_port")?,
                runtime_port: port_from_endpoint(endpoint, "runtime_udp_port")?,
            }),
            "cloud" | "nat" => match mapping["type"].as_str().unwrap_or_default() {
                "ethernet" => Ok(AdapterNio::HostEthernet {
                    interface: required_mapping_text(mapping, "interface")?,
                    tap_interface: host_ethernet_tap_name(endpoint)?,
                }),
                "tap" => Ok(AdapterNio::Tap {
                    interface: required_mapping_text(mapping, "interface")?,
                }),
                "udp" => Ok(AdapterNio::ExternalUdp {
                    local_port: port_from_mapping(mapping, "lport")?,
                    remote_host: safe_remote_host(mapping)?,
                    remote_port: port_from_mapping(mapping, "rport")?,
                }),
                other => Err(RuntimeError::Failed(format!(
                    "Unsupported builtin port type '{other}'"
                ))),
            },
            other => Err(RuntimeError::Failed(format!(
                "Unsupported builtin node type '{other}'"
            ))),
        }
    }
}

fn builtin_template_id(node_type: &str) -> String {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("network-simulator/builtin/{node_type}").as_bytes(),
    )
    .to_string()
}

fn default_switch_ports() -> Vec<Value> {
    (0..DEFAULT_PORTS)
        .map(|index| json!({"name":format!("Ethernet{index}"),"port_number":index,"type":"access","vlan":1,"ethertype":"0x8100"}))
        .collect()
}

fn default_hub_ports() -> Vec<Value> {
    (0..DEFAULT_PORTS)
        .map(|index| json!({"name":format!("Ethernet{index}"),"port_number":index}))
        .collect()
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
