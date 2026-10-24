use super::*;

pub(super) async fn health(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(state.store.pool())
        .await?;
    Ok(Json(json!({ "status": "ok" })))
}

pub(super) async fn version() -> Json<Value> {
    Json(json!({
        "controller_host": null,
        "version": "3.1.0.dev7",
        "local": true
    }))
}

pub(super) async fn runtimes(State(state): State<AppState>) -> Json<Value> {
    Json(json!({
        "docker": state.runtime.info().await,
        "qemu": state.qemu.info().await,
        "vpcs": state.vpcs.info().await,
        "dynamips": state.dynamips.info().await,
        "ubridge": state.ubridge.info().await
    }))
}

pub(super) async fn resource_monitor() -> Json<Value> {
    let cpu_before = proc_cpu_times();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let cpu_after = proc_cpu_times();
    let cpu_usage_percent = cpu_before.zip(cpu_after).and_then(|(before, after)| {
        let total = after.0.checked_sub(before.0)?;
        let idle = after.1.checked_sub(before.1)?;
        (total > 0)
            .then(|| ((total.saturating_sub(idle) as f64 / total as f64) * 100.0).clamp(0.0, 100.0))
    });
    let memory = proc_memory_bytes();
    let controller = proc_controller_stats();
    let load_average = std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|contents| {
            let values = contents
                .split_whitespace()
                .take(3)
                .map(str::parse::<f64>)
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            (values.len() == 3).then_some(values)
        });
    Json(json!({
        "sampled_at_unix_ms": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_millis()),
        "host": {
            "cpu_usage_percent": cpu_usage_percent,
            "cpu_count": proc_cpu_count(),
            "memory_used_bytes": memory.map(|(used, _)| used),
            "memory_total_bytes": memory.map(|(_, total)| total),
            "load_average": load_average
        },
        "controller": controller
    }))
}

pub(super) fn proc_cpu_times() -> Option<(u64, u64)> {
    let contents = std::fs::read_to_string("/proc/stat").ok()?;
    let mut values = contents
        .lines()
        .find(|line| line.starts_with("cpu "))?
        .split_whitespace()
        .skip(1)
        .map(str::parse::<u64>);
    let times = values.by_ref().collect::<Result<Vec<_>, _>>().ok()?;
    let total = times.iter().copied().sum();
    let idle = times
        .get(3)
        .copied()
        .unwrap_or_default()
        .saturating_add(times.get(4).copied().unwrap_or_default());
    Some((total, idle))
}

pub(super) fn proc_cpu_count() -> Option<usize> {
    let contents = std::fs::read_to_string("/proc/stat").ok()?;
    let count = contents
        .lines()
        .filter(|line| {
            line.strip_prefix("cpu").is_some_and(|suffix| {
                suffix
                    .chars()
                    .next()
                    .is_some_and(|digit| digit.is_ascii_digit())
            })
        })
        .count();
    (count > 0).then_some(count)
}

pub(super) fn proc_memory_bytes() -> Option<(u64, u64)> {
    let contents = std::fs::read_to_string("/proc/meminfo").ok()?;
    let values = contents
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            let kibibytes = value.split_whitespace().next()?.parse::<u64>().ok()?;
            Some((key, kibibytes.saturating_mul(1024)))
        })
        .collect::<std::collections::HashMap<_, _>>();
    let total = *values.get("MemTotal")?;
    let available = *values.get("MemAvailable")?;
    Some((total.saturating_sub(available), total))
}

pub(super) fn proc_controller_stats() -> Value {
    let contents = std::fs::read_to_string("/proc/self/status").ok();
    let mut rss_bytes = None;
    let mut thread_count = None;
    if let Some(contents) = contents {
        for line in contents.lines() {
            if let Some(value) = line.strip_prefix("VmRSS:") {
                rss_bytes = value
                    .split_whitespace()
                    .next()
                    .and_then(|value| value.parse::<u64>().ok())
                    .map(|kibibytes| kibibytes.saturating_mul(1024));
            } else if let Some(value) = line.strip_prefix("Threads:") {
                thread_count = value.trim().parse::<u64>().ok();
            }
        }
    }
    json!({ "memory_rss_bytes": rss_bytes, "thread_count": thread_count })
}
