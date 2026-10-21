//! Process ownership and orphan cleanup for Dynamips hypervisors.

use std::{collections::HashMap, fs, path::Path};

/// All transitive children of a process, read from Linux `/proc/*/stat`.
fn dynamips_descendants_of(root: u32) -> Vec<u32> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else {
                continue;
            };
            let Some(rest) = stat.rfind(')').map(|index| &stat[index + 1..]) else {
                continue;
            };
            if let Some(parent) = rest
                .split_whitespace()
                .nth(1)
                .and_then(|field| field.parse::<u32>().ok())
            {
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

/// Kill a Dynamips process group and descendants that may have detached from it.
pub(super) fn kill_dynamips_process_tree(pid: u32) {
    if pid == 0 {
        return;
    }
    let mut victims = dynamips_descendants_of(pid);
    victims.push(pid);
    // SAFETY: callers only pass a child PID from a managed Dynamips spawn or a
    // `/proc` entry verified as a Dynamips process inside the runtime state dir.
    unsafe {
        libc::killpg(pid as libc::pid_t, libc::SIGKILL);
        for victim in victims {
            libc::kill(victim as libc::pid_t, libc::SIGKILL);
        }
    }
}

/// Reap hypervisors left by a crashed controller. Their cwd is the per-node
/// runtime directory, so repository files and unrelated Dynamips processes are
/// outside the match.
pub(super) fn reap_orphaned_dynamips_processes(binary: &Path, state_dir: &Path) -> usize {
    let Ok(state_dir) = state_dir.canonicalize() else {
        return 0;
    };
    // `/proc/<pid>/exe` resolves symlinks and gains a " (deleted)" suffix
    // after the binary is replaced on disk, so accept either spelling.
    let binary_names = [Some(binary.to_path_buf()), binary.canonicalize().ok()]
        .into_iter()
        .flatten()
        .filter_map(|path| path.file_name().map(|name| name.to_owned()))
        .collect::<Vec<_>>();
    if binary_names.is_empty() {
        return 0;
    }
    let Ok(entries) = fs::read_dir("/proc") else {
        return 0;
    };
    let mut killed = 0;
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let executable_name = fs::read_link(format!("/proc/{pid}/exe"))
            .ok()
            .and_then(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.trim_end_matches(" (deleted)").to_owned())
            });
        if !executable_name.is_some_and(|name| {
            binary_names
                .iter()
                .any(|binary_name| binary_name.to_str() == Some(name.as_str()))
        }) {
            continue;
        }
        let Ok(cwd) = fs::read_link(format!("/proc/{pid}/cwd")) else {
            continue;
        };
        if cwd.starts_with(&state_dir) {
            kill_dynamips_process_tree(pid);
            killed += 1;
        }
    }
    killed
}
