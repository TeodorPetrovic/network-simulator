use super::*;

pub(super) async fn available_udp_port() -> Result<u16, RuntimeError> {
    let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not allocate a UDP port: {error}"))
        })?;
    socket
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not read an allocated UDP port: {error}"))
        })
}

pub(super) async fn available_tcp_port() -> Result<u16, RuntimeError> {
    let listener = tokio::net::TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not allocate a TCP port: {error}"))
        })?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| {
            RuntimeError::Unavailable(format!("Could not read an allocated TCP port: {error}"))
        })
}

pub(super) fn parse_id(id: &str) -> Result<Uuid, RuntimeError> {
    Uuid::parse_str(id).map_err(|_| RuntimeError::Failed("VPCS IDs must be valid UUIDs".to_owned()))
}

pub(super) fn mac_offset(node_id: &Uuid) -> u8 {
    node_id.as_bytes()[15]
}

pub(super) fn write_startup_script(
    node_dir: &std::path::Path,
    startup_script: Option<&str>,
    name: &str,
) -> Result<PathBuf, RuntimeError> {
    let path = node_dir.join("startup.vpc");
    let script = startup_script.unwrap_or_default().replace("%h", name);
    fs::write(&path, script).map_err(|error| {
        RuntimeError::Failed(format!("Could not write VPCS startup script: {error}"))
    })?;
    Ok(path)
}

