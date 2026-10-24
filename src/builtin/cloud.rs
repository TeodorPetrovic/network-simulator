use super::*;

pub(super) fn default_nat_interface() -> String {
    std::env::var("NETWORK_SIMULATOR_NAT_INTERFACE").unwrap_or_else(|_| "virbr0".to_owned())
}

pub(super) fn default_host_interfaces() -> Vec<Value> {
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

pub(super) fn normalize_switch_ports(mapping: Option<Value>) -> Result<Vec<Value>, String> {
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

pub(super) fn normalize_hub_ports(mapping: Option<Value>) -> Result<Vec<Value>, String> {
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

pub(super) fn normalize_cloud_ports(mapping: Option<Value>) -> Result<Vec<Value>, String> {
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

pub(super) fn normalize_nat_ports(
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

pub(super) fn validate_host_interface_policy(
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

pub(super) fn configured_allowed_host_interfaces() -> Result<HashSet<String>, String> {
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

pub(super) fn normalize_host_interface_allowlist(value: Option<&Value>) -> Result<HashSet<String>, String> {
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

pub(super) fn default_route_interfaces() -> HashSet<String> {
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

pub(super) fn is_loopback_interface(interface: &str) -> bool {
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

pub(super) fn normalize_cloud_console(object: &mut serde_json::Map<String, Value>) -> Result<(), String> {
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

pub(super) fn validate_port_count(ports: &[Value]) -> Result<(), String> {
    if ports.is_empty() || ports.len() > MAX_PORTS {
        return Err(format!(
            "Builtin nodes require between 1 and {MAX_PORTS} ports"
        ));
    }
    Ok(())
}

pub(super) fn safe_interface_name(name: Option<&str>) -> Result<String, String> {
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

pub(super) fn validate_remote_host(host: &str) -> Result<(), String> {
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


pub(super) fn port_from_endpoint(endpoint: &Value, key: &str) -> Result<u16, RuntimeError> {
    endpoint
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port != 0)
        .ok_or_else(|| RuntimeError::Failed(format!("Builtin link endpoint has no valid {key}")))
}

pub(super) fn port_from_mapping(mapping: &Value, key: &str) -> Result<u16, RuntimeError> {
    mapping
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port != 0)
        .ok_or_else(|| RuntimeError::Failed(format!("Cloud UDP port has no valid {key}")))
}

pub(super) fn required_mapping_text(mapping: &Value, key: &str) -> Result<String, RuntimeError> {
    mapping
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| RuntimeError::Failed(format!("Builtin port mapping has no {key}")))
}

pub(super) fn safe_remote_host(mapping: &Value) -> Result<String, RuntimeError> {
    let host = required_mapping_text(mapping, "rhost")?;
    validate_remote_host(&host).map_err(|message| RuntimeError::Failed(message))?;
    Ok(host)
}

pub(super) fn host_ethernet_tap_name(endpoint: &Value) -> Result<String, RuntimeError> {
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


