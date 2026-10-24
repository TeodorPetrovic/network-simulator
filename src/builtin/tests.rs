use super::*;

use super::*;

pub(super) fn ethernet_frame(destination: [u8; 6], source: [u8; 6]) -> Vec<u8> {
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
    let tagged = classify_egress(&SwitchPortMode::Dot1q { native_vlan: 1 }, 42, &frame).unwrap();
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
        validate_host_interface_policy("cloud", "ens33", "virbr0", &no_allowlist, &routes, false,)
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
        validate_host_interface_policy("cloud", "lo", "virbr0", &allowed, &routes, true,).is_ok()
    );
    assert!(
        validate_host_interface_policy("nat", "virbr0", "virbr0", &no_allowlist, &routes, false,)
            .is_ok()
    );
    assert!(
        validate_host_interface_policy("nat", "ens34", "virbr0", &no_allowlist, &routes, false,)
            .is_err()
    );
    assert!(
        validate_host_interface_policy("nat", "ens34", "virbr0", &allowed, &routes, false,).is_ok()
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
    let socket = || async { Arc::new(UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap()) };
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

#[path = "tests/runtime.rs"]
mod runtime;
