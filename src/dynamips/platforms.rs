use super::*;

pub(super) fn default_slot0(platform: &str, chassis: Option<&str>) -> Option<&'static str> {
    match platform {
        "c7200" => Some("C7200-IO-FE"),
        "c3600" if chassis == Some("3660") => Some("Leopard-2FE"),
        "c3600" => Some("NM-1FE-TX"),
        "c2691" | "c3725" | "c3745" => Some("GT96100-FE"),
        "c2600" => Some(match chassis {
            Some("2611") => "C2600-MB-2E",
            Some("2620" | "2610XM" | "2620XM" | "2650XM") => "C2600-MB-1FE",
            Some("2621" | "2611XM" | "2621XM" | "2651XM") => "C2600-MB-2FE",
            _ => "C2600-MB-1E",
        }),
        "c1700" => Some("C1700-MB-1FE"),
        _ => None,
    }
}

pub(super) fn default_iomem(platform: &str) -> Option<u8> {
    match platform {
        "c1700" | "c2600" => Some(15),
        "c2691" | "c3600" | "c3725" | "c3745" => Some(5),
        _ => None,
    }
}

/// Return the adapter Dynamips installs as part of creating this platform.
/// GNS3's Dynamips node wrappers model these adapters in their slot state, but
/// the hypervisor creates them when the chassis/platform is initialized. Sending
/// `slot_add_binding` for them a second time fails on real Dynamips.
pub(super) fn integrated_slot0(platform: &str, chassis: Option<&str>) -> Option<&'static str> {
    match platform {
        "c1700" => Some("C1700-MB-1FE"),
        "c2600" => Some(match chassis.unwrap_or("2610") {
            "2611" => "C2600-MB-2E",
            "2620" | "2610XM" | "2620XM" | "2650XM" => "C2600-MB-1FE",
            "2621" | "2611XM" | "2621XM" | "2651XM" => "C2600-MB-2FE",
            _ => "C2600-MB-1E",
        }),
        "c3600" if chassis == Some("3660") => Some("Leopard-2FE"),
        "c2691" | "c3725" | "c3745" => Some("GT96100-FE"),
        _ => None,
    }
}

pub(super) fn integrated_slot_module(
    platform: &str,
    chassis: Option<&str>,
    slot: usize,
) -> Option<&'static str> {
    if slot == 0 {
        return integrated_slot0(platform, chassis);
    }
    match (platform, chassis, slot) {
        ("c1700", Some("1751" | "1760"), 1) => Some("C1700-MB-WIC1"),
        _ => None,
    }
}

pub(super) fn validate_platform_settings(
    platform: &str,
    chassis: Option<&str>,
    slot0: Option<&str>,
) -> Result<(), RuntimeError> {
    let slot0 = slot0.or_else(|| default_slot0(platform, chassis));
    const PLATFORMS: &[&str] = &[
        "c1700", "c2600", "c2691", "c3600", "c3725", "c3745", "c7200",
    ];
    let chassis_valid = match platform {
        "c1700" => matches!(
            chassis,
            None | Some("1720" | "1721" | "1750" | "1751" | "1760")
        ),
        "c2600" => matches!(
            chassis,
            None | Some(
                "2610"
                    | "2611"
                    | "2620"
                    | "2610XM"
                    | "2620XM"
                    | "2650XM"
                    | "2621"
                    | "2611XM"
                    | "2621XM"
                    | "2651XM"
            )
        ),
        "c3600" => matches!(chassis, None | Some("3620" | "3640" | "3660")),
        _ => chassis.is_none(),
    };
    let supported = PLATFORMS.contains(&platform)
        && chassis_valid
        && slot0.is_some_and(|module| module_supported_on_slot(platform, chassis, 0, module));
    if !supported {
        return Err(RuntimeError::Failed(format!(
            "Dynamips runtime does not support this configuration (platform '{platform}', chassis '{}', slot 0 '{}')",
            chassis.unwrap_or("default"),
            slot0.unwrap_or("empty")
        )));
    }
    Ok(())
}

pub(super) fn validate_dynamips_options(
    platform: &str,
    chassis: Option<&str>,
    slot0: Option<&str>,
    options: &DynamipsOptions,
) -> Result<(), RuntimeError> {
    validate_platform_settings(platform, chassis, slot0)?;
    if options.ghost_ios && !options.mmap {
        return Err(RuntimeError::Failed(
            "Dynamips mmap support is required to enable ghost IOS".to_owned(),
        ));
    }
    if !(1..=4096).contains(&options.dynamips_id) {
        return Err(RuntimeError::Failed(
            "Dynamips internal ID must be from 1 to 4096".to_owned(),
        ));
    }
    if options.clock_divisor == 0 || options.clock_divisor > 4096 {
        return Err(RuntimeError::Failed(
            "Dynamips clock_divisor must be from 1 to 4096".to_owned(),
        ));
    }
    if options.exec_area > 4096 || options.disk0 > 16_384 || options.disk1 > 16_384 {
        return Err(RuntimeError::Failed(
            "Dynamips exec area and disk sizes are outside the supported range".to_owned(),
        ));
    }
    if options.iomem.is_some_and(|value| value > 100) {
        return Err(RuntimeError::Failed(
            "Dynamips iomem must be from 0 to 100".to_owned(),
        ));
    }
    if options.slots.len() > 7 || options.wics.len() > 3 {
        return Err(RuntimeError::Failed(
            "Dynamips supports slots 0–6 and WIC slots 0–2".to_owned(),
        ));
    }
    let slot_capacity = match platform {
        "c1700" if matches!(chassis, Some("1751" | "1760")) => 2,
        "c1700" => 1,
        "c2600" | "c2691" => 2,
        "c3600" => match chassis.unwrap_or("3640") {
            "3620" => 2,
            "3660" => 7,
            _ => 4,
        },
        "c3725" => 3,
        "c3745" => 5,
        _ => 7,
    };
    if options.slots.iter().enumerate().any(|(slot, module)| {
        slot >= slot_capacity && module.as_deref().is_some_and(|module| !module.is_empty())
    }) {
        return Err(RuntimeError::Failed(format!(
            "Dynamips platform {platform} with chassis '{}' supports slots 0 through {}",
            chassis.unwrap_or("default"),
            slot_capacity - 1
        )));
    }
    if options.slots.iter().enumerate().any(|(slot, module)| {
        module
            .as_deref()
            .is_some_and(|module| !module_supported_on_slot(platform, chassis, slot, module))
    }) {
        return Err(RuntimeError::Failed(
            "Dynamips slot contains a network module that is incompatible with this platform"
                .to_owned(),
        ));
    }
    if options
        .wics
        .iter()
        .flatten()
        .any(|wic| !matches!(wic.as_str(), "" | "WIC-1ENET" | "WIC-1T" | "WIC-2T"))
    {
        return Err(RuntimeError::Failed(
            "Dynamips WIC must be WIC-1ENET, WIC-1T, or WIC-2T".to_owned(),
        ));
    }
    let wic_capacity = match platform {
        "c1700" => 2,
        "c2600" | "c2691" | "c3725" | "c3745" => 3,
        _ => 0,
    };
    if options.wics.iter().enumerate().any(|(slot, wic)| {
        slot >= wic_capacity && wic.as_deref().is_some_and(|wic| !wic.is_empty())
    }) {
        return Err(RuntimeError::Failed(format!(
            "Dynamips platform {platform} provides {wic_capacity} WIC slot(s)"
        )));
    }
    if options.npe.as_deref().is_some_and(|npe| {
        !matches!(
            npe,
            "npe-100"
                | "npe-150"
                | "npe-175"
                | "npe-200"
                | "npe-225"
                | "npe-300"
                | "npe-400"
                | "npe-g2"
        )
    }) || options
        .midplane
        .as_deref()
        .is_some_and(|midplane| !matches!(midplane, "std" | "vxr"))
    {
        return Err(RuntimeError::Failed(
            "Dynamips NPE or midplane model is unsupported".to_owned(),
        ));
    }
    if options.mac_addr.as_deref().is_some_and(|address| {
        !address.is_empty()
            && !(address.len() == 14
                && address.split('.').count() == 3
                && address.split('.').all(|part| {
                    part.len() == 4 && part.bytes().all(|byte| byte.is_ascii_hexdigit())
                }))
    }) {
        return Err(RuntimeError::Failed(
            "Dynamips MAC address must use xxxx.xxxx.xxxx format".to_owned(),
        ));
    }
    if options.system_id.as_deref().is_some_and(|id| {
        !id.is_empty() && (id.len() > 32 || !id.bytes().all(|byte| byte.is_ascii_alphanumeric()))
    }) {
        return Err(RuntimeError::Failed(
            "Dynamips system_id must contain up to 32 letters or numbers".to_owned(),
        ));
    }
    if options.console_type != "telnet"
        && options.console_type != "ssh"
        && options.console_type != "none"
        || options.aux_type != "telnet" && options.aux_type != "ssh" && options.aux_type != "none"
    {
        return Err(RuntimeError::Failed(
            "Dynamips console types must be telnet, ssh, or none".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn module_supported_on_slot(
    platform: &str,
    chassis: Option<&str>,
    slot: usize,
    module: &str,
) -> bool {
    if module.is_empty() {
        return true;
    }
    match module {
        "C7200-IO-2FE" | "C7200-IO-FE" | "C7200-IO-GE-E" => platform == "c7200" && slot == 0,
        "PA-2FE-TX" | "PA-4E" | "PA-4T+" | "PA-8E" | "PA-8T" | "PA-A1" | "PA-FE-TX" | "PA-GE"
        | "PA-POS-OC3" => platform == "c7200" && (1..=6).contains(&slot),
        "C2600-MB-2FE" | "C2600-MB-1E" | "C2600-MB-2E" | "C2600-MB-1FE" => {
            platform == "c2600" && slot == 0
        }
        "C1700-MB-1FE" => platform == "c1700" && slot == 0,
        "C1700-MB-WIC1" => {
            platform == "c1700" && slot == 1 && matches!(chassis, Some("1751" | "1760"))
        }
        "GT96100-FE" => matches!(platform, "c2691" | "c3725" | "c3745") && slot == 0,
        "Leopard-2FE" => platform == "c3600" && chassis == Some("3660") && slot == 0,
        "NM-16ESW" | "NM-1E" | "NM-1FE-TX" | "NM-4E" | "NM-4T" => {
            matches!(platform, "c2600" | "c2691" | "c3600" | "c3725" | "c3745")
                && (slot > 0 || (platform == "c3600" && chassis != Some("3660")))
        }
        _ => false,
    }
}

pub(super) fn router_initialization_commands(vm: &DynamipsVm) -> Vec<String> {
    let mut commands = vec![
        format!(
            "vm create {} {} {}",
            quote(&vm.name),
            vm.options.dynamips_id,
            vm.platform
        ),
        format!(
            "vm set_ios {} {}",
            quote(&vm.name),
            quote(&vm.image.display().to_string())
        ),
        format!("vm set_ram {} {}", quote(&vm.name), vm.ram_mb),
        format!("vm set_nvram {} {}", quote(&vm.name), vm.nvram),
        format!(
            "vm set_ram_mmap {} {}",
            quote(&vm.name),
            u8::from(vm.options.mmap)
        ),
        format!(
            "vm set_sparse_mem {} {}",
            quote(&vm.name),
            u8::from(vm.options.sparsemem)
        ),
        format!(
            "vm set_exec_area {} {}",
            quote(&vm.name),
            vm.options.exec_area
        ),
        format!(
            "vm set_clock_divisor {} {}",
            quote(&vm.name),
            vm.options.clock_divisor
        ),
        format!("vm set_disk0 {} {}", quote(&vm.name), vm.options.disk0),
        format!("vm set_disk1 {} {}", quote(&vm.name), vm.options.disk1),
    ];
    if vm.options.console_type != "none" {
        commands.push(format!(
            "vm set_con_tcp_port {} {}",
            quote(&vm.name),
            vm.console_port
        ));
    }
    if vm.options.aux_type != "none" {
        if let Some(aux_port) = vm.aux_port {
            commands.push(format!(
                "vm set_aux_tcp_port {} {aux_port}",
                quote(&vm.name)
            ));
        }
    }
    if matches!(vm.platform.as_str(), "c1700" | "c2600" | "c3600")
        && let Some(chassis) = &vm.chassis
    {
        commands.push(format!(
            "{} set_chassis {} {chassis}",
            vm.platform,
            quote(&vm.name)
        ));
    }
    if vm.platform == "c7200" {
        if let Some(npe) = &vm.options.npe {
            commands.push(format!("c7200 set_npe {} {npe}", quote(&vm.name)));
        }
        if let Some(midplane) = &vm.options.midplane {
            commands.push(format!("c7200 set_midplane {} {midplane}", quote(&vm.name)));
        }
    } else if let Some(iomem) = vm.options.iomem.or_else(|| default_iomem(&vm.platform)) {
        commands.push(format!(
            "{} set_iomem {} {iomem}",
            vm.platform,
            quote(&vm.name)
        ));
    }
    if let Some(mac_addr) = &vm.options.mac_addr {
        commands.push(format!(
            "{} set_mac_addr {} {mac_addr}",
            vm.platform,
            quote(&vm.name)
        ));
    }
    if let Some(system_id) = &vm.options.system_id {
        commands.push(format!(
            "{} set_system_id {} {system_id}",
            vm.platform,
            quote(&vm.name)
        ));
    }
    if let Some(idle_pc) = &vm.idle_pc {
        commands.push(format!("vm set_idle_pc {} {idle_pc}", quote(&vm.name)));
    }
    let max_slots = match vm.platform.as_str() {
        "c1700" => 2,
        "c2600" | "c2691" => 2,
        "c3600" => match vm.chassis.as_deref().unwrap_or("3640") {
            "3620" => 2,
            "3660" => 7,
            _ => 4,
        },
        "c3725" => 3,
        "c3745" => 5,
        _ => 7,
    };
    for slot in 0..max_slots {
        let module = vm
            .options
            .slots
            .get(slot)
            .and_then(Option::as_deref)
            .filter(|module| !module.is_empty())
            .or_else(|| (slot == 0).then_some(vm.slot0.as_str()));
        if let Some(module) = module {
            if integrated_slot_module(&vm.platform, vm.chassis.as_deref(), slot) == Some(module) {
                continue;
            }
            commands.push(format!(
                "vm slot_add_binding {} {slot} 0 {module}",
                quote(&vm.name)
            ));
        }
    }
    for (wic_slot, wic) in vm.options.wics.iter().enumerate() {
        if let Some(wic) = wic.as_deref().filter(|wic| !wic.is_empty()) {
            commands.push(format!(
                "vm slot_add_binding {} 0 {} {wic}",
                quote(&vm.name),
                16 * (wic_slot + 1)
            ));
        }
    }
    if vm.options.ghost_ios
        && vm.options.mmap
        && vm.ghost_file.as_ref().is_some_and(|path| path.is_file())
    {
        let ghost_file = vm.ghost_file.as_ref().expect("checked ghost file");
        commands.push(format!("vm set_ghost_status {} 2", quote(&vm.name)));
        commands.push(format!(
            "vm set_ghost_file {} {}",
            quote(&vm.name),
            quote(&ghost_file.display().to_string())
        ));
    }
    commands
}

pub(super) fn ghost_router_initialization_commands(
    vm: &DynamipsVm,
    ghost_file: &Path,
) -> Vec<String> {
    let name = "Ghost";
    vec![
        format!("vm create {} 0 {}", quote(name), vm.platform),
        format!(
            "vm set_ios {} {}",
            quote(name),
            quote(&vm.image.display().to_string())
        ),
        format!("vm set_ghost_status {} 1", quote(name)),
        format!(
            "vm set_ghost_file {} {}",
            quote(name),
            quote(&ghost_file.display().to_string())
        ),
        format!("vm set_ram {} {}", quote(name), vm.ram_mb),
    ]
}

/// Idle tuning sent after `vm start`, when Dynamips has created the CPU.
///
/// GNS3 never sends its default `idlemax` 500 / `idlesleep` 30: its router
/// object starts with those values and only issues `vm set_idle_*` when an
/// edit changes them. Dynamips then keeps its own CPU defaults, `idle_max`
/// 500 and `idle_sleep_time` 30000. The sleep time is in microseconds
/// (`cpu_idle_loop`), so sending GNS3's `30` would shorten each idle sleep
/// from 30 ms to 30 µs and keep the host core busy even with a good Idle-PC.
/// Non-default values are passed through unchanged, as GNS3 does.
pub(super) fn router_start_settings_commands(vm: &DynamipsVm) -> Vec<String> {
    let mut commands = Vec::new();
    if vm.options.idlemax != GNS3_DEFAULT_IDLE_MAX {
        commands.push(format!(
            "vm set_idle_max {} 0 {}",
            quote(&vm.name),
            vm.options.idlemax
        ));
    }
    if vm.options.idlesleep != GNS3_DEFAULT_IDLE_SLEEP {
        commands.push(format!(
            "vm set_idle_sleep_time {} 0 {}",
            quote(&vm.name),
            vm.options.idlesleep
        ));
    }
    commands
}

pub(super) fn router_start_idlepc_command(vm: &DynamipsVm) -> Option<String> {
    vm.idle_pc
        .as_deref()
        .map(|idle_pc| format!("vm set_idle_pc {} {idle_pc}", quote(&vm.name)))
}
