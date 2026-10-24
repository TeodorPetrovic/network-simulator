use super::*;

pub(super) fn default_hub_ports() -> Vec<Value> {
    (0..DEFAULT_PORTS)
        .map(|index| json!({"name":format!("Ethernet{index}"),"port_number":index}))
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
