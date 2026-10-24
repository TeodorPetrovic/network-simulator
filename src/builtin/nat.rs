use super::*;

pub(super) fn default_nat_interface() -> String {
    std::env::var("NETWORK_SIMULATOR_NAT_INTERFACE").unwrap_or_else(|_| "virbr0".to_owned())
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
