//! Dynamips version parsing and GNS3 compatibility policy.

const GNS3_MIN_DYNAMIPS_VERSION: (u32, u32, u32) = (0, 2, 11);
pub(super) const CONSOLE_BINDING_MIN_VERSION: (u32, u32, u32) = (0, 2, 23);

pub(super) fn dynamips_version_tuple(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts.next()?.parse::<u32>().ok()?;
    let patch = parts
        .next()
        .unwrap_or_default()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse::<u32>()
        .ok()?;
    Some((major, minor, patch))
}

pub(super) fn dynamips_version_error(version: &str) -> Option<String> {
    let Some(version_tuple) = dynamips_version_tuple(version) else {
        return Some(format!("Could not parse Dynamips version '{version}'"));
    };
    if version_tuple < GNS3_MIN_DYNAMIPS_VERSION {
        return Some(format!(
            "Dynamips 0.2.11 or newer is required (GNS3 3.1 minimum); detected {version}"
        ));
    }
    if version_tuple == (0, 2, 14) {
        return Some(format!(
            "Dynamips {version} is blocked because this build crashes while starting IOS after ROMMON; use Dynamips 0.2.25 or set NETWORK_SIMULATOR_DYNAMIPS to a compatible binary"
        ));
    }
    None
}

/// GNS3 passes `--console-binding-addr` only to Dynamips 0.2.23 and newer.
pub(super) fn dynamips_version_warning(version: &str) -> Option<String> {
    dynamips_version_tuple(version)
        .filter(|version| *version < CONSOLE_BINDING_MIN_VERSION)
        .map(|_| {
            format!(
                "Dynamips {version} predates --console-binding-addr (0.2.23); router console ports cannot be restricted to 127.0.0.1"
            )
        })
}

pub(super) fn supported_dynamips_version(version: &str) -> bool {
    dynamips_version_error(version).is_none()
}
