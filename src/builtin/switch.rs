use super::*;

impl BuiltinRuntime {
    pub async fn attach_switch_port(
        &self,
        node: &Value,
        endpoint: &Value,
    ) -> Result<(), RuntimeError> {
        let node_type = node["node_type"].as_str().unwrap_or_default();
        if node_type != "ethernet_switch" && node_type != "ethernet_hub" {
            return Ok(());
        }
        let node_id = node["node_id"].as_str().unwrap_or_default().to_owned();
        let port_number = endpoint["port_number"].as_u64().unwrap_or_default() as u16;
        let key = (node_id.clone(), port_number);
        if self.state.lock().await.active_ports.contains_key(&key) {
            return Ok(());
        }
        let adapter_port = port_from_endpoint(endpoint, "runtime_adapter_port")?;
        let runtime_port = port_from_endpoint(endpoint, "runtime_udp_port")?;
        let local = SocketAddr::from((Ipv4Addr::LOCALHOST, runtime_port));
        let socket = Arc::new(UdpSocket::bind(local).await.map_err(|error| {
            RuntimeError::Failed(format!(
                "Could not bind builtin adapter UDP port {runtime_port}: {error}"
            ))
        })?);
        let relay_address = SocketAddr::from((Ipv4Addr::LOCALHOST, adapter_port));
        let mode = if node_type == "ethernet_switch" {
            Some(switch_mode(&node["ports_mapping"][port_number as usize])?)
        } else {
            None
        };
        let forwarding = {
            let mut state = self.state.lock().await;
            let instance = state
                .forwarding
                .entry(node_id.clone())
                .or_insert_with(|| {
                    let modes = node["ports_mapping"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .enumerate()
                        .filter_map(|(index, port)| {
                            (node_type == "ethernet_switch")
                                .then(|| switch_mode(port).ok().map(|mode| (index as u16, mode)))
                                .flatten()
                        })
                        .collect();
                    Arc::new(Mutex::new(ForwardingNode {
                        node_type: node_type.to_owned(),
                        modes,
                        ports: HashMap::new(),
                        learned: HashMap::new(),
                    }))
                })
                .clone();
            instance.lock().await.ports.insert(
                port_number,
                RuntimePort {
                    socket: socket.clone(),
                    relay_address,
                    mode,
                },
            );
            instance
        };
        let task_socket = socket.clone();
        let task = tokio::spawn(async move {
            let mut buffer = vec![0u8; 65_536];
            let mut aging = tokio::time::interval(FDB_AGE_INTERVAL);
            aging.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    received = task_socket.recv_from(&mut buffer) => {
                        let Ok((length, _)) = received else { break; };
                        let frame = buffer[..length].to_vec();
                        for (target_socket, target_address, output) in
                            forward_frame(&forwarding, port_number, frame).await
                        {
                            let _ = target_socket.send_to(&output, target_address).await;
                        }
                    }
                    _ = aging.tick() => age_forwarding_table(&forwarding).await,
                }
            }
        });
        self.state
            .lock()
            .await
            .active_ports
            .insert(key, ActivePort { task });
        Ok(())
    }

    pub async fn detach_switch_port(&self, node_id: &str, port_number: u16) {
        let key = (node_id.to_owned(), port_number);
        let mut state = self.state.lock().await;
        if let Some(active) = state.active_ports.remove(&key) {
            active.task.abort();
        }
        let remove_forwarder = if let Some(node) = state.forwarding.get(node_id) {
            let mut node = node.lock().await;
            node.ports.remove(&port_number);
            node.learned
                .retain(|_, learned| learned.port != port_number);
            node.ports.is_empty()
        } else {
            false
        };
        if remove_forwarder {
            state.forwarding.remove(node_id);
        }
    }

    /// Apply updated switch VLAN settings without stopping the always-on
    /// builtin node or disturbing its attached link endpoints.
    pub async fn update_switch_configuration(&self, node: &Value) {
        if node["node_type"].as_str() != Some("ethernet_switch") {
            return;
        }
        let node_id = node["node_id"].as_str().unwrap_or_default();
        let instance = self.state.lock().await.forwarding.get(node_id).cloned();
        let Some(instance) = instance else {
            return;
        };
        let modes = node["ports_mapping"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(index, port)| switch_mode(port).ok().map(|mode| (index as u16, mode)))
            .collect::<HashMap<_, _>>();
        let mut forwarding = instance.lock().await;
        forwarding.modes = modes.clone();
        for (port_number, port) in &mut forwarding.ports {
            port.mode = modes.get(port_number).cloned();
        }
        forwarding.learned.clear();
    }

    pub async fn stop_node(&self, node_id: &str) {
        let mut state = self.state.lock().await;
        let keys = state
            .active_ports
            .keys()
            .filter(|(candidate, _)| candidate == node_id)
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            if let Some(active) = state.active_ports.remove(&key) {
                active.task.abort();
            }
        }
        state.forwarding.remove(node_id);
    }
}

pub(super) fn switch_mode(port: &Value) -> Result<SwitchPortMode, RuntimeError> {
    let vlan = port["vlan"].as_u64().unwrap_or(1) as u16;
    match port["type"].as_str().unwrap_or("access") {
        "access" => Ok(SwitchPortMode::Access { vlan }),
        "dot1q" => Ok(SwitchPortMode::Dot1q { native_vlan: vlan }),
        "qinq" => {
            let ethertype = port["ethertype"]
                .as_str()
                .unwrap_or("0x8100")
                .trim_start_matches("0x");
            let ethertype = u16::from_str_radix(ethertype, 16).map_err(|_| {
                RuntimeError::Failed("Switch port has an invalid QinQ ethertype".to_owned())
            })?;
            Ok(SwitchPortMode::QinQ { vlan, ethertype })
        }
        kind => Err(RuntimeError::Failed(format!(
            "Unsupported switch port type '{kind}'"
        ))),
    }
}

pub(super) async fn forward_frame(
    forwarding: &Arc<Mutex<ForwardingNode>>,
    source_port: u16,
    mut frame: Vec<u8>,
) -> Vec<(Arc<UdpSocket>, SocketAddr, Vec<u8>)> {
    let mut node = forwarding.lock().await;
    if node.node_type == "ethernet_hub" {
        return node
            .ports
            .iter()
            .filter(|(port, _)| **port != source_port)
            .map(|(_, target)| (target.socket.clone(), target.relay_address, frame.clone()))
            .collect();
    }
    let Some(source_mode) = node.modes.get(&source_port).cloned() else {
        return Vec::new();
    };
    let Some((vlan, stripped_frame)) = classify_ingress(&source_mode, &frame) else {
        return Vec::new();
    };
    frame = stripped_frame;
    if frame.len() < 14 {
        return Vec::new();
    }
    let source_mac: [u8; 6] = frame[6..12].try_into().unwrap_or([0; 6]);
    let destination_mac: [u8; 6] = frame[0..6].try_into().unwrap_or([0; 6]);
    let now = Instant::now();
    age_forwarding_node(&mut node, now);
    if !is_multicast(source_mac) && source_mac != [0; 6] {
        learn_address(&mut node, vlan, source_mac, source_port, now);
    }

    let known_destination = (!is_multicast(destination_mac))
        .then(|| {
            node.learned
                .get(&(vlan, destination_mac))
                .map(|entry| entry.port)
        })
        .flatten();
    node.ports
        .iter()
        .filter(|(port, target)| {
            **port != source_port
                && known_destination.map_or(true, |known| known == **port)
                && target
                    .mode
                    .as_ref()
                    .is_some_and(|mode| vlan_allowed(mode, vlan))
        })
        .filter_map(|(_, target)| {
            let output = classify_egress(target.mode.as_ref()?, vlan, &frame)?;
            Some((target.socket.clone(), target.relay_address, output))
        })
        .collect()
}

pub(super) async fn age_forwarding_table(forwarding: &Arc<Mutex<ForwardingNode>>) {
    age_forwarding_node(&mut *forwarding.lock().await, Instant::now());
}

pub(super) fn age_forwarding_node(node: &mut ForwardingNode, now: Instant) {
    node.learned
        .retain(|_, learned| now.saturating_duration_since(learned.learned_at) < FDB_AGE);
}

pub(super) fn learn_address(
    node: &mut ForwardingNode,
    vlan: u16,
    mac: [u8; 6],
    port: u16,
    learned_at: Instant,
) {
    let key = (vlan, mac);
    if !node.learned.contains_key(&key) && node.learned.len() >= MAX_FDB_ENTRIES {
        if let Some(oldest) = node
            .learned
            .iter()
            .min_by_key(|(_, learned)| learned.learned_at)
            .map(|(key, _)| *key)
        {
            node.learned.remove(&oldest);
        }
    }
    node.learned
        .insert(key, LearnedAddress { port, learned_at });
}

pub(super) fn is_multicast(mac: [u8; 6]) -> bool {
    mac[0] & 1 == 1
}

pub(super) fn vlan_allowed(mode: &SwitchPortMode, vlan: u16) -> bool {
    match mode {
        SwitchPortMode::Access { vlan: access_vlan }
        | SwitchPortMode::QinQ {
            vlan: access_vlan, ..
        } => *access_vlan == vlan,
        SwitchPortMode::Dot1q { .. } => true,
    }
}

pub(super) fn classify_ingress(mode: &SwitchPortMode, frame: &[u8]) -> Option<(u16, Vec<u8>)> {
    if frame.len() < 14 {
        return None;
    }
    let ethertype = u16::from_be_bytes([frame[12], frame[13]]);
    let tagged = matches!(ethertype, 0x8100 | 0x88a8 | 0x9100 | 0x9200) && frame.len() >= 18;
    let tagged_vlan = tagged.then(|| u16::from_be_bytes([frame[14], frame[15]]) & 0x0fff);
    match mode {
        SwitchPortMode::Access { vlan } => match tagged_vlan {
            Some(tag) if tag != *vlan => None,
            _ => Some((*vlan, frame.to_vec())),
        },
        SwitchPortMode::Dot1q { native_vlan } => {
            Some((tagged_vlan.unwrap_or(*native_vlan), frame.to_vec()))
        }
        SwitchPortMode::QinQ {
            vlan,
            ethertype: qinq_type,
        } => {
            if tagged && ethertype != *qinq_type {
                return None;
            }
            Some((tagged_vlan.unwrap_or(*vlan), frame.to_vec()))
        }
    }
}

pub(super) fn classify_egress(mode: &SwitchPortMode, vlan: u16, frame: &[u8]) -> Option<Vec<u8>> {
    let (kind, native_vlan, ethertype) = match mode {
        SwitchPortMode::Access { vlan } => ("access", *vlan, 0x8100),
        SwitchPortMode::Dot1q { native_vlan } => ("dot1q", *native_vlan, 0x8100),
        SwitchPortMode::QinQ { vlan, ethertype } => ("qinq", *vlan, *ethertype),
    };
    let frame = remove_outer_vlan(frame);
    if kind == "access" || (kind == "dot1q" && vlan == native_vlan) {
        return Some(frame);
    }
    Some(insert_vlan(&frame, ethertype, vlan))
}

pub(super) fn remove_outer_vlan(frame: &[u8]) -> Vec<u8> {
    if frame.len() >= 18
        && matches!(
            u16::from_be_bytes([frame[12], frame[13]]),
            0x8100 | 0x88a8 | 0x9100 | 0x9200
        )
    {
        let mut output = Vec::with_capacity(frame.len() - 4);
        output.extend_from_slice(&frame[..12]);
        output.extend_from_slice(&frame[16..]);
        output
    } else {
        frame.to_vec()
    }
}

pub(super) fn insert_vlan(frame: &[u8], ethertype: u16, vlan: u16) -> Vec<u8> {
    if frame.len() < 14 {
        return frame.to_vec();
    }
    let mut output = Vec::with_capacity(frame.len() + 4);
    output.extend_from_slice(&frame[..12]);
    output.extend_from_slice(&ethertype.to_be_bytes());
    output.extend_from_slice(&(vlan & 0x0fff).to_be_bytes());
    output.extend_from_slice(&frame[12..]);
    output
}

pub(super) fn default_switch_ports() -> Vec<Value> {
    (0..DEFAULT_PORTS)
        .map(|index| json!({"name":format!("Ethernet{index}"),"port_number":index,"type":"access","vlan":1,"ethertype":"0x8100"}))
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
