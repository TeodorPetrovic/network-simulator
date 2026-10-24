use super::*;

#[derive(Debug)]
pub(super) struct ImportedProjectArchive {
    pub(super) project: Value,
    pub(super) project_files: Vec<(String, Vec<u8>)>,
    pub(super) images: Vec<ImportedProjectImage>,
}

#[derive(Debug)]
pub(super) struct ImportedProjectImage {
    pub(super) image_type: String,
    pub(super) file_name: String,
    pub(super) contents: Vec<u8>,
}

pub(super) fn write_project_archive(
    topology: Vec<u8>,
    project_files: Vec<(String, Vec<u8>)>,
) -> Result<Vec<u8>, ApiError> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    writer
        .start_file("project.gns3", options)
        .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    writer
        .write_all(&topology)
        .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    for (file_path, contents) in project_files {
        let file_path = clean_relative_path(&file_path)?;
        writer
            .start_file(file_path, options)
            .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        writer
            .write_all(&contents)
            .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    }
    writer
        .finish()
        .map(|cursor| cursor.into_inner())
        .map_err(|error| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

pub(super) fn read_project_archive(bytes: &[u8]) -> Result<ImportedProjectArchive, ApiError> {
    if !bytes.starts_with(b"PK") {
        return serde_json::from_slice(bytes)
            .map(|project| ImportedProjectArchive {
                project: normalize_imported_project(project),
                project_files: Vec::new(),
                images: Vec::new(),
            })
            .map_err(|error| {
                archive_bad_request(format!("Invalid .gns3project archive or JSON: {error}"))
            });
    }

    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| archive_bad_request(format!("Invalid .gns3project ZIP: {error}")))?;
    let mut topology = None;
    let mut project_files = Vec::new();
    let mut images = Vec::new();
    let mut expanded_size = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            archive_bad_request(format!("Invalid .gns3project ZIP entry: {error}"))
        })?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_owned();
        let project_file_path = archive_project_file_path(&name)?;
        let image_path = archive_project_image_path(&name)?;
        let is_topology = name == "project.gns3" || (topology.is_none() && name.ends_with(".gns3"));
        if !is_topology && project_file_path.is_none() && image_path.is_none() {
            continue;
        }
        let advertised_size = entry.size();
        let remaining = MAX_PROJECT_ARCHIVE_FILES_BYTES.saturating_sub(expanded_size);
        let max_size = if is_topology {
            MAX_PROJECT_FILE_BYTES.min(remaining)
        } else {
            MAX_PROJECT_FILE_BYTES.min(remaining)
        };
        let contents = read_bounded_archive_entry(&mut entry, advertised_size, max_size)?;
        expanded_size = expanded_size.saturating_add(contents.len() as u64);
        if is_topology {
            if topology.is_some() && name == "project.gns3" {
                return Err(archive_bad_request(
                    "Project archive contains more than one project.gns3 topology file",
                ));
            }
            topology = Some(contents);
        } else if let Some(path) = project_file_path {
            project_files.push((path, contents));
        } else if let Some((image_type, file_name)) = image_path {
            images.push(ImportedProjectImage {
                image_type,
                file_name,
                contents,
            });
        }
    }
    let topology = topology
        .ok_or_else(|| archive_bad_request("Project archive contains no .gns3 topology file"))?;
    let project = serde_json::from_slice(&topology).map_err(|error| {
        archive_bad_request(format!("Project archive topology is invalid: {error}"))
    })?;
    Ok(ImportedProjectArchive {
        project: normalize_imported_project(project),
        project_files,
        images,
    })
}

pub(super) fn read_bounded_archive_entry(
    entry: &mut impl Read,
    advertised_size: u64,
    max_size: u64,
) -> Result<Vec<u8>, ApiError> {
    if advertised_size > max_size {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "Project archive contents exceed the 128 MiB expanded size limit",
        ));
    }
    let mut contents = Vec::with_capacity(advertised_size as usize);
    entry
        .take(max_size.saturating_add(1))
        .read_to_end(&mut contents)
        .map_err(|error| {
            archive_bad_request(format!("Could not read project archive entry: {error}"))
        })?;
    if contents.len() as u64 > max_size || contents.len() as u64 != advertised_size {
        return Err(archive_bad_request("Project archive entry size is invalid"));
    }
    Ok(contents)
}

pub(super) fn archive_project_file_path(name: &str) -> Result<Option<String>, ApiError> {
    let Some(path) = name
        .strip_prefix("project-files/")
        .or_else(|| name.strip_prefix("project_files/"))
    else {
        return Ok(None);
    };
    if path.is_empty() || path.starts_with('/') || name.ends_with('/') || path.contains('\\') {
        return Err(archive_bad_request(
            "Project archive contains an invalid project file path",
        ));
    }
    clean_relative_path(path)
        .map(Some)
        .map_err(|error| archive_bad_request(error.message))
}

pub(super) fn archive_project_image_path(name: &str) -> Result<Option<(String, String)>, ApiError> {
    let Some(path) = name.strip_prefix("images/") else {
        return Ok(None);
    };
    let Some((directory, file_name)) = path.split_once('/') else {
        return Ok(None);
    };
    let image_type = match directory.to_ascii_lowercase().as_str() {
        "qemu" => "qemu",
        "ios" | "dynamips" => "dynamips",
        _ => return Ok(None),
    };
    if file_name.contains('/') || file_name.is_empty() {
        return Err(archive_bad_request(
            "Project archive image entries must use a simple file name",
        ));
    }
    let safe_name =
        clean_relative_path(file_name).map_err(|error| archive_bad_request(error.message))?;
    if safe_name.contains('/') {
        return Err(archive_bad_request(
            "Project archive image entries must use a simple file name",
        ));
    }
    Ok(Some((image_type.to_owned(), safe_name)))
}

pub(super) fn install_imported_project_images(
    images: &[ImportedProjectImage],
) -> Result<HashMap<(String, String), String>, ApiError> {
    let mut installed = HashMap::new();
    for image in images {
        let directory = match image.image_type.as_str() {
            "qemu" => crate::image_paths::qemu_images(),
            "dynamips" => crate::image_paths::dynamips_images(),
            _ => continue,
        };
        std::fs::create_dir_all(&directory).map_err(|error| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Could not create imported image directory: {error}"),
            )
        })?;
        let destination_name =
            imported_image_destination_name(&directory, &image.file_name, &image.contents)?;
        let destination = directory.join(&destination_name);
        if !destination.exists() {
            let temporary = directory.join(format!(".import-{}.tmp", Uuid::new_v4()));
            let result = (|| -> std::io::Result<()> {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temporary)?;
                file.write_all(&image.contents)?;
                file.sync_all()?;
                std::fs::rename(&temporary, &destination)
            })();
            if let Err(error) = result {
                let _ = std::fs::remove_file(&temporary);
                if !destination.is_file() {
                    return Err(ApiError::new(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!(
                            "Could not install imported image '{}': {error}",
                            image.file_name
                        ),
                    ));
                }
            }
        }
        installed.insert(
            (image.image_type.clone(), image.file_name.clone()),
            destination_name,
        );
    }
    Ok(installed)
}

pub(super) fn imported_image_destination_name(
    directory: &Path,
    file_name: &str,
    contents: &[u8],
) -> Result<String, ApiError> {
    let safe_name = clean_relative_path(file_name)?;
    if safe_name.contains('/') {
        return Err(archive_bad_request(
            "Imported image file names must be simple file names",
        ));
    }
    let requested = directory.join(&safe_name);
    if !requested.exists() || std::fs::read(&requested).ok().as_deref() == Some(contents) {
        return Ok(safe_name);
    }
    let path = Path::new(&safe_name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("image");
    let extension = path.extension().and_then(|value| value.to_str());
    let digest = format!("{:x}", md5::Md5::digest(contents));
    for index in 0..1000 {
        let suffix = if index == 0 {
            format!("-import-{}", &digest[..8])
        } else {
            format!("-import-{}-{index}", &digest[..8])
        };
        let candidate = match extension {
            Some(extension) => format!("{stem}{suffix}.{extension}"),
            None => format!("{stem}{suffix}"),
        };
        let path = directory.join(&candidate);
        if !path.exists() || std::fs::read(&path).ok().as_deref() == Some(contents) {
            return Ok(candidate);
        }
    }
    Err(ApiError::conflict(
        "Could not allocate a unique name for an imported image",
    ))
}

pub(super) fn remap_imported_project_images(
    project: &mut Value,
    image_map: &HashMap<(String, String), String>,
) {
    let Some(nodes) = project.get_mut("nodes").and_then(Value::as_array_mut) else {
        return;
    };
    for node in nodes {
        let image_type = match node.get("node_type").and_then(Value::as_str) {
            Some("qemu") => "qemu",
            Some("dynamips") => "dynamips",
            _ => continue,
        };
        let fields: &[&str] = if image_type == "qemu" {
            &[
                "hda_disk_image",
                "hdb_disk_image",
                "hdc_disk_image",
                "hdd_disk_image",
                "cdrom_image",
            ]
        } else {
            &["image"]
        };
        for field in fields {
            let Some(reference) = node.get(*field).and_then(Value::as_str) else {
                continue;
            };
            let file_name = Path::new(reference)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(reference)
                .to_owned();
            if let Some(imported_name) = image_map.get(&(image_type.to_owned(), file_name)) {
                node[*field] = json!(imported_name);
            }
        }
    }
}

pub(super) fn regenerate_imported_project_ids(project: &mut Value, project_id: &str) {
    project["project_id"] = json!(project_id);
    let mut node_ids = HashMap::new();
    if let Some(nodes) = project.get_mut("nodes").and_then(Value::as_array_mut) {
        for node in nodes {
            let old_id = node
                .get("node_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let new_id = Uuid::new_v4().to_string();
            if let Some(old_id) = old_id {
                node_ids.insert(old_id, new_id.clone());
            }
            node["node_id"] = json!(new_id);
            node["project_id"] = json!(project_id);
            node["compute_id"] = json!("local");
            node["status"] = json!("stopped");
            strip_node_runtime_fields(node);
        }
    }
    if let Some(links) = project.get_mut("links").and_then(Value::as_array_mut) {
        for link in links {
            link["link_id"] = json!(Uuid::new_v4());
            link["project_id"] = json!(project_id);
            if let Some(endpoints) = link.get_mut("nodes").and_then(Value::as_array_mut) {
                for endpoint in endpoints {
                    if let Some(old_id) = endpoint.get("node_id").and_then(Value::as_str)
                        && let Some(new_id) = node_ids.get(old_id)
                    {
                        endpoint["node_id"] = json!(new_id);
                    }
                }
            }
            strip_link_runtime_fields(link);
        }
    }
    if let Some(drawings) = project.get_mut("drawings").and_then(Value::as_array_mut) {
        for drawing in drawings {
            drawing["drawing_id"] = json!(Uuid::new_v4());
            drawing["project_id"] = json!(project_id);
        }
    }
}

pub(super) fn archive_bad_request(message: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, message)
}

pub(super) fn strip_node_runtime_fields(node: &mut Value) {
    let Some(object) = node.as_object_mut() else {
        return;
    };
    object.retain(|key, _| {
        let key = key.to_ascii_lowercase();
        !key.starts_with("runtime_")
            && !matches!(
                key.as_str(),
                "container_id"
                    | "container_name"
                    | "container_pid"
                    | "image_digest"
                    | "dynamips_id"
                    | "pid"
                    | "pid_file"
                    | "qemu_pid"
                    | "qmp"
                    | "qmp_socket"
                    | "qmp_path"
                    | "serial"
                    | "serial_socket"
                    | "serial_path"
                    | "overlay"
                    | "overlay_path"
                    | "log"
                    | "log_file"
                    | "log_path"
                    | "working_dir"
                    | "working_directory"
                    | "hypervisor_port"
                    | "console"
                    | "aux"
                    | "console_host"
                    | "console_port"
                    | "aux_port"
            )
            && !key.ends_with("_pid")
            && !key.ends_with("_pid_file")
            && !key.ends_with("_qmp_socket")
            && !key.ends_with("_qmp_path")
            && !key.ends_with("_qmp")
            && !key.ends_with("_serial")
            && !key.ends_with("_serial_socket")
            && !key.ends_with("_serial_path")
            && !key.ends_with("_overlay")
            && !key.ends_with("_overlay_path")
            && !key.ends_with("_log_file")
            && !key.ends_with("_log_path")
            && !key.ends_with("_working_dir")
            && !key.ends_with("_working_directory")
            && !key.ends_with("_hypervisor_port")
            && !key.ends_with("_container_id")
    });
}

pub(super) fn strip_link_runtime_fields(link: &mut Value) {
    if let Some(endpoints) = link.get_mut("nodes").and_then(Value::as_array_mut) {
        for endpoint in endpoints {
            if let Some(endpoint) = endpoint.as_object_mut() {
                endpoint.retain(|key, _| !key.starts_with("runtime_"));
            }
        }
    }
    if let Some(object) = link.as_object_mut() {
        object.insert("capturing".into(), json!(false));
        object.insert("capture_file_name".into(), Value::Null);
        object.insert("capture_file_path".into(), Value::Null);
        object.insert("capture_endpoint_id".into(), Value::Null);
        object.insert("wireshark".into(), json!(false));
    }
}

pub(super) fn strip_project_runtime_fields(project: &mut Value) {
    let nodes = if project.get("topology").is_some() {
        project
            .get_mut("topology")
            .and_then(|topology| topology.get_mut("nodes"))
    } else {
        project.get_mut("nodes")
    };
    if let Some(nodes) = nodes.and_then(Value::as_array_mut) {
        for node in nodes {
            strip_node_runtime_fields(node);
            if node.get("status").is_some() {
                node["status"] = json!("stopped");
            }
        }
    }
    let links = if project.get("topology").is_some() {
        project
            .get_mut("topology")
            .and_then(|topology| topology.get_mut("links"))
    } else {
        project.get_mut("links")
    };
    if let Some(links) = links.and_then(Value::as_array_mut) {
        for link in links {
            strip_link_runtime_fields(link);
        }
    }
}
