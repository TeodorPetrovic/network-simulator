use std::{fs, path::PathBuf};

use crate::runtime::RuntimeError;

/// TAPs created by this controller are edge ports: no bridge can exist behind
/// them. Put the port directly into forwarding so an enabled host bridge's STP
/// listening/learning timers don't discard the guest's first ARP probes.
pub(super) async fn enable_edge_forwarding(tap: Option<&str>) -> Result<(), RuntimeError> {
    let Some(tap) = tap else {
        return Ok(());
    };
    // Forcing the state needs CAP_NET_ADMIN, which the controller usually lacks
    // (only uBridge holds it). When that fails, fall back to waiting for the
    // kernel to move the port to forwarding on its own: libvirt's bridges use
    // an STP delay of 0, so this is immediate there.
    let forced = match tokio::process::Command::new("bridge")
        .args(["link", "set", "dev", tap, "state", "3"])
        .output()
        .await
    {
        Ok(output) => output.status.success(),
        Err(_) => false,
    };
    let state_file = PathBuf::from(format!("/sys/class/net/{tap}/brport/state"));
    let attempts = if forced { 1 } else { 100 };
    let mut last = String::new();
    for attempt in 0..attempts {
        last = fs::read_to_string(&state_file)
            .map_err(|error| {
                RuntimeError::Failed(format!("Could not inspect bridge port {tap}: {error}"))
            })?
            .trim()
            .to_owned();
        if last == "3" {
            return Ok(());
        }
        if attempt + 1 < attempts {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
    Err(RuntimeError::Unavailable(format!(
        "Controller TAP {tap} did not enter forwarding state (state {last}); the host bridge has STP enabled with a forwarding delay and the controller cannot force the port state without CAP_NET_ADMIN"
    )))
}
