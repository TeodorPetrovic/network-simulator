use super::*;

impl DynamipsRuntime {
    pub async fn set_idle_pc(&self, vm: &DynamipsVm, idle_pc: &str) -> Result<(), RuntimeError> {
        if !is_hex_pointer(idle_pc) {
            return Err(RuntimeError::Failed(
                "Idle-PC must be a hexadecimal pointer".to_owned(),
            ));
        }
        let _guard = self.operation_lock.lock().await;
        if self.hypervisor_status(vm.hypervisor_port, &vm.name).await? != "stopped" {
            return Err(RuntimeError::Failed(
                "Stop the Dynamips router before changing its Idle-PC value".to_owned(),
            ));
        }
        self.send(
            vm.hypervisor_port,
            &format!("vm set_idle_pc {} {idle_pc}", quote(&vm.name)),
        )
        .await?;
        Ok(())
    }

    pub async fn idlepc_proposals(&self, vm: &DynamipsVm) -> Result<Vec<String>, RuntimeError> {
        let _guard = self.operation_lock.lock().await;
        let was_auto_started =
            self.hypervisor_status(vm.hypervisor_port, &vm.name).await? != "started";
        let probe = async {
            if was_auto_started {
                self.start_locked(vm).await?;
                sleep(Duration::from_secs(20)).await;
            }
            // Dynamips only searches for candidates while no online Idle-PC
            // value is active. Restore the configured value after the probe.
            self.send(
                vm.hypervisor_port,
                &format!("vm set_idle_pc_online {} 0 0x0", quote(&vm.name)),
            )
            .await?;
            self.send_with_timeout(
                vm.hypervisor_port,
                &format!("vm get_idle_pc_prop {} 0", quote(&vm.name)),
                IDLE_PC_PROPOSAL_TIMEOUT,
            )
            .await
        }
        .await;
        let stopped = if was_auto_started {
            match self.stop_locked(vm).await {
                Ok(_) => self
                    .send(
                        vm.hypervisor_port,
                        &format!(
                            "vm set_idle_pc {} {}",
                            quote(&vm.name),
                            vm.idle_pc.as_deref().unwrap_or("0x0")
                        ),
                    )
                    .await
                    .map(|_| ()),
                Err(error) => Err(error),
            }
        } else {
            self.send(
                vm.hypervisor_port,
                &format!(
                    "vm set_idle_pc_online {} 0 {}",
                    quote(&vm.name),
                    vm.idle_pc.as_deref().unwrap_or("0x0")
                ),
            )
            .await
            .map(|_| ())
        };
        match (probe, stopped) {
            (Ok(proposals), Ok(())) => Ok(proposals),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }

    pub async fn auto_idlepc(&self, vm: &DynamipsVm) -> Result<String, RuntimeError> {
        let _guard = self.operation_lock.lock().await;
        let was_auto_started =
            self.hypervisor_status(vm.hypervisor_port, &vm.name).await? != "started";
        let probe = async {
            if was_auto_started {
                self.start_locked(vm).await?;
                sleep(Duration::from_secs(20)).await;
            }
            self.send(
                vm.hypervisor_port,
                &format!("vm set_idle_pc_online {} 0 0x0", quote(&vm.name)),
            )
            .await?;
            let proposals = self
                .send_with_timeout(
                    vm.hypervisor_port,
                    &format!("vm get_idle_pc_prop {} 0", quote(&vm.name)),
                    IDLE_PC_PROPOSAL_TIMEOUT,
                )
                .await?;
            let process_id = self
                .routers
                .lock()
                .await
                .get(&vm.node_id)
                .and_then(|router| router.child.id());
            let ticks_per_second = process_ticks_per_second();
            let mut best: Option<(String, f64)> = None;
            let mut applied = None;
            for proposal in proposals {
                let Some(candidate) = idlepc_candidate(&proposal) else {
                    continue;
                };
                self.send(
                    vm.hypervisor_port,
                    &format!("vm set_idle_pc_online {} 0 {candidate}", quote(&vm.name)),
                )
                .await?;
                applied = Some(candidate.to_owned());
                let process_sample =
                    process_id
                        .zip(ticks_per_second)
                        .and_then(|(pid, ticks_per_second)| {
                            read_process_cpu_ticks(pid).map(|ticks| (pid, ticks_per_second, ticks))
                        });
                let cpu_seconds_before = self.cpu_usage(vm.hypervisor_port, &vm.name).await?;
                let started = tokio::time::Instant::now();
                sleep(IDLE_PC_CPU_SAMPLE_TIME).await;
                let process_cpu_percent =
                    process_sample.and_then(|(pid, ticks_per_second, before)| {
                        read_process_cpu_ticks(pid).map(|after| {
                            process_cpu_rate(before, after, started.elapsed(), ticks_per_second)
                        })
                    });
                let cpu_percent = if let Some(cpu_percent) = process_cpu_percent {
                    cpu_percent
                } else {
                    let after = self.cpu_usage(vm.hypervisor_port, &vm.name).await?;
                    idlepc_cpu_rate(cpu_seconds_before, after, started.elapsed())
                };
                tracing::debug!(
                    node_id = %vm.node_id,
                    idle_pc = candidate,
                    cpu_percent,
                    "Validated Dynamips Idle-PC candidate"
                );
                if cpu_percent < IDLE_PC_CPU_TARGET_PERCENT {
                    return Ok(candidate.to_owned());
                }
                if best
                    .as_ref()
                    .is_none_or(|(_, best_percent)| cpu_percent < *best_percent)
                {
                    best = Some((candidate.to_owned(), cpu_percent));
                }
            }
            // GNS3 accepts the first candidate below 70%. Prefer one that
            // reaches the idle target, otherwise keep GNS3's acceptance rule
            // but return the lowest measured candidate instead of the first.
            match best {
                Some((candidate, cpu_percent)) if cpu_percent < GNS3_IDLE_PC_CPU_LIMIT_PERCENT => {
                    tracing::warn!(
                        node_id = %vm.node_id,
                        idle_pc = %candidate,
                        cpu_percent,
                        "No Dynamips Idle-PC candidate reached the idle CPU target; using the lowest"
                    );
                    if applied.as_deref() != Some(candidate.as_str()) {
                        self.send(
                            vm.hypervisor_port,
                            &format!("vm set_idle_pc_online {} 0 {candidate}", quote(&vm.name)),
                        )
                        .await?;
                    }
                    Ok(candidate)
                }
                _ => Err(RuntimeError::Failed(
                    "Sorry, no idle-pc value was suitable".to_owned(),
                )),
            }
        }
        .await;
        let stopped = if was_auto_started {
            self.stop_locked(vm).await.map(|_| ())
        } else {
            if probe.is_err() {
                // Leave a running router on the Idle-PC it had before probing.
                let _ = self
                    .send(
                        vm.hypervisor_port,
                        &format!(
                            "vm set_idle_pc_online {} 0 {}",
                            quote(&vm.name),
                            vm.idle_pc.as_deref().unwrap_or("0x0")
                        ),
                    )
                    .await;
            }
            Ok(())
        };
        match (probe, stopped) {
            (Ok(idlepc), Ok(())) => Ok(idlepc),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }
}

pub(super) fn is_hex_pointer(value: &str) -> bool {
    value.strip_prefix("0x").is_some_and(|digits| {
        !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

pub(super) fn idlepc_candidate(proposal: &str) -> Option<&str> {
    let candidate = proposal.split_whitespace().next()?;
    (candidate.len() == 10
        && candidate.starts_with("0x")
        && candidate[2..].bytes().all(|byte| byte.is_ascii_hexdigit()))
    .then_some(candidate)
}

pub(super) fn idlepc_cpu_rate(before: i64, after: i64, elapsed: Duration) -> f64 {
    let elapsed_seconds = elapsed.as_secs_f64().max(0.001);
    (after.saturating_sub(before).unsigned_abs() as f64 * 100.0 / elapsed_seconds).min(100.0)
}

pub(super) fn process_ticks_per_second() -> Option<u64> {
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    u64::try_from(ticks).ok().filter(|ticks| *ticks > 0)
}

pub(super) fn read_process_cpu_ticks(pid: u32) -> Option<u64> {
    let tasks = fs::read_dir(format!("/proc/{pid}/task")).ok()?;
    let mut total_ticks = 0_u64;
    let mut found_thread = false;
    for task in tasks.flatten() {
        let Ok(stat) = fs::read_to_string(task.path().join("stat")) else {
            continue;
        };
        let Some(ticks) = process_thread_cpu_ticks(&stat) else {
            continue;
        };
        total_ticks = total_ticks.saturating_add(ticks);
        found_thread = true;
    }
    found_thread.then_some(total_ticks)
}

pub(super) fn process_thread_cpu_ticks(stat: &str) -> Option<u64> {
    let fields = stat
        .rsplit_once(')')?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    let user_ticks = fields.get(11)?.parse::<u64>().ok()?;
    let system_ticks = fields.get(12)?.parse::<u64>().ok()?;
    Some(user_ticks.saturating_add(system_ticks))
}

pub(super) fn process_cpu_rate(
    before: u64,
    after: u64,
    elapsed: Duration,
    ticks_per_second: u64,
) -> f64 {
    if ticks_per_second == 0 {
        return f64::INFINITY;
    }
    let elapsed_seconds = elapsed.as_secs_f64().max(0.001);
    after.saturating_sub(before) as f64 * 100.0 / ticks_per_second as f64 / elapsed_seconds
}
