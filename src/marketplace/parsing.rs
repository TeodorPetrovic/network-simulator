use super::*;

pub(super) fn ishare_row_matches_query(row: &Value, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    row["name"]
        .as_str()
        .is_some_and(|name| name.to_lowercase().contains(needle))
        || row["files"].as_array().is_some_and(|files| {
            files.iter().any(|file| {
                file["filename"]
                    .as_str()
                    .is_some_and(|filename| filename.to_lowercase().contains(needle))
            })
        })
}

impl Default for MarketplaceCatalog {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) fn validate_query(query: &str) -> Result<&str, String> {
    if query.len() > 120 {
        return Err("Catalog searches are limited to 120 characters".to_owned());
    }
    Ok(query.trim())
}

pub(super) fn safe_catalog_component(value: &str) -> Result<String, String> {
    let mut component = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    while component.starts_with('.') {
        component.remove(0);
    }
    component.truncate(96);
    if component.is_empty() || component == "." || component == ".." {
        return Err("The iShare2 image name cannot be used as a filename".to_owned());
    }
    Ok(component)
}

pub(super) fn safe_download_filename(filename: &str) -> bool {
    !filename.is_empty()
        && filename.len() <= 255
        && !filename.starts_with('.')
        && !filename.contains('/')
        && !filename.contains('\\')
        && filename.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'.' | b'_' | b'-' | b'+' | b'[' | b']' | b'(' | b')')
        })
}

pub(super) fn safe_ishare_path(path: &str) -> bool {
    if !path.starts_with("/addons/")
        || path.contains('?')
        || path.contains('#')
        || path.contains('\\')
        || path.to_ascii_lowercase().contains("%2f")
        || path.to_ascii_lowercase().contains("%5c")
        || path.to_ascii_lowercase().contains("%2e")
    {
        return false;
    }
    path.split('/')
        .all(|component| component != ".." && component != ".")
        && path.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'/' | b'.' | b'_' | b'-' | b'[' | b']' | b'%' | b'+')
        })
}

pub(super) fn valid_ishare_host(host: &str) -> bool {
    host == "labhub.eu.org" || host.ends_with(".labhub.eu.org")
}

pub(super) fn valid_ishare_prefix(prefix: &str) -> bool {
    prefix.starts_with('/')
        && !prefix.contains("..")
        && prefix.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b':' | b'-' | b'_' | b'.')
        })
}

pub(super) fn is_supported_qemu_extension(extension: &str) -> bool {
    matches!(extension, "qcow2" | "qcow" | "raw" | "img" | "vmdk" | "vdi")
}

pub(super) fn validate_appliance_file_name(file_name: &str) -> Result<&str, String> {
    if !file_name.ends_with(".gns3a")
        || file_name.len() > 180
        || !file_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+'))
    {
        return Err("Appliance path must be a single .gns3a filename".to_owned());
    }
    Ok(file_name)
}

pub(super) fn title_from_slug(slug: &str) -> String {
    slug.split(['-', '_', '+'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut characters = part.chars();
            characters.next().map_or_else(String::new, |first| {
                first.to_uppercase().collect::<String>() + characters.as_str()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn image_file_name(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
}

pub(super) fn summarize_appliance(
    file_name: &str,
    appliance: &Value,
    imported: bool,
) -> Result<ApplianceRecord, String> {
    let appliance_id = appliance
        .get("appliance_id")
        .and_then(Value::as_str)
        .filter(|value| Uuid::parse_str(value).is_ok())
        .ok_or_else(|| "GNS3 appliance is missing a valid appliance_id".to_owned())?;
    let name = appliance
        .get("name")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "GNS3 appliance is missing its name".to_owned())?;
    let mut required_files = Vec::new();
    if let Some(images) = appliance.get("images").and_then(Value::as_array) {
        for image in images {
            let Some(filename) = image.get("filename").and_then(Value::as_str) else {
                continue;
            };
            required_files.push(ApplianceFile {
                filename: filename.to_owned(),
                version: image
                    .get("version")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                size: image.get("filesize").and_then(Value::as_u64),
                md5sum: image
                    .get("checksum")
                    .or_else(|| image.get("md5sum"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                download_url: image
                    .get("direct_download_url")
                    .or_else(|| image.get("download_url"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            });
        }
    }
    let versions = appliance_versions(appliance, &required_files);
    Ok(ApplianceRecord {
        file_name: file_name.to_owned(),
        appliance_id: appliance_id.to_owned(),
        name: name.to_owned(),
        origin: if appliance["network_simulator"]["origin"].as_str() == Some("custom") {
            "custom".to_owned()
        } else {
            "gns3".to_owned()
        },
        category: appliance
            .get("category")
            .and_then(Value::as_str)
            .map(str::to_owned),
        description: appliance
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_owned),
        status: appliance
            .get("status")
            .and_then(Value::as_str)
            .map(str::to_owned),
        registry_version: appliance.get("registry_version").and_then(Value::as_u64),
        template_type: appliance_runtime_type(appliance).map(str::to_owned),
        imported,
        source_url: if appliance["network_simulator"]["origin"].as_str() == Some("custom") {
            "Custom appliance created in this library".to_owned()
        } else {
            appliance
                .get("source_url")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    format!(
                        "https://github.com/GNS3/gns3-registry/blob/master/appliances/{file_name}"
                    )
                })
        },
        required_files,
        versions,
    })
}
