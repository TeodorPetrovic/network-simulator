use std::os::unix::fs::PermissionsExt;
use std::{env, fs, io, path::PathBuf};

pub fn application_data_directory() -> PathBuf {
    if let Some(path) = env::var_os("NETWORK_SIMULATOR_DATA_HOME") {
        return PathBuf::from(path);
    }
    let base = env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("data"));
    base.join("network-simulator")
}

pub fn image_root() -> PathBuf {
    image_root_from(
        env::var_os("NETWORK_SIMULATOR_IMAGE_HOME").map(PathBuf::from),
        application_data_directory(),
    )
}

fn image_root_from(custom: Option<PathBuf>, app_data: PathBuf) -> PathBuf {
    custom.unwrap_or_else(|| app_data.join("images"))
}

pub fn qemu_images() -> PathBuf {
    env::var_os("NETWORK_SIMULATOR_QEMU_IMAGES")
        .map(PathBuf::from)
        .unwrap_or_else(|| image_root().join("qemu"))
}

pub fn dynamips_images() -> PathBuf {
    env::var_os("NETWORK_SIMULATOR_DYNAMIPS_IMAGES")
        .map(PathBuf::from)
        .unwrap_or_else(|| image_root().join("dynamips"))
}

pub fn appliance_definitions() -> PathBuf {
    env::var_os("NETWORK_SIMULATOR_APPLIANCES")
        .map(PathBuf::from)
        .unwrap_or_else(|| application_data_directory().join("appliances"))
}

pub fn runtime_binary(variable: &str, program: &str) -> PathBuf {
    if let Some(path) = env::var_os(variable) {
        return PathBuf::from(path);
    }
    let local_binary = application_data_directory().join("bin").join(program);
    if local_binary.is_file() {
        local_binary
    } else {
        PathBuf::from(program)
    }
}

pub fn runtime_binary_prefer_path(variable: &str, program: &str) -> PathBuf {
    if let Some(path) = env::var_os(variable) {
        return PathBuf::from(path);
    }
    if let Some(path) = env::var_os("PATH")
        .into_iter()
        .flat_map(|path| env::split_paths(&path).collect::<Vec<_>>())
        .map(|directory| directory.join(program))
        .find(|path| {
            fs::metadata(path)
                .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        })
    {
        return path;
    }
    let local_binary = application_data_directory().join("bin").join(program);
    if local_binary.is_file() {
        local_binary
    } else {
        PathBuf::from(program)
    }
}

pub fn create_default_directories() -> io::Result<()> {
    for directory in [qemu_images(), dynamips_images(), appliance_definitions()] {
        fs::create_dir_all(directory)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::image_root_from;
    use std::path::PathBuf;

    #[test]
    fn custom_image_home_is_the_shared_parent_for_runtime_image_directories() {
        let custom = PathBuf::from("/custom/net-images");
        assert_eq!(
            image_root_from(Some(custom.clone()), PathBuf::from("/app-data")),
            custom
        );
        assert_eq!(
            image_root_from(None, PathBuf::from("/app-data")),
            PathBuf::from("/app-data/images")
        );
    }
}
