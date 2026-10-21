use std::{fs, path::PathBuf};

use crate::runtime::RuntimeError;

/// TAPs created by this controller are edge ports: no bridge can exist behind
/// them. Put the port directly into forwarding so an enabled host bridge's STP
/// listening/learning timers don't discard the guest's first ARP probes.
pub(super) async fn enable_edge_forwarding(tap: Option<&str>) -> Result<(), RuntimeError> {
    let Some(tap) = tap else {
        return Ok(());
    };
    let output = tokio::process::Command::new("bridge")
        .args(["link", "set", "dev", tap, "state", "3"])
        .output()
        .await
        .map_err(|error| {
            RuntimeError::Unavailable(format!(
                "Could not set controller TAP {tap} to edge forwarding: {error}"
            ))
        })?;
    if !output.status.success() {
        return Err(RuntimeError::Unavailable(format!(
            "Could not set controller TAP {tap} to edge forwarding: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let state_file = PathBuf::from(format!("/sys/class/net/{tap}/brport/state"));
    let state = fs::read_to_string(&state_file).map_err(|error| {
        RuntimeError::Failed(format!("Could not inspect bridge port {tap}: {error}"))
    })?;
    if state.trim() != "3" {
        return Err(RuntimeError::Unavailable(format!(
            "Controller TAP {tap} did not enter forwarding state (state {})",
            state.trim()
        )));
    }
    Ok(())
}
