//! Companion tools the desktop app can detect, download and launch: Wireshark (packet capture
//! viewer) and Motty (native SSH/VNC/RDP client). The simulator runtime stays on the server, so
//! neither tool needs a local network driver: captures arrive as a pcap stream over HTTP.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncWriteExt;

const MOTTY_REPO: &str = "t8y2/motty";

#[derive(Serialize, Clone)]
pub struct ToolStatus {
    id: &'static str,
    name: &'static str,
    installed: bool,
    path: Option<String>,
    /// Whether this build can download the tool for the current OS.
    can_install: bool,
    note: String,
}

#[derive(Serialize, Clone)]
struct Progress<'a> {
    id: &'a str,
    stage: &'a str,
    done: u64,
    total: u64,
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) { vec![".exe".into(), ".cmd".into(), "".into()] } else { vec!["".into()] };
    std::env::split_paths(&path).find_map(|dir| exts.iter().map(|e| dir.join(format!("{name}{e}"))).find(|p| p.is_file()))
}

fn tools_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(|e| e.to_string())?.join("tools"))
}

fn motty_binary(app: &AppHandle) -> Option<PathBuf> {
    let exe = if cfg!(windows) { "motty.exe" } else { "motty" };
    let managed = tools_dir(app).ok()?.join("motty").join(exe);
    if managed.is_file() { return Some(managed); }
    // MOTTY_PATH lets a dev build (e.g. motty-dev.exe) stand in for the released one.
    if let Some(custom) = std::env::var_os("MOTTY_PATH").map(PathBuf::from).filter(|p| p.is_file()) { return Some(custom); }
    which("motty").or_else(|| which("motty-dev"))
}

/// Arguments for Motty's `vnc <host> <port>` command line.
fn vnc_args(host: &str, port: u16) -> Result<Vec<String>, String> {
    let valid_host = !host.is_empty()
        && host.len() <= 253
        && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '_' | '[' | ']'));
    if !valid_host || host.starts_with('-') { return Err("Invalid VNC host".into()); }
    if port == 0 { return Err("Invalid VNC port".into()); }
    Ok(vec!["vnc".into(), host.into(), port.to_string()])
}

/// Opens a VNC session in Motty (`motty vnc <host> <port>`).
#[tauri::command]
pub fn launch_motty_vnc(app: AppHandle, host: String, port: u16) -> Result<(), String> {
    let binary = motty_binary(&app).ok_or("Motty is not installed")?;
    let args = vnc_args(&host, port)?;
    std::process::Command::new(binary).args(args).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

fn wireshark_binary() -> Option<PathBuf> {
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
            if let Some(root) = std::env::var_os(var) {
                let candidate = Path::new(&root).join("Wireshark").join("Wireshark.exe");
                if candidate.is_file() { return Some(candidate); }
            }
        }
    }
    which("wireshark")
}

fn motty_asset(version: &str) -> Option<String> {
    let (os, ext) = match std::env::consts::OS { "windows" => ("windows", "zip"), "linux" => ("linux", "tar.gz"), _ => return None };
    let arch = match std::env::consts::ARCH { "x86_64" => "amd64", "aarch64" => "arm64", _ => return None };
    Some(format!("motty-{version}-{os}-{arch}.{ext}"))
}

#[tauri::command]
pub fn tool_status(app: AppHandle) -> Vec<ToolStatus> {
    let wireshark = wireshark_binary();
    let motty = motty_binary(&app);
    vec![
        ToolStatus {
            id: "wireshark", name: "Wireshark", installed: wireshark.is_some(),
            path: wireshark.map(|p| p.display().to_string()),
            can_install: cfg!(windows),
            note: if cfg!(windows) { "Opens live link captures streamed from the server. Npcap is not required.".into() }
                  else { "Install it with your package manager (for example: sudo apt install wireshark).".into() },
        },
        ToolStatus {
            id: "motty", name: "Motty", installed: motty.is_some(),
            path: motty.map(|p| p.display().to_string()),
            can_install: motty_asset("0").is_some(),
            note: "Native SSH, VNC and RDP client. Choose it as the console or VNC viewer in Preferences.".into(),
        },
    ]
}

fn emit(app: &AppHandle, id: &str, stage: &str, done: u64, total: u64) {
    let _ = app.emit("tool-progress", Progress { id, stage, done, total });
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder().user_agent("network-studio-desktop").build().map_err(|e| e.to_string())
}

async fn download(app: &AppHandle, id: &str, url: &str, dest: &Path) -> Result<String, String> {
    let response = client()?.get(url).send().await.and_then(|r| r.error_for_status()).map_err(|e| format!("download failed: {e}"))?;
    let total = response.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(dest).await.map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut done = 0u64;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("download failed: {e}"))?;
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        done += chunk.len() as u64;
        emit(app, id, "download", done, total);
    }
    file.flush().await.map_err(|e| e.to_string())?;
    Ok(format!("{:x}", hasher.finalize()))
}

async fn text(url: &str) -> Result<String, String> {
    client()?.get(url).send().await.and_then(|r| r.error_for_status()).map_err(|e| format!("request failed: {e}"))?
        .text().await.map_err(|e| e.to_string())
}

/// Finds `<hex>  <name>` (sha256sum) or `SHA256(<name>)=<hex>` (Wireshark) in a checksum listing.
fn expected_hash(listing: &str, name: &str) -> Option<String> {
    listing.lines().find_map(|line| {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(&format!("SHA256({name})=")) { return Some(rest.trim().to_lowercase()); }
        let mut parts = line.split_whitespace();
        let (hash, file) = (parts.next()?, parts.next()?);
        (file.trim_start_matches('*') == name && hash.len() == 64).then(|| hash.to_lowercase())
    })
}

fn check(actual: &str, listing: &str, name: &str) -> Result<(), String> {
    let expected = expected_hash(listing, name).ok_or_else(|| format!("no published checksum for {name}"))?;
    if expected == actual { Ok(()) } else { Err(format!("checksum mismatch for {name}: refusing to install")) }
}

async fn install_motty(app: &AppHandle) -> Result<(), String> {
    let release: serde_json::Value = client()?
        .get(format!("https://api.github.com/repos/{MOTTY_REPO}/releases/latest")).send().await
        .and_then(|r| r.error_for_status()).map_err(|e| e.to_string())?.json().await.map_err(|e| e.to_string())?;
    let tag = release["tag_name"].as_str().ok_or("unexpected release response")?;
    let version = tag.trim_start_matches('v');
    let asset = motty_asset(version).ok_or("Motty has no build for this platform")?;
    let base = format!("https://github.com/{MOTTY_REPO}/releases/download/{tag}");
    let dir = tools_dir(app)?;
    let target = dir.join("motty");
    tokio::fs::create_dir_all(&dir).await.map_err(|e| e.to_string())?;
    let archive = dir.join(&asset);
    let hash = download(app, "motty", &format!("{base}/{asset}"), &archive).await?;
    let sums = text(&format!("{base}/SHA256SUMS")).await?;
    if let Err(error) = check(&hash, &sums, &asset) { let _ = tokio::fs::remove_file(&archive).await; return Err(error); }
    emit(app, "motty", "extract", 0, 0);
    let (archive_path, target_path) = (archive.clone(), target.clone());
    tokio::task::spawn_blocking(move || extract(&archive_path, &target_path)).await.map_err(|e| e.to_string())??;
    let _ = tokio::fs::remove_file(&archive).await;
    Ok(())
}

fn extract(archive: &Path, target: &Path) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(target);
    std::fs::create_dir_all(target).map_err(|e| e.to_string())?;
    let file = std::fs::File::open(archive).map_err(|e| e.to_string())?;
    if archive.extension().is_some_and(|e| e == "zip") {
        zip::ZipArchive::new(file).map_err(|e| e.to_string())?.extract(target).map_err(|e| e.to_string())?;
    } else {
        tar::Archive::new(flate2::read::GzDecoder::new(file)).unpack(target).map_err(|e| e.to_string())?;
    }
    // Archives may wrap everything in one folder; flatten so `motty/motty(.exe)` always exists.
    let exe = if cfg!(windows) { "motty.exe" } else { "motty" };
    if !target.join(exe).is_file() {
        let nested = std::fs::read_dir(target).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).find(|p| p.join(exe).is_file());
        if let Some(nested) = nested {
            for entry in std::fs::read_dir(&nested).map_err(|e| e.to_string())?.flatten() {
                std::fs::rename(entry.path(), target.join(entry.file_name())).map_err(|e| e.to_string())?;
            }
        }
    }
    if !target.join(exe).is_file() { return Err("archive did not contain the Motty executable".into()); }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = target.join(exe);
        let mut perms = std::fs::metadata(&path).map_err(|e| e.to_string())?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&path, perms).map_err(|e| e.to_string())?;
    }
    Ok(())
}

async fn install_wireshark(app: &AppHandle) -> Result<(), String> {
    if !cfg!(windows) { return Err("Install Wireshark with your package manager (for example: sudo apt install wireshark).".into()); }
    // Newest stable release: even minor version (odd minors are development builds).
    let index = text("https://www.wireshark.org/download/win64/").await?;
    let mut versions: Vec<(u32, u32, u32)> = index.split("Wireshark-").skip(1).filter_map(|s| {
        let v = s.split("-x64.exe").next()?;
        let mut n = v.split('.').map(|p| p.parse::<u32>().ok());
        let (a, b, c) = (n.next()??, n.next()??, n.next()??);
        (b % 2 == 0).then_some((a, b, c))
    }).collect();
    versions.sort();
    let (a, b, c) = *versions.last().ok_or("could not find a stable Wireshark release")?;
    let version = format!("{a}.{b}.{c}");
    let name = format!("Wireshark-{version}-x64.exe");
    let dest = tools_dir(app)?.join(&name);
    tokio::fs::create_dir_all(dest.parent().unwrap()).await.map_err(|e| e.to_string())?;
    let hash = download(app, "wireshark", &format!("https://www.wireshark.org/download/win64/{name}"), &dest).await?;
    let sums = text(&format!("https://www.wireshark.org/download/SIGNATURES-{version}.txt")).await?;
    if let Err(error) = check(&hash, &sums, &name) { let _ = tokio::fs::remove_file(&dest).await; return Err(error); }
    emit(app, "wireshark", "install", 0, 0);
    // The installer needs administrator rights: let Windows show its normal elevation prompt.
    let status = tokio::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &format!("Start-Process -FilePath '{}' -ArgumentList '/S','/desktopicon=no','/quicklaunchicon=no' -Verb RunAs -Wait", dest.display())])
        .status().await.map_err(|e| e.to_string())?;
    let _ = tokio::fs::remove_file(&dest).await;
    if !status.success() { return Err("the Wireshark installer did not finish (elevation declined?)".into()); }
    Ok(())
}

#[tauri::command]
pub async fn install_tool(app: AppHandle, id: String) -> Result<Vec<ToolStatus>, String> {
    let result = match id.as_str() {
        "motty" => install_motty(&app).await,
        "wireshark" => install_wireshark(&app).await,
        _ => Err(format!("unknown tool {id}")),
    };
    emit(&app, &id, if result.is_ok() { "done" } else { "failed" }, 0, 0);
    result.map(|_| tool_status(app))
}

/// Starts Wireshark on a live capture: the pcap stream is downloaded here and piped to
/// `wireshark -k -i -`, so no local capture driver is involved.
#[tauri::command]
pub async fn launch_wireshark(stream_url: String, title: String) -> Result<(), String> {
    let binary = wireshark_binary().ok_or("Wireshark is not installed")?;
    let response = client()?.get(&stream_url).send().await.and_then(|r| r.error_for_status()).map_err(|e| format!("capture stream: {e}"))?;
    let mut child = tokio::process::Command::new(binary)
        .args(["-k", "-i", "-", "-o", &format!("gui.window_title:{title}")])
        .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().map_err(|e| e.to_string())?;
    let mut stdin = child.stdin.take().ok_or("could not open Wireshark's input")?;
    tauri::async_runtime::spawn(async move {
        let mut stream = response.bytes_stream();
        while let Some(Ok(chunk)) = stream.next().await {
            if stdin.write_all(&chunk).await.is_err() { break; }
        }
        let _ = child.wait().await;
    });
    Ok(())
}

/// Opens a downloaded capture file (non-live) in Wireshark.
#[tauri::command]
pub async fn open_in_wireshark(app: AppHandle, file_url: String, name: String) -> Result<(), String> {
    let binary = wireshark_binary().ok_or("Wireshark is not installed")?;
    let dir = tools_dir(&app)?.join("captures");
    tokio::fs::create_dir_all(&dir).await.map_err(|e| e.to_string())?;
    let safe: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    let dest = dir.join(format!("{safe}.pcap"));
    download(&app, "capture", &file_url, &dest).await?;
    std::process::Command::new(binary).arg(&dest).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

/// Starts Motty, optionally on a `telnet://host:port/` or `ssh://user@host:port/` link: a running
/// Motty hands the link to its existing window, a fresh one opens and connects.
#[tauri::command]
pub fn launch_motty(app: AppHandle, target: Option<String>) -> Result<(), String> {
    let binary = motty_binary(&app).ok_or("Motty is not installed")?;
    let mut command = std::process::Command::new(binary);
    if let Some(target) = target {
        let lower = target.to_ascii_lowercase();
        if !(lower.starts_with("telnet://") || lower.starts_with("ssh://")) || target.chars().any(char::is_whitespace) {
            return Err("Motty can only be opened on telnet:// or ssh:// links".into());
        }
        command.arg(target);
    }
    command.spawn().map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vnc_arguments_are_host_then_port_and_reject_junk() {
        assert_eq!(vnc_args("192.168.233.10", 5900).unwrap(), ["vnc", "192.168.233.10", "5900"]);
        assert!(vnc_args("host; calc", 5900).is_err());
        assert!(vnc_args("--flag", 5900).is_err());
        assert!(vnc_args("host", 0).is_err());
    }

    #[test]
    fn parses_both_checksum_formats() {
        let sums = format!("{}  motty-1-linux-amd64.tar.gz\nSHA256(Wireshark-4.6.9-x64.exe)={}\n", "a".repeat(64), "B".repeat(64));
        assert_eq!(expected_hash(&sums, "motty-1-linux-amd64.tar.gz"), Some("a".repeat(64)));
        assert_eq!(expected_hash(&sums, "Wireshark-4.6.9-x64.exe"), Some("b".repeat(64)));
        assert_eq!(expected_hash(&sums, "missing"), None);
    }

    #[test]
    fn rejects_mismatched_hash() {
        let sums = format!("{}  f.zip", "a".repeat(64));
        assert!(check(&"a".repeat(64), &sums, "f.zip").is_ok());
        assert!(check(&"c".repeat(64), &sums, "f.zip").is_err());
    }
}
