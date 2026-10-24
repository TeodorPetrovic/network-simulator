use super::*;
use crate::ubridge::UbridgeRuntime;
use std::{env, path::PathBuf, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};
use uuid::Uuid;

pub(super) fn strip_ansi(input: &[u8]) -> String {
    let mut output = String::new();
    let mut index = 0;
    while index < input.len() {
        if input[index] == 0x1b {
            index += 1;
            if input.get(index) == Some(&b'[') {
                index += 1;
                while index < input.len() {
                    let byte = input[index];
                    index += 1;
                    if (0x40..=0x7e).contains(&byte) {
                        break;
                    }
                }
            } else if index < input.len() {
                index += 1;
            }
        } else {
            let byte = input[index];
            index += 1;
            if byte == b'\r' || byte == b'\n' || byte.is_ascii_graphic() || byte == b' ' {
                output.push(byte as char);
            }
        }
    }
    output
}

pub(super) async fn read_until(
    serial: &mut BufReader<UnixStream>,
    marker: &str,
    duration: Duration,
) -> Result<String, String> {
    let mut bytes = Vec::new();
    timeout(duration, async {
        loop {
            let (read, found) = {
                let available = serial.fill_buf().await.map_err(|error| error.to_string())?;
                if available.is_empty() {
                    return Err("QEMU serial socket closed".to_owned());
                }
                let previous_len = bytes.len();
                bytes.extend_from_slice(available);
                let marker_end = bytes
                    .windows(marker.len())
                    .position(|window| window == marker.as_bytes())
                    .map(|position| position + marker.len());
                match marker_end {
                    Some(end) => (end.saturating_sub(previous_len).min(available.len()), true),
                    None => (available.len(), false),
                }
            };
            serial.consume(read);
            if found {
                return Ok::<String, String>(strip_ansi(&bytes));
            }
        }
    })
    .await
    .map_err(|_| {
        format!(
            "timed out waiting for {marker:?}; output: {}",
            strip_ansi(&bytes)
        )
    })?
}

pub(super) async fn routeros_command(
    serial: &mut BufReader<UnixStream>,
    command: &str,
) -> Result<String, String> {
    serial
        .write_all(command.as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    serial
        .write_all(b"\r")
        .await
        .map_err(|error| error.to_string())?;
    read_until(serial, &format!("{command}\r\n"), Duration::from_secs(5)).await?;
    read_until(serial, "[admin@CHR] >", Duration::from_secs(15)).await
}

pub(super) fn qemu_test_mac(node_id: &str, adapter_number: u32) -> Result<String, String> {
    let id = Uuid::parse_str(node_id).map_err(|error| error.to_string())?;
    let bytes = id.as_bytes();
    Ok(format!(
        "52:54:00:{:02x}:{:02x}:{:02x}",
        bytes[13] ^ ((adapter_number >> 16) as u8),
        bytes[14] ^ ((adapter_number >> 8) as u8),
        bytes[15] ^ (adapter_number as u8)
    ))
}

pub(super) fn qemu_interface_for_mac(output: &str, mac: &str) -> Option<String> {
    let mut name = None;
    for line in output.lines() {
        if let Some((_, value)) = line.split_once("name=") {
            let value = value.trim_start_matches('"');
            let end = value
                .find('"')
                .or_else(|| value.find(char::is_whitespace))
                .unwrap_or(value.len());
            name = Some(value[..end].to_owned());
        }
        if line
            .to_ascii_lowercase()
            .contains(&mac.to_ascii_lowercase())
        {
            return name;
        }
    }
    None
}

pub(super) async fn live_chr_environment() -> (
    TempDir,
    DockerRuntime,
    QemuRuntime,
    VpcsRuntime,
    UbridgeRuntime,
    axum::Router,
) {
    let directory = TempDir::new().expect("temporary CHR test state");
    let store = Store::connect(
        directory
            .path()
            .join("controller.sqlite")
            .to_str()
            .expect("UTF-8 test database path"),
    )
    .await
    .expect("test database connects");
    sqlx::query("UPDATE auth_config SET auth_enabled = 0 WHERE singleton = 1")
        .execute(store.pool())
        .await
        .expect("keep test authentication disabled");
    sqlx::query(
        "INSERT INTO auth_users (user_id, username, full_name, password_hash, is_superadmin) VALUES (?, 'live-test-admin', 'Live test administrator', 'unused', 1)",
    )
    .bind(Uuid::new_v4().to_string())
    .execute(store.pool())
    .await
    .expect("seed loopback superadmin principal for the auth middleware");
    let qemu = QemuRuntime::new();
    let docker = DockerRuntime::new();
    let vpcs_binary = env::var_os("NETWORK_SIMULATOR_TEST_VPCS")
        .expect("NETWORK_SIMULATOR_TEST_VPCS names upstream VPCS");
    let ubridge_binary = env::var_os("NETWORK_SIMULATOR_TEST_UBRIDGE")
        .expect("NETWORK_SIMULATOR_TEST_UBRIDGE names capable upstream uBridge");
    let vpcs = VpcsRuntime::with_paths(vpcs_binary, directory.path().join("vpcs-state"));
    let ubridge = crate::ubridge::UbridgeRuntime::with_paths(
        PathBuf::from(ubridge_binary),
        directory.path().join("ubridge.sock"),
    );
    let app = router(AppState::with_runtimes_and_ubridge(
        store,
        docker.clone(),
        qemu.clone(),
        vpcs.clone(),
        DynamipsRuntime::new(),
        ubridge.clone(),
        crate::marketplace::MarketplaceCatalog::with_directory(directory.path().join("appliances")),
    ));
    (directory, docker, qemu, vpcs, ubridge, app)
}

pub(super) async fn delete_live_links(
    app: &axum::Router,
    project_id: &str,
    link_ids: &[String],
) -> Result<(), String> {
    for link_id in link_ids {
        let (status, body) = request(
            app,
            Method::DELETE,
            format!("/v3/projects/{project_id}/links/{link_id}"),
            None,
        )
        .await;
        if status != StatusCode::NO_CONTENT {
            return Err(format!("live link deletion returned {status}: {body}"));
        }
    }
    Ok(())
}

pub(super) async fn cleanup_live_project(
    app: &axum::Router,
    project_id: Option<String>,
) -> Result<(), String> {
    if let Some(project_id) = project_id {
        let (status, body) = request(
            app,
            Method::DELETE,
            format!("/v3/projects/{project_id}"),
            None,
        )
        .await;
        (status == StatusCode::NO_CONTENT)
            .then_some(())
            .ok_or_else(|| format!("temporary CHR project cleanup returned {status}: {body}"))
    } else {
        Ok(())
    }
}
