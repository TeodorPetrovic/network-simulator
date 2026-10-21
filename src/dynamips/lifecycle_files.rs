use super::*;

pub(super) fn remove_auto_delete_disk_files(
    working_dir: &Path,
    platform: &str,
    dynamips_id: u32,
) -> Result<(), RuntimeError> {
    if !working_dir.exists() {
        return Ok(());
    }
    let entries = fs::read_dir(working_dir).map_err(|error| {
        RuntimeError::Failed(format!("Could not inspect Dynamips disk files: {error}"))
    })?;
    let removable_names = [
        format!("{platform}_i{dynamips_id}_disk0"),
        format!("{platform}_i{dynamips_id}_disk1"),
        format!("{platform}_i{dynamips_id}_slot0"),
        format!("{platform}_i{dynamips_id}_slot1"),
        format!("{platform}_i{dynamips_id}_nvram"),
        format!("{platform}_i{dynamips_id}_flash0"),
        format!("{platform}_i{dynamips_id}_flash1"),
        format!("{platform}_i{dynamips_id}_rom"),
        format!("{platform}_i{dynamips_id}_bootflash"),
        format!("{platform}_i{dynamips_id}_ssa"),
    ];
    for entry in entries {
        let entry = entry.map_err(|error| {
            RuntimeError::Failed(format!("Could not inspect Dynamips disk files: {error}"))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            RuntimeError::Failed(format!("Could not inspect Dynamips disk file: {error}"))
        })?;
        if file_type.is_file()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| removable_names.iter().any(|candidate| candidate == name))
        {
            fs::remove_file(entry.path()).map_err(|error| {
                RuntimeError::Failed(format!("Could not remove Dynamips disk file: {error}"))
            })?;
        }
    }
    Ok(())
}

pub(super) fn remove_transient_runtime_files(working_dir: &Path) -> Result<(), RuntimeError> {
    if !working_dir.exists() {
        return Ok(());
    }
    let entries = fs::read_dir(working_dir).map_err(|error| {
        RuntimeError::Failed(format!("Could not inspect Dynamips runtime files: {error}"))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            RuntimeError::Failed(format!("Could not inspect Dynamips runtime file: {error}"))
        })?;
        if !entry
            .file_type()
            .map_err(|error| {
                RuntimeError::Failed(format!("Could not inspect Dynamips runtime file: {error}"))
            })?
            .is_file()
        {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name == "dynamips_stdout.log"
            || name == "dynamips_engine.log"
            || name.starts_with("ilt_")
            || is_vm_transient_file(name)
        {
            fs::remove_file(entry.path()).map_err(|error| {
                RuntimeError::Failed(format!("Could not remove Dynamips runtime file: {error}"))
            })?;
        }
    }
    Ok(())
}

/// Per-instance files Dynamips recreates on every `vm create`/`vm start`
/// (`<platform>_i<id>_lock`, `_log.txt`, `_rommon_vars`). GNS3 deletes the same
/// set when a project closes; NVRAM, disks, and flash stay with the node.
pub(super) fn is_vm_transient_file(name: &str) -> bool {
    let Some((instance, suffix)) = name
        .strip_prefix('c')
        .and_then(|rest| rest.split_once("_i"))
        .and_then(|(model, rest)| {
            (model.len() == 4 && model.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| rest.split_once('_'))
                .flatten()
        })
    else {
        return false;
    };
    !instance.is_empty()
        && instance.bytes().all(|byte| byte.is_ascii_digit())
        && matches!(suffix, "lock" | "log.txt" | "rommon_vars")
}

pub(super) fn remove_runtime_manifest(working_dir: &Path) -> Result<(), RuntimeError> {
    match fs::remove_file(working_dir.join(RUNTIME_MANIFEST_FILE)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(RuntimeError::Failed(format!(
            "Could not remove Dynamips runtime ownership marker: {error}"
        ))),
    }
}

pub(super) fn remove_empty_runtime_directories(working_dir: &Path) -> Result<(), RuntimeError> {
    for directory in [working_dir.join("configs"), working_dir.to_path_buf()] {
        match fs::remove_dir(&directory) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) => {
                return Err(RuntimeError::Failed(format!(
                    "Could not remove empty Dynamips runtime directory '{}': {error}",
                    directory.display()
                )));
            }
        }
    }
    if let Some(project_dir) = working_dir.parent() {
        match fs::remove_dir(project_dir) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::NotFound | ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) => {
                return Err(RuntimeError::Failed(format!(
                    "Could not remove empty Dynamips project directory '{}': {error}",
                    project_dir.display()
                )));
            }
        }
    }
    Ok(())
}

pub(super) fn remove_generated_runtime_configs(working_dir: &Path) -> Result<(), RuntimeError> {
    let configs = working_dir.join("configs");
    match fs::remove_dir_all(&configs) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(RuntimeError::Failed(format!(
            "Could not remove generated Dynamips configs '{}': {error}",
            configs.display()
        ))),
    }
}
