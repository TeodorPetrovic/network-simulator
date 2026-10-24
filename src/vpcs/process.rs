use super::*;

pub(super) struct ManagedVpcs {
    pub(super) child: Child,
    pub(super) console: SocketAddr,
}

impl ManagedVpcs {
    /// Kill every process in this VPCS's own process group. VPCS forks its PC
    /// process behind the tracked console daemon, and killing only the daemon
    /// (as `kill_on_drop` does) leaves the fork running forever.
    pub(super) fn kill_group(&self) {
        if let Some(pid) = self.child.id() {
            kill_process_group(pid);
        }
    }
}

impl Drop for ManagedVpcs {
    fn drop(&mut self) {
        self.kill_group();
    }
}

/// SIGKILL `pid`, its process group, and every descendant. The real VPCS
/// daemon forks its PC process into a *new session*, so the process group alone
/// does not reach it; the descendants are collected before anything is killed
/// because a child is re-parented (and untraceable) once its parent dies.
pub(super) fn kill_process_group(pid: u32) {
    let mut victims = descendants_of(pid);
    victims.push(pid);
    // SAFETY: every PID was read from /proc as a descendant of the VPCS daemon
    // this runtime spawned, and killpg targets the group it created.
    unsafe {
        libc::killpg(pid as libc::pid_t, libc::SIGKILL);
        for victim in victims {
            libc::kill(victim as libc::pid_t, libc::SIGKILL);
        }
    }
}

/// All transitive children of `root`, read from `/proc/*/stat`.
pub(super) fn descendants_of(root: u32) -> Vec<u32> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|n| n.parse::<u32>().ok())
            else {
                continue;
            };
            let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
                continue;
            };
            // The command name may contain spaces and parentheses; the
            // parent PID is the second field after the last ')'.
            let Some(rest) = stat.rfind(')').map(|index| &stat[index + 1..]) else {
                continue;
            };
            if let Some(parent) = rest.split_whitespace().nth(1).and_then(|f| f.parse().ok()) {
                children.entry(parent).or_default().push(pid);
            }
        }
    }
    let mut found = Vec::new();
    let mut queue = vec![root];
    while let Some(parent) = queue.pop() {
        for child in children.get(&parent).into_iter().flatten() {
            found.push(*child);
            queue.push(*child);
        }
    }
    found
}

/// Kill leftover VPCS processes whose working directory lies inside
/// `state_dir`. Run once at controller start-up: a previous controller that
/// crashed cannot have stopped them, and this controller keeps no handle to
/// them. Returns the number of processes killed.
pub fn reap_orphaned_processes(binary: &Path, state_dir: &Path) -> usize {
    let Ok(state_dir) = state_dir.canonicalize() else {
        return 0;
    };
    let binary_name = binary.file_name().map(|name| name.to_owned());
    let Ok(entries) = fs::read_dir("/proc") else {
        return 0;
    };
    let mut killed = 0;
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        let exe_name = fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .and_then(|path| path.file_name().map(|name| name.to_owned()));
        if exe_name.is_none() || exe_name != binary_name {
            continue;
        }
        let Ok(cwd) = fs::read_link(format!("/proc/{pid}/cwd")) else {
            continue;
        };
        if cwd.starts_with(&state_dir) {
            // SAFETY: the PID belongs to a VPCS process confirmed above.
            if unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) } == 0 {
                killed += 1;
            }
        }
    }
    killed
}

pub(super) async fn connect_vpcs_console(address: SocketAddr) -> Result<TcpStream, RuntimeError> {
    timeout(Duration::from_secs(3), async {
        let mut stream = TcpStream::connect(address).await.map_err(|error| {
            RuntimeError::Unavailable(format!("Could not connect to the VPCS console: {error}"))
        })?;
        let mut handshake = [0_u8; TELNET_HANDSHAKE.len()];
        stream.read_exact(&mut handshake).await.map_err(|error| {
            RuntimeError::Unavailable(format!(
                "Could not negotiate the VPCS Telnet console: {error}"
            ))
        })?;
        if handshake != TELNET_HANDSHAKE {
            return Err(RuntimeError::Failed(
                "VPCS sent an unsupported Telnet console handshake".to_owned(),
            ));
        }
        stream.write_all(&TELNET_RESPONSE).await.map_err(|error| {
            RuntimeError::Unavailable(format!(
                "Could not answer the VPCS Telnet handshake: {error}"
            ))
        })?;
        Ok(stream)
    })
    .await
    .map_err(|_| RuntimeError::Unavailable("VPCS Telnet console handshake timed out".to_owned()))?
}

pub(super) async fn stop_vpcs_process(process: &mut ManagedVpcs) -> Result<(), RuntimeError> {
    if process
        .child
        .try_wait()
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not inspect VPCS process: {error}"))
        })?
        .is_some()
    {
        return Ok(());
    }

    // Upstream VPCS forks its PC process behind the console daemon. Its `quit`
    // command notifies that daemon to stop the forked process as well; killing
    // only the tracked daemon PID leaves the emulator child orphaned.
    if let Ok(mut console) = connect_vpcs_console(process.console).await {
        let _ = console.write_all(b"quit\r").await;
    }
    if wait_vpcs_process(&mut process.child).await? {
        return Ok(());
    }

    // Match VPCS's own daemon shutdown path if the console command could not
    // complete (for example, a console client already owns the connection).
    signal_vpcs_process(&process.child, libc::SIGUSR1)?;
    if wait_vpcs_process(&mut process.child).await? {
        return Ok(());
    }
    signal_vpcs_process(&process.child, libc::SIGTERM)?;
    if wait_vpcs_process(&mut process.child).await? {
        return Ok(());
    }
    Err(RuntimeError::Failed(
        "VPCS daemon did not stop after its console quit command and shutdown signals".to_owned(),
    ))
}

pub(super) async fn wait_vpcs_process(child: &mut Child) -> Result<bool, RuntimeError> {
    match timeout(Duration::from_secs(3), child.wait()).await {
        Ok(Ok(_)) => Ok(true),
        Ok(Err(error)) => Err(RuntimeError::Unavailable(format!(
            "Could not wait for the VPCS process: {error}"
        ))),
        Err(_) => Ok(false),
    }
}

pub(super) fn signal_vpcs_process(child: &Child, signal: libc::c_int) -> Result<(), RuntimeError> {
    let Some(pid) = child.id() else {
        return Ok(());
    };
    // SAFETY: The PID is owned and tracked by this child handle.
    if unsafe { libc::kill(pid as libc::pid_t, signal) } == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(())
    } else {
        Err(RuntimeError::Failed(format!(
            "Could not signal the VPCS daemon: {error}"
        )))
    }
}

pub(super) async fn kill_vpcs_startup_tree(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        kill_process_group(pid);
    }
    let _ = child.start_kill();
    let _ = child.wait().await;
}
