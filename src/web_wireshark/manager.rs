use std::{
    collections::{HashMap, HashSet},
    process::Stdio,
    sync::Mutex,
    time::Duration,
};

use tokio::{process::Command, time::timeout};

const FIRST_PORT: u16 = 10_000;
const LAST_PORT: u16 = 19_999;
const NETWORK: &str = "gns3-wireshark";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XpraSession {
    pub display: u16,
    pub port: u16,
}

#[derive(Clone, Default)]
pub struct SessionAllocator {
    allocated: std::sync::Arc<Mutex<HashMap<String, XpraSession>>>,
}

impl SessionAllocator {
    pub fn get(&self, key: &str) -> Option<XpraSession> {
        self.allocated.lock().ok()?.get(key).cloned()
    }
    pub fn allocate(&self, key: &str) -> Option<XpraSession> {
        let mut allocated = self.allocated.lock().ok()?;
        if let Some(session) = allocated.get(key) {
            return Some(session.clone());
        }
        let used: HashSet<u16> = allocated.values().map(|s| s.port).collect();
        let port = (FIRST_PORT..=LAST_PORT).find(|port| !used.contains(port))?;
        let session = XpraSession {
            display: port,
            port,
        };
        allocated.insert(key.to_owned(), session.clone());
        Some(session)
    }
    pub fn release(&self, key: &str) {
        if let Ok(mut allocated) = self.allocated.lock() {
            allocated.remove(key);
        }
    }
    pub fn reserve(&self, key: &str, session: XpraSession) -> bool {
        let Ok(mut allocated) = self.allocated.lock() else {
            return false;
        };
        if allocated
            .iter()
            .any(|(other, value)| other != key && value.port == session.port)
        {
            return false;
        }
        allocated.insert(key.to_owned(), session);
        true
    }
}

#[derive(Clone)]
pub struct WebWiresharkManager {
    allocator: SessionAllocator,
    image: String,
    subnet: String,
}

impl Default for WebWiresharkManager {
    fn default() -> Self {
        Self {
            allocator: SessionAllocator::default(),
            image: "gns3/web-wireshark:latest".into(),
            subnet: "172.31.0.0/22".into(),
        }
    }
}

impl WebWiresharkManager {
    pub fn container_name(project_id: &str) -> String {
        format!("gns3-wireshark-{project_id}")
    }
    pub async fn ensure_image(&self) -> Result<(), String> {
        if docker(&["pull", &self.image]).await.is_ok() {
            return Ok(());
        }
        let dockerfile =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docker/web-wireshark");
        docker(&[
            "build",
            "-t",
            &self.image,
            "-f",
            dockerfile
                .join("Dockerfile")
                .to_str()
                .ok_or("invalid Dockerfile path")?,
            dockerfile.to_str().ok_or("invalid Docker build path")?,
        ])
        .await
        .map(|_| ())
    }
    pub async fn ensure_container(&self, project_id: &str) -> Result<(String, String), String> {
        self.ensure_image().await?;
        let name = Self::container_name(project_id);
        if let Ok(output) = docker(&[
            "inspect",
            "-f",
            "{{.State.Running}} {{(index .NetworkSettings.Networks \"gns3-wireshark\").IPAddress}}",
            &name,
        ])
        .await
        {
            let mut values = output.split_whitespace();
            let running = values.next() == Some("true");
            let ip = values.next().unwrap_or("").to_owned();
            if !running {
                docker(&["start", &name]).await?;
            }
            if !ip.is_empty() {
                self.rebuild_allocations(&name, project_id).await;
                return Ok((name, ip));
            }
        }
        if docker(&["network", "inspect", NETWORK]).await.is_err() {
            docker(&[
                "network",
                "create",
                "--driver",
                "bridge",
                &format!("--subnet={}", self.subnet),
                NETWORK,
            ])
            .await?;
        }
        docker(&[
            "run",
            "-d",
            "--init",
            "--restart",
            "unless-stopped",
            "--name",
            &name,
            "--network",
            NETWORK,
            "--memory",
            "2g",
            "--cpus",
            "1.0",
            "--pids-limit",
            "1000",
            &self.image,
        ])
        .await?;
        let ip = docker(&[
            "inspect",
            "-f",
            &format!("{{{{(index .NetworkSettings.Networks \"{NETWORK}\").IPAddress}}}}"),
            &name,
        ])
        .await?;
        Ok((name, ip.trim().to_owned()))
    }
    pub async fn start_session(
        &self,
        project_id: &str,
        link_id: &str,
        stream_url: &str,
    ) -> Result<(XpraSession, String), String> {
        let (container, ip) = self.ensure_container(project_id).await?;
        let key = format!("{project_id}/{link_id}");
        let session = self
            .allocator
            .allocate(&key)
            .ok_or("No free Web Wireshark ports")?;
        for command in cleanup_commands(session.display) {
            let _ = docker(&["exec", &container, "sh", "-lc", &command]).await;
        }
        let args = xpra_start_command(&container, session.display, session.port);
        let mut args = args;
        if let Some(index) = args
            .iter()
            .position(|arg| arg == "--session-name=link-<id>")
        {
            args[index] = format!("--session-name=link-{link_id}");
        }
        docker(&args.iter().map(String::as_str).collect::<Vec<_>>()).await?;
        let stream = format!(
            "curl -N '{}' | wireshark -i - -k --fullscreen -display :{} &",
            stream_url.replace('\'', "'\\''"),
            session.display
        );
        docker(&[
            "exec",
            "-e",
            "XPRA_CLIENT_CAN_SHUTDOWN=false",
            &container,
            "sh",
            "-lc",
            &stream,
        ])
        .await?;
        Ok((session, ip))
    }
    pub async fn stop_session(&self, project_id: &str, link_id: &str) -> Result<(), String> {
        let key = format!("{project_id}/{link_id}");
        let Some(session) = self.allocator.get(&key) else {
            return Ok(());
        };
        let name = Self::container_name(project_id);
        let _ = docker(&[
            "exec",
            &name,
            "xpra",
            "stop",
            &format!(":{}", session.display),
        ])
        .await;
        for command in cleanup_commands(session.display) {
            let _ = docker(&["exec", &name, "sh", "-lc", &command]).await;
        }
        self.allocator.release(&key);
        let manager = self.clone();
        let project = project_id.to_owned();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            if manager
                .allocator
                .allocated
                .lock()
                .map(|sessions| {
                    !sessions
                        .keys()
                        .any(|key| key.starts_with(&format!("{project}/")))
                })
                .unwrap_or(false)
            {
                manager.delete_project(&project).await;
            }
        });
        Ok(())
    }
    pub async fn delete_project(&self, project_id: &str) {
        let _ = docker(&["rm", "-f", &Self::container_name(project_id)]).await;
    }
    pub async fn container_ip(&self, project_id: &str) -> Result<String, String> {
        let (_, ip) = self.ensure_container(project_id).await?;
        Ok(ip)
    }
    pub fn session(&self, project_id: &str, link_id: &str) -> Option<XpraSession> {
        self.allocator.get(&format!("{project_id}/{link_id}"))
    }
    async fn rebuild_allocations(&self, container: &str, project_id: &str) {
        let Ok(listing) = docker(&["exec", container, "xpra", "list"]).await else {
            return;
        };
        for line in listing.lines() {
            let Some(display) = line
                .split(':')
                .nth(1)
                .and_then(|value| value.split(|c: char| !c.is_ascii_digit()).next())
                .and_then(|value| value.parse::<u16>().ok())
            else {
                continue;
            };
            if !(FIRST_PORT..=LAST_PORT).contains(&display) {
                continue;
            }
            // `xpra list` is authoritative for live displays. It does not expose the
            // custom session name consistently across xpra releases, so reserve the
            // display under a synthetic key to prevent allocator collisions.
            self.allocator.reserve(
                &format!("{project_id}/recovered-{display}"),
                XpraSession {
                    display,
                    port: display,
                },
            );
        }
    }
}

pub fn xpra_start_command(container: &str, display: u16, port: u16) -> Vec<String> {
    vec![
        "exec".into(),
        "-e".into(),
        "XPRA_CLIENT_CAN_SHUTDOWN=false".into(),
        container.into(),
        "xpra".into(),
        "start".into(),
        format!(":{display}"),
        "--xvfb=Xvfb -screen 0 1920x1080x24 +extension RANDR".into(),
        "--html=off".into(),
        format!("--bind-ws=0.0.0.0:{port}"),
        "--session-name=link-<id>".into(),
        "--daemon=yes".into(),
        "--dbus-launch=no".into(),
        "--resize-display=yes".into(),
    ]
}
fn cleanup_commands(display: u16) -> [String; 2] {
    [
        format!(
            "pkill -f '^wireshark .* -display :{display}([ ]|$)' || true; pkill -f '^Xvfb :{display}([ ]|$)' || true"
        ),
        format!("rm -f /tmp/.X{display}-lock /tmp/.X11-unix/X{display}"),
    ]
}
async fn docker(args: &[&str]) -> Result<String, String> {
    let output = timeout(
        Duration::from_secs(60),
        Command::new("docker")
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    .map_err(|_| "docker command timed out".to_owned())?
    .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allocator_tracks_and_reuses_ports() {
        let allocator = SessionAllocator::default();
        let a = allocator.allocate("a").unwrap();
        let b = allocator.allocate("b").unwrap();
        assert_eq!(a.port, 10_000);
        assert_eq!(b.port, 10_001);
        allocator.release("a");
        assert_eq!(allocator.allocate("c").unwrap().port, 10_000);
    }
    #[test]
    fn xpra_command_contains_requested_display_and_port() {
        let command = xpra_start_command("container", 10031, 10032).join(" ");
        assert!(command.contains(":10031"));
        assert!(command.contains("0.0.0.0:10032"));
        assert!(command.contains("--daemon=yes"));
    }
}
