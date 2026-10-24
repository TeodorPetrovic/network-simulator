use std::{collections::HashMap, net::SocketAddr, sync::Arc, time::Instant};
use tokio::{net::UdpSocket, sync::Mutex, task::JoinHandle};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum SwitchPortMode {
    Access { vlan: u16 },
    Dot1q { native_vlan: u16 },
    QinQ { vlan: u16, ethertype: u16 },
}

#[derive(Clone)]
pub(super) struct RuntimePort {
    pub(super) socket: Arc<UdpSocket>,
    pub(super) relay_address: SocketAddr,
    pub(super) mode: Option<SwitchPortMode>,
}

#[derive(Clone)]
pub(super) struct LearnedAddress {
    pub(super) port: u16,
    pub(super) learned_at: Instant,
}

pub(super) struct ForwardingNode {
    pub(super) node_type: String,
    pub(super) modes: HashMap<u16, SwitchPortMode>,
    pub(super) ports: HashMap<u16, RuntimePort>,
    pub(super) learned: HashMap<(u16, [u8; 6]), LearnedAddress>,
}

pub(super) struct ActivePort {
    pub(super) task: JoinHandle<()>,
}

#[derive(Default)]
pub(super) struct BuiltinState {
    pub(super) forwarding: HashMap<String, Arc<Mutex<ForwardingNode>>>,
    pub(super) active_ports: HashMap<(String, u16), ActivePort>,
}

/// Runtime state for builtins. These nodes do not start a separate VM process;
/// the only long-running work is the UDP forwarding task for each switch/hub
/// port that is attached to a topology link.
#[derive(Clone, Default)]
pub struct BuiltinRuntime {
    pub(super) state: Arc<Mutex<BuiltinState>>,
}
