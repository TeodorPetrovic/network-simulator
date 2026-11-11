use super::*;

/// MAC for one adapter: the node's stored base address (stable across project import) with the
/// adapter number added to the last octet, or the legacy node-id derived address.
pub(super) fn adapter_mac(vm: &QemuVm, adapter_number: u32) -> String {
    if let Some(base) = vm.mac_base.as_deref() {
        let mut octets: Vec<u8> = base.split(':').filter_map(|part| u8::from_str_radix(part, 16).ok()).collect();
        if octets.len() == 6 {
            let last = u32::from(octets[5]) + adapter_number;
            octets[5] = (last & 0xff) as u8;
            octets[4] = octets[4].wrapping_add((last >> 8) as u8);
            return format!("{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", octets[0], octets[1], octets[2], octets[3], octets[4], octets[5]);
        }
    }
    mac_address(&vm.node_id, adapter_number)
}

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

pub(super) fn qemu_adapter_netdev_id(adapter_number: u32) -> String {
    format!("nsboot{adapter_number}")
}

pub(super) fn qemu_set_link_command(adapter_number: u32, up: bool) -> String {
    format!(
        "set_link {} {}",
        qemu_adapter_netdev_id(adapter_number),
        if up { "on" } else { "off" }
    )
}

pub(super) fn qemu_adapter_device_id(adapter_number: u32) -> String {
    format!("nsdev_boot{adapter_number}")
}

pub(super) fn qmp_reports_missing_id(error: &RuntimeError) -> bool {
    let message = error.message().to_ascii_lowercase();
    message.contains("not found")
        || message.contains("does not exist")
        || message.contains("not a valid")
}

#[cfg(test)]
mod tests {
    use super::qemu_set_link_command;

    #[test]
    fn set_link_targets_boot_netdev_ids() {
        assert_eq!(qemu_set_link_command(0, true), "set_link nsboot0 on");
        assert_eq!(qemu_set_link_command(12, false), "set_link nsboot12 off");
    }
}
