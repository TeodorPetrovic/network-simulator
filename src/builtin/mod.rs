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

#[path = "types.rs"]
mod types;
mod switch;
use switch::switch_mode;
#[cfg(test)]
use switch::{age_forwarding_node, classify_egress, classify_ingress, forward_frame, learn_address, vlan_allowed};

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

fn default_nat_interface() -> String {
    std::env::var("NETWORK_SIMULATOR_NAT_INTERFACE").unwrap_or_else(|_| "virbr0".to_owned())
}

fn default_host_interfaces() -> Vec<Value> {
    let uplinks = default_route_interfaces();
    let Ok(entries) = std::fs::read_dir("/sys/class/net") else {
        return Vec::new();
    };
    let mut interfaces = entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| {
            name != "lo"
                && !name.starts_with("docker")
                && !name.starts_with("br-")
                && !name.starts_with("veth")
                && !name.starts_with("ns")
                && !name.starts_with("gns3")
                && !uplinks.contains(name)
        })
        .collect::<Vec<_>>();
    interfaces.sort();
    interfaces
        .into_iter()
        .enumerate()
        .map(|(index, interface)| {
            json!({"name":format!("Ethernet{index}"),"port_number":index,"type":"ethernet","interface":interface})
        })
        .collect()
}

fn normalize_switch_ports(mapping: Option<Value>) -> Result<Vec<Value>, String> {
    let ports = mapping
        .as_ref()
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(default_switch_ports);
    validate_port_count(&ports)?;
    ports
        .into_iter()
        .enumerate()
        .map(|(index, mut port)| {
            let object = port
                .as_object_mut()
                .ok_or_else(|| "Each switch port must be an object".to_owned())?;
            let kind = object
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("access")
                .to_owned();
            if !matches!(kind.as_str(), "access" | "dot1q" | "qinq") {
                return Err(format!("Unsupported switch port type '{kind}'"));
            }
            let vlan = object.get("vlan").and_then(Value::as_u64).unwrap_or(1);
            if !(1..=4094).contains(&vlan) {
                return Err("Switch VLAN must be from 1 to 4094".to_owned());
            }
            let ethertype = object
                .get("ethertype")
                .and_then(Value::as_str)
                .unwrap_or("0x8100")
                .to_ascii_lowercase();
            if !matches!(
                ethertype.as_str(),
                "0x8100" | "0x88a8" | "0x9100" | "0x9200"
            ) {
                return Err(format!("Unsupported QinQ ethertype '{ethertype}'"));
            }
            if kind != "qinq" && ethertype != "0x8100" {
                return Err("Non-default ethertype is valid only on QinQ ports".to_owned());
            }
            object.insert("name".into(), json!(format!("Ethernet{index}")));
            object.insert("port_number".into(), json!(index));
            object.insert("type".into(), json!(kind.clone()));
            object.insert("vlan".into(), json!(vlan));
            object.insert("ethertype".into(), json!(ethertype));
            Ok(port)
        })
        .collect()
}

fn normalize_hub_ports(mapping: Option<Value>) -> Result<Vec<Value>, String> {
    let ports = mapping
        .as_ref()
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(default_hub_ports);
    validate_port_count(&ports)?;
    ports
        .into_iter()
        .enumerate()
        .map(|(index, mut port)| {
            let object = port
                .as_object_mut()
                .ok_or_else(|| "Each hub port must be an object".to_owned())?;
            object.insert("name".into(), json!(format!("Ethernet{index}")));
            object.insert("port_number".into(), json!(index));
            Ok(port)
        })
        .collect()
}

fn normalize_cloud_ports(mapping: Option<Value>) -> Result<Vec<Value>, String> {
    let ports = mapping
        .as_ref()
        .and_then(Value::as_array)
        .filter(|ports| !ports.is_empty())
        .cloned()
        .unwrap_or_else(default_host_interfaces);
    if ports.len() > MAX_PORTS {
        return Err(format!("Builtin nodes support at most {MAX_PORTS} ports"));
    }
    let mut used_interfaces = HashSet::new();
    ports
        .into_iter()
        .enumerate()
        .map(|(index, mut port)| {
            let object = port
                .as_object_mut()
                .ok_or_else(|| "Each cloud port must be an object".to_owned())?;
            let kind = object
                .get("type")
                .and_then(Value::as_str)
                .ok_or_else(|| "Cloud ports require a type".to_owned())?
                .to_owned();
            if !matches!(kind.as_str(), "ethernet" | "tap" | "udp") {
                return Err(format!("Unsupported cloud port type '{kind}'"));
            }
            let name = object
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Ethernet{index}"));
            object.insert("name".into(), json!(name));
            object.insert("port_number".into(), json!(index));
            object.insert("type".into(), json!(kind.clone()));
            match kind.as_str() {
                "ethernet" | "tap" => {
                    let interface =
                        safe_interface_name(object.get("interface").and_then(Value::as_str))?;
                    if !used_interfaces.insert(interface.clone()) {
                        return Err(format!(
                            "Cloud interface '{interface}' is mapped more than once"
                        ));
                    }
                    object.insert("interface".into(), json!(interface));
                }
                "udp" => {
                    let local = object
                        .get("lport")
                        .and_then(Value::as_u64)
                        .unwrap_or_default();
                    let remote = object
                        .get("rport")
                        .and_then(Value::as_u64)
                        .unwrap_or_default();
                    if !(1..=65535).contains(&local) || !(1..=65535).contains(&remote) {
                        return Err("Cloud UDP ports must be from 1 to 65535".to_owned());
                    }
                    let remote_host = object
                        .get("rhost")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    validate_remote_host(&remote_host)?;
                    object.insert("lport".into(), json!(local));
                    object.insert("rport".into(), json!(remote));
                    object.insert("rhost".into(), json!(remote_host));
                }
                _ => unreachable!(),
            }
            Ok(port)
        })
        .collect()
}

fn normalize_nat_ports(
    object: &mut serde_json::Map<String, Value>,
    mapping: Option<Value>,
) -> Result<Vec<Value>, String> {
    let default = default_nat_interface();
    let interface = mapping
        .as_ref()
        .and_then(Value::as_array)
        .and_then(|ports| ports.first())
        .and_then(|port| port.get("interface"))
        .and_then(Value::as_str)
        .or_else(|| object.get("nat_interface").and_then(Value::as_str))
        .unwrap_or(&default);
    let interface = safe_interface_name(Some(interface))?;
    Ok(vec![
        json!({"name":"nat0","port_number":0,"type":"ethernet","interface":interface}),
    ])
}

/// Validate Cloud/NAT host interfaces against the actual host and controller
/// safety policy. A node's `allow_host_interfaces` array and the optional
/// `NETWORK_SIMULATOR_ALLOW_HOST_INTERFACES` operator allowlist grant explicit
/// exceptions. NAT always permits its configured default (`virbr0` by default);
/// Cloud requires an allowlist entry for loopback and default-route interfaces.
pub fn validate_host_interfaces(node: &Value) -> Result<(), String> {
    let node_type = node["node_type"].as_str().unwrap_or_default();
    if node_type != "cloud" && node_type != "nat" {
        return Ok(());
    }
    let mut allowed = configured_allowed_host_interfaces()?;
    allowed.extend(normalize_host_interface_allowlist(
        node.get("allow_host_interfaces"),
    )?);
    let default_routes = default_route_interfaces();
    let mappings = node["ports_mapping"]
        .as_array()
        .ok_or_else(|| "Builtin host node has no port mappings".to_owned())?;
    for mapping in mappings {
        let kind = mapping["type"].as_str().unwrap_or_default();
        if !matches!(kind, "ethernet" | "tap") {
            continue;
        }
        let interface = safe_interface_name(mapping["interface"].as_str())?;
        if !std::path::Path::new("/sys/class/net")
            .join(&interface)
            .exists()
        {
            return Err(format!("Host interface '{interface}' does not exist"));
        }
        validate_host_interface_policy(
            node_type,
            &interface,
            &default_nat_interface(),
            &allowed,
            &default_routes,
            is_loopback_interface(&interface),
        )?;
    }
    Ok(())
}

fn validate_host_interface_policy(
    node_type: &str,
    interface: &str,
    nat_default: &str,
    allowed: &HashSet<String>,
    default_routes: &HashSet<String>,
    loopback: bool,
) -> Result<(), String> {
    if node_type == "nat" {
        if interface != nat_default && !allowed.contains(interface) {
            return Err(format!(
                "NAT interface '{interface}' is not the configured default; add it to allow_host_interfaces to permit it"
            ));
        }
    } else {
        let protected = loopback || default_routes.contains(interface);
        if protected && !allowed.contains(interface) {
            return Err(format!(
                "Cloud interface '{interface}' is the controller loopback or default-route interface; add it to allow_host_interfaces to permit it"
            ));
        }
    }
    Ok(())
}

fn configured_allowed_host_interfaces() -> Result<HashSet<String>, String> {
    let mut allowed = HashSet::new();
    if let Some(configured) = std::env::var_os("NETWORK_SIMULATOR_ALLOW_HOST_INTERFACES") {
        for interface in configured.to_string_lossy().split(',').map(str::trim) {
            if interface.is_empty() {
                continue;
            }
            allowed.insert(safe_interface_name(Some(interface))?);
        }
    }
    Ok(allowed)
}

fn normalize_host_interface_allowlist(value: Option<&Value>) -> Result<HashSet<String>, String> {
    let Some(value) = value else {
        return Ok(HashSet::new());
    };
    let interfaces = value.as_array().ok_or_else(|| {
        "allow_host_interfaces must be an array of host interface names".to_owned()
    })?;
    interfaces
        .iter()
        .map(|interface| {
            let interface = interface.as_str().ok_or_else(|| {
                "allow_host_interfaces entries must be host interface names".to_owned()
            })?;
            safe_interface_name(Some(interface))
        })
        .collect()
}

fn default_route_interfaces() -> HashSet<String> {
    let mut interfaces = HashSet::new();
    if let Ok(routes) = std::fs::read_to_string("/proc/net/route") {
        for line in routes.lines().skip(1) {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() < 4 || fields[1] != "00000000" {
                continue;
            }
            if u32::from_str_radix(fields[3], 16).is_ok_and(|flags| flags & 1 != 0) {
                interfaces.insert(fields[0].to_owned());
            }
        }
    }
    if let Ok(routes) = std::fs::read_to_string("/proc/net/ipv6_route") {
        for line in routes.lines() {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() >= 10 && fields[0].bytes().all(|byte| byte == b'0') && fields[1] == "00"
            {
                interfaces.insert(fields[9].to_owned());
            }
        }
    }
    interfaces
}

fn is_loopback_interface(interface: &str) -> bool {
    if interface == "lo" {
        return true;
    }
    std::fs::read_to_string(
        std::path::Path::new("/sys/class/net")
            .join(interface)
            .join("flags"),
    )
    .ok()
    .and_then(|flags| u32::from_str_radix(flags.trim().trim_start_matches("0x"), 16).ok())
    .is_some_and(|flags| flags & 0x8 != 0)
}

fn normalize_cloud_console(object: &mut serde_json::Map<String, Value>) -> Result<(), String> {
    let host = object
        .get("remote_console_host")
        .and_then(Value::as_str)
        .unwrap_or("127.0.0.1")
        .to_owned();
    if host.len() > 255 || host.bytes().any(|byte| byte.is_ascii_control()) {
        return Err("remote_console_host is invalid".to_owned());
    }
    let port = object
        .get("remote_console_port")
        .and_then(Value::as_u64)
        .unwrap_or(23);
    if !(1..=65535).contains(&port) {
        return Err("remote_console_port must be from 1 to 65535".to_owned());
    }
    let console_type = object
        .get("remote_console_type")
        .and_then(Value::as_str)
        .unwrap_or("none")
        .to_owned();
    if !matches!(
        console_type.as_str(),
        "none" | "telnet" | "ssh" | "vnc" | "spice" | "http" | "https"
    ) {
        return Err("Unsupported cloud remote console type".to_owned());
    }
    let path = object
        .get("remote_console_http_path")
        .and_then(Value::as_str)
        .unwrap_or("/")
        .to_owned();
    if !path.starts_with('/') || path.bytes().any(|byte| byte.is_ascii_control()) {
        return Err("remote_console_http_path must be an absolute path".to_owned());
    }
    object.insert("remote_console_host".into(), json!(host));
    object.insert("remote_console_port".into(), json!(port));
    object.insert("remote_console_type".into(), json!(console_type));
    object.insert("remote_console_http_path".into(), json!(path));
    Ok(())
}

fn validate_port_count(ports: &[Value]) -> Result<(), String> {
    if ports.is_empty() || ports.len() > MAX_PORTS {
        return Err(format!(
            "Builtin nodes require between 1 and {MAX_PORTS} ports"
        ));
    }
    Ok(())
}

fn safe_interface_name(name: Option<&str>) -> Result<String, String> {
    let name = name
        .map(str::trim)
        .filter(|name| !name.is_empty() && name.len() <= 15)
        .ok_or_else(|| "A host interface name is required".to_owned())?;
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(
            "Host interface names may contain only letters, digits, '.', '-', or '_'".to_owned(),
        );
    }
    Ok(name.to_owned())
}

fn validate_remote_host(host: &str) -> Result<(), String> {
    if host.is_empty()
        || host.len() > 255
        || !host.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b':' | b'%' | b'[' | b']')
        })
    {
        return Err("Cloud UDP remote host must be an IP address or safe hostname".to_owned());
    }
    Ok(())
}


fn port_from_endpoint(endpoint: &Value, key: &str) -> Result<u16, RuntimeError> {
    endpoint
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port != 0)
        .ok_or_else(|| RuntimeError::Failed(format!("Builtin link endpoint has no valid {key}")))
}

fn port_from_mapping(mapping: &Value, key: &str) -> Result<u16, RuntimeError> {
    mapping
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port != 0)
        .ok_or_else(|| RuntimeError::Failed(format!("Cloud UDP port has no valid {key}")))
}

fn required_mapping_text(mapping: &Value, key: &str) -> Result<String, RuntimeError> {
    mapping
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| RuntimeError::Failed(format!("Builtin port mapping has no {key}")))
}

fn safe_remote_host(mapping: &Value) -> Result<String, RuntimeError> {
    let host = required_mapping_text(mapping, "rhost")?;
    validate_remote_host(&host).map_err(|message| RuntimeError::Failed(message))?;
    Ok(host)
}

fn host_ethernet_tap_name(endpoint: &Value) -> Result<String, RuntimeError> {
    let endpoint_id = endpoint
        .get("runtime_endpoint_id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            RuntimeError::Failed("Builtin link endpoint has no runtime ID".to_owned())
        })?;
    let compact = endpoint_id
        .chars()
        .filter(|character| character.is_ascii_hexdigit())
        .take(11)
        .collect::<String>();
    if compact.len() != 11 {
        return Err(RuntimeError::Failed(
            "Builtin link endpoint ID is invalid".to_owned(),
        ));
    }
    Ok(format!("nsct{compact}"))
}


#[cfg(test)]
mod tests {
    use super::*;

    fn ethernet_frame(destination: [u8; 6], source: [u8; 6]) -> Vec<u8> {
        let mut frame = Vec::from(destination);
        frame.extend_from_slice(&source);
        frame.extend_from_slice(&[0x08, 0x00, 0, 1, 2, 3]);
        frame
    }

    #[test]
    fn builtin_templates_have_upstream_names_and_port_shapes() {
        let templates = BuiltinRuntime::templates();
        assert_eq!(templates.len(), 4);
        assert_eq!(templates[0]["ports_mapping"].as_array().unwrap().len(), 8);
        assert_eq!(templates[0]["ports_mapping"][7]["name"], "Ethernet7");
        assert_eq!(templates[1]["ports_mapping"].as_array().unwrap().len(), 8);
        assert_eq!(templates[2]["template_type"], "cloud");
        assert_eq!(templates[3]["ports_mapping"][0]["name"], "nat0");
    }

    #[test]
    fn switch_access_vlan_rejects_cross_vlan_ingress_and_normalizes_ports() {
        let node = BuiltinRuntime::normalize_node(json!({
            "name":"S1","node_type":"ethernet_switch",
            "ports_mapping":[
                {"type":"access","vlan":7},
                {"type":"access","vlan":8}
            ]
        }))
        .unwrap();
        assert_eq!(node["ports_mapping"][1]["name"], "Ethernet1");
        let frame = ethernet_frame([0xff; 6], [0x02, 0, 0, 0, 0, 1]);
        assert_eq!(
            classify_ingress(&switch_mode(&node["ports_mapping"][0]).unwrap(), &frame)
                .unwrap()
                .0,
            7
        );
        assert!(!vlan_allowed(
            &switch_mode(&node["ports_mapping"][1]).unwrap(),
            7
        ));
    }

    #[test]
    fn dot1q_and_qinq_egress_preserve_vlan_tags() {
        let frame = ethernet_frame([0x02, 0, 0, 0, 0, 2], [0x02, 0, 0, 0, 0, 1]);
        let tagged =
            classify_egress(&SwitchPortMode::Dot1q { native_vlan: 1 }, 42, &frame).unwrap();
        assert_eq!(&tagged[12..16], &[0x81, 0x00, 0x00, 0x2a]);
        let qinq = classify_egress(
            &SwitchPortMode::QinQ {
                vlan: 42,
                ethertype: 0x88a8,
            },
            42,
            &frame,
        )
        .unwrap();
        assert_eq!(&qinq[12..16], &[0x88, 0xa8, 0x00, 0x2a]);
    }

    #[test]
    fn normalizes_cloud_ethernet_udp_and_tap_ports() {
        let node = BuiltinRuntime::normalize_node(json!({
            "name":"cloud1","node_type":"cloud",
            "ports_mapping":[
                {"type":"ethernet","interface":"ens33"},
                {"type":"tap","interface":"tap0"},
                {"type":"udp","lport":4001,"rhost":"127.0.0.1","rport":4002}
            ]
        }))
        .unwrap();
        assert_eq!(node["ports_mapping"][2]["port_number"], 2);
        assert_eq!(
            BuiltinRuntime::adapter_nio(
                &node,
                &json!({"port_number":0,"runtime_endpoint_id":"0123456789ab-cdef"})
            )
            .unwrap(),
            AdapterNio::HostEthernet {
                interface: "ens33".to_owned(),
                tap_interface: "nsct0123456789a".to_owned(),
            }
        );
        assert_eq!(
            BuiltinRuntime::adapter_nio(&node, &json!({"port_number":1})).unwrap(),
            AdapterNio::Tap {
                interface: "tap0".to_owned(),
            }
        );
        assert_eq!(
            BuiltinRuntime::adapter_nio(&node, &json!({"port_number":2})).unwrap(),
            AdapterNio::ExternalUdp {
                local_port: 4001,
                remote_host: "127.0.0.1".to_owned(),
                remote_port: 4002,
            }
        );
    }

    #[test]
    fn rejects_invalid_vlan_and_shell_syntax_in_host_mapping() {
        let invalid_vlan = BuiltinRuntime::normalize_node(json!({
            "name":"S1","node_type":"ethernet_switch",
            "ports_mapping":[{"type":"access","vlan":4095}]
        }));
        assert!(invalid_vlan.is_err());
        let invalid_interface = BuiltinRuntime::normalize_node(json!({
            "name":"cloud1","node_type":"cloud",
            "ports_mapping":[{"type":"ethernet","interface":"eth0;reboot"}]
        }));
        assert!(invalid_interface.is_err());
    }

    #[test]
    fn host_interface_policy_requires_allowlisting_protected_and_nondefault_nat_ports() {
        let routes = HashSet::from(["ens33".to_owned()]);
        let no_allowlist = HashSet::new();
        assert!(
            validate_host_interface_policy(
                "cloud",
                "ens33",
                "virbr0",
                &no_allowlist,
                &routes,
                false,
            )
            .is_err()
        );
        assert!(
            validate_host_interface_policy("cloud", "lo", "virbr0", &no_allowlist, &routes, true,)
                .is_err()
        );
        let allowed = HashSet::from(["ens33".to_owned(), "lo".to_owned(), "ens34".to_owned()]);
        assert!(
            validate_host_interface_policy("cloud", "ens33", "virbr0", &allowed, &routes, false,)
                .is_ok()
        );
        assert!(
            validate_host_interface_policy("cloud", "lo", "virbr0", &allowed, &routes, true,)
                .is_ok()
        );
        assert!(
            validate_host_interface_policy(
                "nat",
                "virbr0",
                "virbr0",
                &no_allowlist,
                &routes,
                false,
            )
            .is_ok()
        );
        assert!(validate_host_interface_policy(
            "nat",
            "ens34",
            "virbr0",
            &no_allowlist,
            &routes,
            false,
        )
        .is_err());
        assert!(
            validate_host_interface_policy("nat", "ens34", "virbr0", &allowed, &routes, false,)
                .is_ok()
        );
    }

    #[test]
    fn node_allow_host_interfaces_is_a_validated_explicit_exception_list() {
        let cloud = BuiltinRuntime::normalize_node(json!({
            "name":"Cloud1",
            "node_type":"cloud",
            "ports_mapping":[{"type":"ethernet","interface":"ens33"}],
            "allow_host_interfaces":["ens33"]
        }))
        .expect("valid allowlist normalizes");
        assert_eq!(cloud["allow_host_interfaces"], json!(["ens33"]));
        let patch = BuiltinRuntime::normalize_configuration_patch(
            &cloud,
            json!({"allow_host_interfaces":["lo"]}),
        )
        .expect("allowlist is editable as a node setting");
        assert_eq!(patch["allow_host_interfaces"], json!(["lo"]));

        let invalid = BuiltinRuntime::normalize_node(json!({
            "name":"Cloud2",
            "node_type":"cloud",
            "ports_mapping":[{"type":"ethernet","interface":"ens33"}],
            "allow_host_interfaces":["ens33\nstop"]
        }));
        assert!(invalid.is_err());
    }

    #[tokio::test]
    async fn hub_floods_every_frame_to_all_other_active_ports() {
        let mut ports = HashMap::new();
        for port in 0..3u16 {
            let socket = Arc::new(UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap());
            ports.insert(
                port,
                RuntimePort {
                    socket,
                    relay_address: SocketAddr::from((Ipv4Addr::LOCALHOST, 20000 + port)),
                    mode: None,
                },
            );
        }
        let forwarding = Arc::new(Mutex::new(ForwardingNode {
            node_type: "ethernet_hub".to_owned(),
            modes: HashMap::new(),
            ports,
            learned: HashMap::new(),
        }));
        let frame = ethernet_frame([0x02, 0, 0, 0, 0, 2], [0x02, 0, 0, 0, 0, 1]);
        let forwarded = forward_frame(&forwarding, 0, frame.clone()).await;
        assert_eq!(forwarded.len(), 2);
        assert!(forwarded.iter().all(|(_, _, packet)| packet == &frame));
    }

    #[tokio::test]
    async fn switch_learns_unicast_and_keeps_access_vlans_isolated() {
        let socket =
            || async { Arc::new(UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap()) };
        let port_socket_0 = socket().await;
        let port_socket_1 = socket().await;
        let port_socket_2 = socket().await;
        let ports = HashMap::from([
            (
                0,
                RuntimePort {
                    socket: port_socket_0,
                    relay_address: SocketAddr::from((Ipv4Addr::LOCALHOST, 20000)),
                    mode: Some(SwitchPortMode::Access { vlan: 10 }),
                },
            ),
            (
                1,
                RuntimePort {
                    socket: port_socket_1,
                    relay_address: SocketAddr::from((Ipv4Addr::LOCALHOST, 20001)),
                    mode: Some(SwitchPortMode::Access { vlan: 10 }),
                },
            ),
            (
                2,
                RuntimePort {
                    socket: port_socket_2,
                    relay_address: SocketAddr::from((Ipv4Addr::LOCALHOST, 20002)),
                    mode: Some(SwitchPortMode::Access { vlan: 20 }),
                },
            ),
        ]);
        let forwarding = Arc::new(Mutex::new(ForwardingNode {
            node_type: "ethernet_switch".to_owned(),
            modes: HashMap::from([
                (0, SwitchPortMode::Access { vlan: 10 }),
                (1, SwitchPortMode::Access { vlan: 10 }),
                (2, SwitchPortMode::Access { vlan: 20 }),
            ]),
            ports,
            learned: HashMap::new(),
        }));
        let host_a = [0x02, 0, 0, 0, 0, 1];
        let host_b = [0x02, 0, 0, 0, 0, 2];
        let flood = forward_frame(&forwarding, 0, ethernet_frame([0xff; 6], host_a)).await;
        assert_eq!(flood.len(), 1);
        assert_eq!(flood[0].1.port(), 20001);

        let learned = forward_frame(&forwarding, 1, ethernet_frame(host_a, host_b)).await;
        assert_eq!(learned.len(), 1);
        assert_eq!(learned[0].1.port(), 20000);

        let isolated = forward_frame(
            &forwarding,
            2,
            ethernet_frame(host_a, [0x02, 0, 0, 0, 0, 3]),
        )
        .await;
        assert!(isolated.is_empty());
    }

    #[test]
    fn switch_forwarding_table_is_bounded_and_evicts_the_oldest_entry() {
        let mut node = ForwardingNode {
            node_type: "ethernet_switch".to_owned(),
            modes: HashMap::new(),
            ports: HashMap::new(),
            learned: HashMap::new(),
        };
        let start = Instant::now();
        let mac_for = |index: u64| {
            [
                0x02,
                (index >> 32) as u8,
                (index >> 24) as u8,
                (index >> 16) as u8,
                (index >> 8) as u8,
                index as u8,
            ]
        };
        let oldest = mac_for(0);
        for index in 0..=MAX_FDB_ENTRIES {
            learn_address(
                &mut node,
                1,
                mac_for(index as u64),
                0,
                start + Duration::from_millis(index as u64),
            );
        }
        assert_eq!(node.learned.len(), MAX_FDB_ENTRIES);
        assert!(!node.learned.contains_key(&(1, oldest)));
        assert!(
            node.learned
                .contains_key(&(1, mac_for(MAX_FDB_ENTRIES as u64)))
        );
    }

    #[test]
    fn switch_forwarding_entries_age_after_the_configured_lifetime() {
        let mut node = ForwardingNode {
            node_type: "ethernet_switch".to_owned(),
            modes: HashMap::new(),
            ports: HashMap::new(),
            learned: HashMap::new(),
        };
        let now = Instant::now();
        learn_address(&mut node, 1, [2, 0, 0, 0, 0, 1], 0, now);
        age_forwarding_node(&mut node, now + FDB_AGE);
        assert!(node.learned.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires upstream uBridge with CAP_NET_ADMIN and CAP_NET_RAW"]
    async fn cloud_udp_nio_exchanges_frames_with_the_link_engine() {
        use std::{env, path::PathBuf};

        use crate::{links::LinkEngine, ubridge::UbridgeRuntime};
        use tokio::time::timeout;

        let binary = env::var_os("NETWORK_SIMULATOR_TEST_UBRIDGE")
            .expect("NETWORK_SIMULATOR_TEST_UBRIDGE names a capable upstream uBridge");
        let directory = tempfile::tempdir().expect("temporary uBridge state");
        let ubridge = UbridgeRuntime::with_paths(
            PathBuf::from(binary),
            directory.path().join("ubridge.sock"),
        );
        let engine = LinkEngine::new(ubridge.clone());
        let external_peer = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind Cloud UDP peer");
        let external_port = external_peer.local_addr().unwrap().port();
        let cloud_local_port = ubridge.reserve_udp_port().await.unwrap();
        let cloud_id = Uuid::new_v4().to_string();
        let peer_id = Uuid::new_v4().to_string();
        let cloud = BuiltinRuntime::normalize_node(json!({
            "node_id": cloud_id,
            "name": "Cloud UDP live test",
            "node_type": "cloud",
            "ports_mapping": [{
                "name": "External UDP",
                "type": "udp",
                "lport": cloud_local_port,
                "rhost": "127.0.0.1",
                "rport": external_port
            }]
        }))
        .expect("normalize Cloud UDP mapping");
        let mut link = json!({
            "link_id": Uuid::new_v4().to_string(),
            "nodes": [
                {"node_id": cloud_id, "node_type": "cloud", "adapter_number": 0, "port_number": 0},
                {"node_id": peer_id, "node_type": "qemu", "adapter_number": 0, "port_number": 0}
            ]
        });
        engine
            .reserve_pair(&mut link)
            .await
            .expect("allocate link NIO ports");
        let endpoints = link["nodes"].as_array().unwrap();
        let cloud_endpoint = endpoints[0].clone();
        let peer_endpoint = endpoints[1].clone();
        let runtime_port = peer_endpoint["runtime_udp_port"].as_u64().unwrap() as u16;
        let adapter_port = peer_endpoint["runtime_adapter_port"].as_u64().unwrap() as u16;
        let peer_runtime = UdpSocket::bind((Ipv4Addr::LOCALHOST, runtime_port))
            .await
            .expect("bind peer runtime NIO");
        let run: Result<(), String> = async {
            let cloud_nio = BuiltinRuntime::adapter_nio(&cloud, &cloud_endpoint)
                .map_err(|error| error.message().to_owned())?;
            engine
                .attach_endpoint(&cloud_endpoint, cloud_nio, &json!({}), false, None)
                .await
                .map_err(|error| error.message().to_owned())?;
            engine
                .attach_endpoint(
                    &peer_endpoint,
                    AdapterNio::Udp {
                        adapter_port,
                        runtime_port,
                    },
                    &json!({}),
                    false,
                    None,
                )
                .await
                .map_err(|error| error.message().to_owned())?;

            let from_cloud = vec![
                0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02, 0, 0, 0, 0, 1, 0x88, 0xb5, 1, 2, 3,
            ];
            external_peer
                .send_to(&from_cloud, (Ipv4Addr::LOCALHOST, cloud_local_port))
                .await
                .map_err(|error| error.to_string())?;
            let mut received = [0u8; 128];
            let (length, _) = timeout(
                Duration::from_secs(3),
                peer_runtime.recv_from(&mut received),
            )
            .await
            .map_err(|_| "Cloud UDP frame did not reach the link peer".to_owned())?
            .map_err(|error| error.to_string())?;
            if received[..length] != from_cloud {
                return Err("Cloud UDP link changed the incoming Ethernet frame".to_owned());
            }

            let from_peer = vec![
                0x02, 0, 0, 0, 0, 1, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x88, 0xb5, 4, 5, 6,
            ];
            peer_runtime
                .send_to(&from_peer, (Ipv4Addr::LOCALHOST, adapter_port))
                .await
                .map_err(|error| error.to_string())?;
            let (length, _) = timeout(
                Duration::from_secs(3),
                external_peer.recv_from(&mut received),
            )
            .await
            .map_err(|_| "Link peer frame did not reach the Cloud UDP peer".to_owned())?
            .map_err(|error| error.to_string())?;
            if received[..length] != from_peer {
                return Err("Cloud UDP link changed the outgoing Ethernet frame".to_owned());
            }
            Ok(())
        }
        .await;

        let _ = engine.detach_endpoint(&cloud_endpoint).await;
        let _ = engine.detach_endpoint(&peer_endpoint).await;
        assert!(
            !ubridge
                .has_link_endpoint(cloud_endpoint["runtime_endpoint_id"].as_str().unwrap())
                .await
        );
        assert!(
            !ubridge
                .has_link_endpoint(peer_endpoint["runtime_endpoint_id"].as_str().unwrap())
                .await
        );
        assert!(run.is_ok(), "Cloud UDP live test failed: {run:?}");
    }

    #[tokio::test]
    #[ignore = "requires Docker with alpine:latest, capable uBridge, and active libvirt default NAT network 'default' bridged as virbr0 with 192.168.122.1/24"]
    async fn nat_live_routes_docker_traffic_through_libvirt() {
        use std::{env, path::PathBuf};

        use crate::{
            links::{LinkEngine, NioAdapter},
            runtime::{DockerRuntime, NodeAction},
            ubridge::UbridgeRuntime,
        };
        use tokio::{process::Command, time::timeout};

        let ubridge_binary = env::var_os("NETWORK_SIMULATOR_TEST_UBRIDGE")
            .expect("NETWORK_SIMULATOR_TEST_UBRIDGE names a capable upstream uBridge");
        let directory = tempfile::tempdir().expect("temporary builtin NAT test state");
        let ubridge = UbridgeRuntime::with_paths(
            PathBuf::from(ubridge_binary),
            directory.path().join("ubridge.sock"),
        );
        let docker = DockerRuntime::new();
        let engine = LinkEngine::new(ubridge.clone());
        let docker_binary = env::var_os("NETWORK_SIMULATOR_DOCKER")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("docker"));
        let project_id = Uuid::new_v4().to_string();
        let nat_id = Uuid::new_v4().to_string();
        let docker_id = Uuid::new_v4().to_string();
        let mut nat = BuiltinRuntime::normalize_node(json!({
            "node_id": nat_id,
            "name": "NAT live test",
            "node_type": "nat",
            "ports_mapping": [{"type":"ethernet", "interface":"virbr0"}]
        }))
        .expect("normalize libvirt NAT node");
        nat["project_id"] = json!(project_id);
        let mut link = json!({
            "link_id": Uuid::new_v4().to_string(),
            "nodes": [
                {"node_id": nat_id, "node_type":"nat", "adapter_number":0, "port_number":0},
                {"node_id": docker_id, "node_type":"docker", "adapter_number":0, "port_number":0}
            ]
        });
        engine
            .reserve_pair(&mut link)
            .await
            .expect("allocate NAT link NIO ports");
        let endpoints = link["nodes"].as_array().unwrap();
        let nat_endpoint = endpoints[0].clone();
        let docker_endpoint = endpoints[1].clone();
        let nat_tap = host_ethernet_tap_name(&nat_endpoint).expect("derive NAT bridge TAP");
        let docker_host = crate::ubridge::host_veth_name(
            docker_endpoint["runtime_endpoint_id"].as_str().unwrap(),
        )
        .expect("derive Docker host veth");
        let docker_guest = crate::ubridge::guest_veth_name(
            docker_endpoint["runtime_endpoint_id"].as_str().unwrap(),
        )
        .expect("derive Docker guest veth");
        let docker_node = Uuid::parse_str(&docker_id).unwrap();
        let mac = format!(
            "02:{:02x}:{:02x}:{:02x}:{:02x}:00",
            docker_node.as_bytes()[0],
            docker_node.as_bytes()[1],
            docker_node.as_bytes()[2],
            docker_node.as_bytes()[3]
        );
        let mut container_id = None;

        let run: Result<(), String> = async {
            let network = Command::new("virsh")
                .args(["net-info", "default"])
                .output()
                .await
                .map_err(|error| format!("Could not inspect libvirt default network: {error}"))?;
            let network_info = String::from_utf8_lossy(&network.stdout);
            let network_field = |name: &str| {
                network_info.lines().any(|line| {
                    line.split_once(':')
                        .is_some_and(|(key, value)| key.trim() == name && value.trim() == "yes")
                })
            };
            if !network.status.success()
                || !network_field("Active")
                || !network_info.lines().any(|line| {
                    line.split_once(':').is_some_and(|(key, value)| {
                        key.trim() == "Bridge" && value.trim() == "virbr0"
                    })
                })
            {
                return Err(format!(
                    "libvirt default network 'default' must be active and use virbr0; virsh net-info returned: {}{}",
                    network_info,
                    String::from_utf8_lossy(&network.stderr)
                ));
            }
            let bridge_address = Command::new("ip")
                .args(["-o", "-4", "addr", "show", "dev", "virbr0"])
                .output()
                .await
                .map_err(|error| format!("Could not inspect virbr0 address: {error}"))?;
            let bridge_address_text = String::from_utf8_lossy(&bridge_address.stdout);
            if !bridge_address.status.success()
                || !bridge_address_text.contains("192.168.122.1/24")
            {
                return Err(format!(
                    "libvirt default network 'default' must configure 192.168.122.1/24 on virbr0; ip addr returned: {}{}",
                    bridge_address_text,
                    String::from_utf8_lossy(&bridge_address.stderr)
                ));
            }
            BuiltinRuntime::validate_host_interfaces(&nat)
                .map_err(|error| format!("configured NAT interface failed validation: {error}"))?;
            let (created, _) = docker
                .create_container(
                    &project_id,
                    &docker_id,
                    "alpine:latest",
                    "Built-in NAT live test",
                    Some("/bin/sh -c 'sleep 3600'"),
                    None,
                    None,
                )
                .await
                .map_err(|error| error.message().to_owned())?;
            container_id = Some(created.clone());
            docker
                .lifecycle(&created, NodeAction::Start)
                .await
                .map_err(|error| error.message().to_owned())?;
            let pid = docker
                .container_pid(&created)
                .await
                .map_err(|error| error.message().to_owned())?;
            ubridge
                .create_veth_pair(&docker_host, &docker_guest)
                .await
                .map_err(|error| error.message().to_owned())?;
            ubridge
                .set_docker_mac_address(&docker_guest, &mac)
                .await
                .map_err(|error| error.message().to_owned())?;
            ubridge
                .set_interface_up(&docker_guest)
                .await
                .map_err(|error| error.message().to_owned())?;
            ubridge
                .set_interface_up(&docker_host)
                .await
                .map_err(|error| error.message().to_owned())?;
            ubridge
                .move_veth_to_container(&docker_guest, pid, "eth0")
                .await
                .map_err(|error| error.message().to_owned())?;

            let nat_nio = BuiltinRuntime::adapter_nio(&nat, &nat_endpoint)
                .map_err(|error| error.message().to_owned())?;
            let attach_started = Instant::now();
            engine
                .attach_endpoint(&nat_endpoint, nat_nio, &json!({}), false, None)
                .await
                .map_err(|error| error.message().to_owned())?;
            eprintln!("NAT TAP attachment reached forwarding after {:?}", attach_started.elapsed());
            let bridge_ports = Command::new("bridge")
                .args(["link", "show", "master", "virbr0"])
                .output()
                .await
                .map_err(|error| format!("Could not inspect virbr0 bridge ports: {error}"))?;
            if !String::from_utf8_lossy(&bridge_ports.stdout).contains(&nat_tap) {
                return Err(format!(
                    "Cloud/NAT TAP {nat_tap} is not enslaved to virbr0: {}",
                    String::from_utf8_lossy(&bridge_ports.stdout)
                ));
            }
            // Attachment must return only after the libvirt bridge can
            // forward the guest's first ARP packet.
            let bridge_state_path = format!("/sys/class/net/{nat_tap}/brport/state");
            let bridge_state = std::fs::read_to_string(&bridge_state_path)
                .map_err(|error| format!("Could not read bridge state for {nat_tap}: {error}"))?;
            if bridge_state.trim() != "3" {
                return Err(format!("NAT TAP {nat_tap} was reported ready in bridge state {}", bridge_state.trim()));
            }
            let docker_nio = docker
                .adapter_nio(&docker_endpoint)
                .map_err(|error| error.message().to_owned())?;
            engine
                .attach_endpoint(&docker_endpoint, docker_nio, &json!({}), false, None)
                .await
                .map_err(|error| error.message().to_owned())?;
            let capture_paths = [
                (
                    nat_endpoint["runtime_endpoint_id"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    directory.path().join("nat-endpoint.pcap"),
                ),
                (
                    docker_endpoint["runtime_endpoint_id"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    directory.path().join("docker-endpoint.pcap"),
                ),
            ];
            for (endpoint_id, path) in &capture_paths {
                engine
                    .start_capture(
                        &json!({"runtime_endpoint_id": endpoint_id}),
                        &path.to_string_lossy(),
                    )
                    .await
                    .map_err(|error| error.message().to_owned())?;
            }

            for arguments in [
                vec!["ip", "link", "set", "eth0", "up"],
                vec!["ip", "addr", "add", "192.168.122.250/24", "dev", "eth0"],
                vec!["ip", "route", "replace", "default", "via", "192.168.122.1"],
            ] {
                let output = Command::new(&docker_binary)
                    .args(["exec", &created])
                    .args(arguments)
                    .output()
                    .await
                    .map_err(|error| {
                        format!("Could not configure Alpine NAT interface: {error}")
                    })?;
                if !output.status.success() {
                    return Err(format!(
                        "Could not configure Alpine NAT interface: {}",
                        String::from_utf8_lossy(&output.stderr).trim()
                    ));
                }
            }

            for destination in ["192.168.122.1", "1.1.1.1"] {
                let output = timeout(
                    Duration::from_secs(8),
                    Command::new(&docker_binary)
                        .args(["exec", &created, "ping", "-c", "1", "-W", "5", destination])
                        .output(),
                )
                .await
                .map_err(|_| format!("Alpine ping to {destination} timed out"))?
                .map_err(|error| format!("Alpine ping to {destination} could not run: {error}"))?;
                if !output.status.success() {
                    let neighbors = Command::new(&docker_binary)
                        .args(["exec", &created, "ip", "neigh", "show", "dev", "eth0"])
                        .output()
                        .await
                        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                        .unwrap_or_default();
                    let tap_state = Command::new("ip")
                        .args(["-d", "link", "show", "dev", &nat_tap])
                        .output()
                        .await
                        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                        .unwrap_or_default();
                    let tap_stats = Command::new("ip")
                        .args(["-s", "link", "show", "dev", &nat_tap])
                        .output()
                        .await
                        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                        .unwrap_or_default();
                    let bridge_stats = Command::new("ip")
                        .args(["-s", "link", "show", "dev", "virbr0"])
                        .output()
                        .await
                        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                        .unwrap_or_default();
                    let host_neighbors = Command::new("ip")
                        .args(["neigh", "show", "dev", "virbr0"])
                        .output()
                        .await
                        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                        .unwrap_or_default();
                    let fdb = Command::new("bridge")
                        .args(["fdb", "show", "br", "virbr0"])
                        .output()
                        .await
                        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                        .unwrap_or_default();
                    let mut capture_output = String::new();
                    for (endpoint_id, path) in &capture_paths {
                        let _ = engine
                            .stop_capture(&json!({"runtime_endpoint_id": endpoint_id}))
                            .await;
                        let capture = Command::new("tcpdump")
                            .args(["-nn", "-e", "-tt", "-r"])
                            .arg(path)
                            .arg("arp or icmp")
                            .output()
                            .await
                            .map(|output| {
                                format!(
                                    "{}{}",
                                    String::from_utf8_lossy(&output.stdout),
                                    String::from_utf8_lossy(&output.stderr)
                                )
                            })
                            .unwrap_or_else(|error| format!("capture decode failed: {error}"));
                        capture_output.push_str(&format!("\n{endpoint_id}:\n{capture}"));
                    }
                    return Err(format!(
                        "Alpine could not ping {destination}: {}{}; guest_neighbors={neighbors}; host_virbr0_neighbors={host_neighbors}; tap={tap_state}; tap_stats={tap_stats}; virbr0_stats={bridge_stats}; virbr0_fdb={fdb}; captures={capture_output}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    ));
                }
            }
            Ok(())
        }
        .await;

        let _ = engine.detach_endpoint(&nat_endpoint).await;
        let _ = engine.detach_endpoint(&docker_endpoint).await;
        let _ = ubridge.delete_veth(&docker_host).await;
        if let Some(container_id) = container_id.as_deref() {
            let _ = docker
                .remove_node_resources(&docker_id, Some(container_id))
                .await;
        }
        let nat_tap_remaining = Command::new("ip")
            .args(["link", "show", "dev", &nat_tap])
            .output()
            .await
            .expect("inspect cleaned NAT TAP");
        let docker_veth_remaining = Command::new("ip")
            .args(["link", "show", "dev", &docker_host])
            .output()
            .await
            .expect("inspect cleaned Docker veth");
        assert!(
            !nat_tap_remaining.status.success(),
            "NAT bridge TAP remained: {nat_tap}"
        );
        assert!(
            !docker_veth_remaining.status.success(),
            "Docker host veth remained: {docker_host}"
        );
        assert!(
            !ubridge
                .has_link_endpoint(nat_endpoint["runtime_endpoint_id"].as_str().unwrap())
                .await
        );
        assert!(
            !ubridge
                .has_link_endpoint(docker_endpoint["runtime_endpoint_id"].as_str().unwrap())
                .await
        );
        assert!(run.is_ok(), "NAT live test failed: {run:?}");
    }
}
