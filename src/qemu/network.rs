use super::*;

pub(super) fn qemu_keyval_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace(',', "\\,")
}

pub(super) fn mac_address(node_id: &str, adapter_number: u32) -> String {
    let bytes = Uuid::parse_str(node_id)
        .map(|id| *id.as_bytes())
        .unwrap_or([0; 16]);
    format!(
        "52:54:00:{:02x}:{:02x}:{:02x}",
        bytes[13] ^ ((adapter_number >> 16) as u8),
        bytes[14] ^ ((adapter_number >> 8) as u8),
        bytes[15] ^ (adapter_number as u8)
    )
}

pub(super) fn qemu_netdev_argument(link: &QemuLink, net_id: &str) -> String {
    format!(
        "socket,id={net_id},udp=127.0.0.1:{},localaddr=127.0.0.1:{}",
        link.adapter_port, link.runtime_port
    )
}

pub(super) fn qemu_nio_id(link_id: &str) -> String {
    link_id.chars().filter(char::is_ascii_hexdigit).collect()
}

pub(super) fn qmp_reports_duplicate_id(error: &RuntimeError) -> bool {
    let message = error.message().to_ascii_lowercase();
    message.contains("duplicate") || message.contains("already exists")
}

pub(super) fn qmp_reports_missing_id(error: &RuntimeError) -> bool {
    let message = error.message().to_ascii_lowercase();
    message.contains("not found")
        || message.contains("does not exist")
        || message.contains("not a valid")
}

