use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, UNIX_EPOCH},
};

use md5::Md5;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha1::{Digest, Sha1};
use tokio::{io::AsyncWriteExt, sync::Mutex};
use uuid::Uuid;

const ISHARE_RELEASE_API: &str = "https://api.github.com/repos/ishare2-org/mirrors/releases/latest";
const GNS3_APPLIANCES_API: &str =
    "https://api.github.com/repos/GNS3/gns3-registry/contents/appliances";
const GNS3_RAW_APPLIANCES: &str =
    "https://raw.githubusercontent.com/GNS3/gns3-registry/master/appliances";
const CACHE_TTL: Duration = Duration::from_secs(30 * 60);
const MAX_INDEX_BYTES: usize = 32 * 1024 * 1024;
const MAX_APPLIANCE_BYTES: usize = 2 * 1024 * 1024;
const MAX_SEARCH_RESULTS: usize = 100;
const MAX_GNS3_APPLIANCE_RESULTS: usize = 500;
const MAX_ISHARE_FILE_BYTES: u64 = 64 * 1024 * 1024 * 1024;

#[derive(Clone)]
pub struct MarketplaceCatalog {
    client: Client,
    download_client: Client,
    appliance_dir: PathBuf,
    cache: Arc<Mutex<CatalogCache>>,
    image_hashes: Arc<Mutex<HashMap<PathBuf, CachedImageHash>>>,
    binding_lock: Arc<Mutex<()>>,
}

#[derive(Default)]
struct CatalogCache {
    ishare_index: Option<Cached<Value>>,
    gns3_files: Option<Cached<Vec<GithubContent>>>,
}

#[derive(Clone)]
struct Cached<T> {
    loaded_at: Instant,
    value: T,
}

#[derive(Clone, Debug, Deserialize)]
struct GithubContent {
    name: String,
    path: String,
    html_url: String,
    download_url: Option<String>,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct GithubRelease {
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Serialize)]
struct IshareImage {
    id: u64,
    name: String,
    image_type: String,
    size: String,
    total_size: u64,
    file_count: usize,
    files: Vec<String>,
    catalog: &'static str,
}

#[derive(Serialize)]
pub struct IshareDownloadResult {
    pub id: u64,
    pub name: String,
    pub image_type: String,
    pub files: Vec<String>,
    pub source_files: Vec<String>,
    pub image_directory: String,
}

struct InstalledIshareFiles {
    files: Vec<String>,
    source_files: Vec<String>,
}

#[derive(Serialize)]
pub struct ApplianceFile {
    pub filename: String,
    pub version: Option<String>,
    pub size: Option<u64>,
    pub md5sum: Option<String>,
    pub download_url: Option<String>,
}

#[derive(Serialize)]
pub struct ApplianceRecord {
    pub file_name: String,
    pub appliance_id: String,
    pub name: String,
    pub origin: String,
    pub category: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub registry_version: Option<u64>,
    pub template_type: Option<String>,
    pub imported: bool,
    pub source_url: String,
    pub required_files: Vec<ApplianceFile>,
    pub versions: Vec<ApplianceVersion>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ApplianceVersion {
    pub name: String,
    pub settings: Option<String>,
    pub template_type: Option<String>,
    pub runtime_supported: bool,
    pub docker_image: Option<String>,
    pub docker_image_id: Option<String>,
    pub docker_image_available: bool,
    pub images: Vec<ApplianceImageRequirement>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ApplianceImageRequirement {
    pub key: String,
    pub filename: String,
    pub assignable: bool,
    pub filesize: Option<u64>,
    pub md5sum: Option<String>,
    pub assigned_image_id: Option<String>,
    pub assigned_image_name: Option<String>,
    pub suggested_image_id: Option<String>,
    pub suggested_image_name: Option<String>,
    pub image_path: Option<String>,
    pub actual_size: Option<u64>,
    pub actual_md5sum: Option<String>,
    pub actual_sha1sum: Option<String>,
    pub status: String,
    pub allow_checksum_mismatch: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct LocalApplianceImage {
    pub image_id: String,
    pub name: String,
    pub template_type: String,
    pub path: String,
    pub size: u64,
    pub md5sum: Option<String>,
    pub sha1sum: Option<String>,
}

#[derive(Serialize)]
pub struct ApplianceManagerState {
    pub appliances: Vec<ApplianceRecord>,
    pub local_images: Vec<LocalApplianceImage>,
    pub custom_image_options: Vec<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PersistedImageBinding {
    version: String,
    image_key: String,
    image_id: String,
    image_path: String,
    allow_checksum_mismatch: bool,
}

#[derive(Clone)]
struct CachedImageHash {
    size: u64,
    modified_ns: u128,
    md5sum: String,
    sha1sum: String,
}

impl MarketplaceCatalog {
    pub fn new() -> Self {
        Self::with_directory(crate::image_paths::appliance_definitions())
    }

    pub(crate) fn with_directory(appliance_dir: impl Into<PathBuf>) -> Self {
        Self::build(appliance_dir.into())
    }

    fn build(appliance_dir: PathBuf) -> Self {
        let client = Client::builder()
            .user_agent(concat!("network-simulator/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(20))
            .build()
            .expect("the static marketplace HTTP client configuration is valid");
        let download_client = Client::builder()
            .user_agent(concat!("network-simulator/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("the static image download HTTP client configuration is valid");
        Self {
            client,
            download_client,
            appliance_dir,
            cache: Arc::new(Mutex::new(CatalogCache::default())),
            image_hashes: Arc::new(Mutex::new(HashMap::new())),
            binding_lock: Arc::new(Mutex::new(())),
        }
    }

    pub async fn search_ishare2(
        &self,
        query: &str,
        image_type: &str,
    ) -> Result<Vec<Value>, String> {
        let query = validate_query(query)?;
        let image_type = image_type.to_ascii_lowercase();
        if !matches!(image_type.as_str(), "all" | "qemu" | "dynamips" | "iol") {
            return Err("iShare2 image type must be all, qemu, dynamips, or iol".to_owned());
        }
        let index = self.ishare_index().await?;
        let categories: &[&str] = match image_type.as_str() {
            "all" => &["QEMU", "DYNAMIPS", "IOL"],
            "qemu" => &["QEMU"],
            "dynamips" => &["DYNAMIPS"],
            "iol" => &["IOL"],
            _ => unreachable!("image type was validated"),
        };
        let needle = query.to_lowercase();
        let mut images = Vec::new();
        for category in categories {
            let Some(rows) = index.get(*category).and_then(Value::as_array) else {
                continue;
            };
            for row in rows {
                let Some(name) = row.get("name").and_then(Value::as_str) else {
                    continue;
                };
                if !ishare_row_matches_query(row, &needle) {
                    continue;
                }
                let metadata = &row["metadata"];
                images.push(
                    serde_json::to_value(IshareImage {
                        id: row["id"].as_u64().unwrap_or_default(),
                        name: name.to_owned(),
                        image_type: row["type"]
                            .as_str()
                            .unwrap_or(category)
                            .to_ascii_lowercase(),
                        size: metadata["total_human_size"]
                            .as_str()
                            .unwrap_or("size unknown")
                            .to_owned(),
                        total_size: metadata["total_size"].as_u64().unwrap_or_default(),
                        file_count: row["files"].as_array().map_or(0, Vec::len),
                        files: row["files"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|file| file["filename"].as_str().map(str::to_owned))
                            .collect(),
                        catalog: "iShare2 / LabHub",
                    })
                    .map_err(|error| format!("Could not encode iShare2 result: {error}"))?,
                );
                if images.len() >= MAX_SEARCH_RESULTS {
                    return Ok(images);
                }
            }
        }
        Ok(images)
    }

    pub async fn download_ishare2(
        &self,
        image_type: &str,
        image_id: u64,
    ) -> Result<IshareDownloadResult, String> {
        let image_type = image_type.to_ascii_lowercase();
        let category = match image_type.as_str() {
            "qemu" => "QEMU",
            "dynamips" => "DYNAMIPS",
            "iol" => {
                return Err("IOL images are not supported by an installed runtime".to_owned());
            }
            _ => return Err("Image type must be qemu or dynamips".to_owned()),
        };
        let index = self.ishare_index().await?;
        let row = index
            .get(category)
            .and_then(Value::as_array)
            .and_then(|images| {
                images
                    .iter()
                    .find(|image| image["id"].as_u64() == Some(image_id))
            })
            .ok_or_else(|| format!("iShare2 {image_type} image {image_id} was not found"))?;
        let name = row["name"]
            .as_str()
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| "The iShare2 image has no name".to_owned())?;
        let files = row["files"]
            .as_array()
            .filter(|files| !files.is_empty())
            .ok_or_else(|| "The iShare2 image has no downloadable files".to_owned())?;
        let url_properties = index
            .get("url_properties")
            .ok_or_else(|| "The iShare2 index has no mirror configuration".to_owned())?;
        let protocol = url_properties["protocol"]
            .as_str()
            .filter(|protocol| *protocol == "https")
            .ok_or_else(|| "The iShare2 index does not provide HTTPS downloads".to_owned())?;
        let safe_name = safe_catalog_component(name)?;
        let image_directory = match image_type.as_str() {
            "qemu" => crate::image_paths::qemu_images(),
            "dynamips" => crate::image_paths::dynamips_images(),
            _ => unreachable!("image type was validated"),
        };
        fs::create_dir_all(&image_directory).map_err(|error| {
            format!(
                "Could not create global {image_type} image directory '{}': {error}",
                image_directory.display()
            )
        })?;

        let mut last_error = None;
        for mirror in ["drive", "main"] {
            let Some(host) = url_properties["hostnames"][mirror].as_str() else {
                continue;
            };
            let Some(prefix) = url_properties["prefixes"][mirror].as_str() else {
                continue;
            };
            if !valid_ishare_host(host) || !valid_ishare_prefix(prefix) {
                continue;
            }
            match self
                .download_ishare_attempt(
                    files,
                    protocol,
                    host,
                    prefix,
                    &image_type,
                    image_id,
                    &safe_name,
                    &image_directory,
                )
                .await
            {
                Ok(installed_files) => {
                    return Ok(IshareDownloadResult {
                        id: image_id,
                        name: name.to_owned(),
                        image_type,
                        files: installed_files.files,
                        source_files: installed_files.source_files,
                        image_directory: image_directory.display().to_string(),
                    });
                }
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.unwrap_or_else(|| {
            "The iShare2 index has no supported HTTPS mirrors for this image".to_owned()
        }))
    }

    async fn download_ishare_attempt(
        &self,
        files: &[Value],
        protocol: &str,
        host: &str,
        prefix: &str,
        image_type: &str,
        image_id: u64,
        safe_name: &str,
        image_directory: &Path,
    ) -> Result<InstalledIshareFiles, String> {
        let staging = image_directory.join(format!(".ishare-{image_id}-{}", Uuid::new_v4()));
        fs::create_dir(&staging).map_err(|error| {
            format!("Could not create image download staging directory: {error}")
        })?;
        let download = async {
            let mut staged_files = Vec::with_capacity(files.len());
            for file in files {
                let filename = file["filename"]
                    .as_str()
                    .filter(|filename| safe_download_filename(filename))
                    .ok_or_else(|| "The iShare2 manifest contains an unsafe filename".to_owned())?;
                let path = file["path"]
                    .as_str()
                    .filter(|path| safe_ishare_path(path))
                    .ok_or_else(|| "The iShare2 manifest contains an unsafe download path".to_owned())?;
                let url = format!("{protocol}://{host}{prefix}{path}");
                let target = staging.join(filename);
                let downloaded = self
                    .download_verified_file(&url, &target, file)
                    .await?;
                staged_files.push((filename.to_owned(), downloaded));
            }

            let mut to_install = Vec::new();
            for (filename, staged_path) in staged_files {
                let extension = Path::new(&filename)
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                let (install_name, validate_ios) = match image_type {
                    "qemu" if is_supported_qemu_extension(&extension) => {
                        (format!("{safe_name}-{filename}"), false)
                    }
                    "qemu" => continue,
                    "dynamips" if files.len() == 1 => {
                        let prefixed = format!("ishare-{image_id}-{filename}");
                        (
                            if prefixed.len() <= 255 {
                                prefixed
                            } else {
                                filename.to_owned()
                            },
                            true,
                        )
                    }
                    "dynamips" => {
                        return Err("The selected Dynamips image contains multiple files; install it with the upstream iShare2 CLI".to_owned());
                    }
                    _ => unreachable!("image type was validated"),
                };
                if validate_ios {
                    crate::dynamips::validate_ios_image(&staged_path).map_err(|error| {
                        format!("Downloaded iShare2 image failed Dynamips IOS validation: {}", error.message())
                    })?;
                }
                let destination = image_directory.join(install_name);
                if destination.exists() {
                    return Err(format!(
                        "Global image '{}' already exists",
                        destination.file_name().unwrap_or_default().to_string_lossy()
                    ));
                }
                to_install.push((staged_path, destination, filename));
            }
            if to_install.is_empty() {
                return Err(format!(
                    "The selected iShare2 {image_type} package contains no supported disk image file"
                ));
            }
            let mut installed_paths = Vec::new();
            for (staged_path, destination, _) in &to_install {
                if let Err(error) = fs::hard_link(staged_path, destination) {
                    for installed in &installed_paths {
                        let _ = fs::remove_file(installed);
                    }
                    return Err(format!("Could not install verified image: {error}"));
                }
                installed_paths.push(destination.clone());
            }
            let source_files = to_install
                .iter()
                .map(|(_, _, source)| source.to_owned())
                .collect();
            let installed = to_install
                .into_iter()
                .map(|(_, path, _)| {
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect();
            let _ = fs::remove_dir_all(&staging);
            Ok(InstalledIshareFiles {
                files: installed,
                source_files,
            })
        }
        .await;
        if download.is_err() {
            let _ = fs::remove_dir_all(&staging);
        }
        download
    }

    async fn download_verified_file(
        &self,
        url: &str,
        target: &Path,
        manifest: &Value,
    ) -> Result<PathBuf, String> {
        let response = self
            .download_client
            .get(url)
            .send()
            .await
            .map_err(|error| format!("Could not download iShare2 image file: {error}"))?
            .error_for_status()
            .map_err(|error| format!("iShare2 image download failed: {error}"))?;
        if response
            .content_length()
            .is_some_and(|length| length > MAX_ISHARE_FILE_BYTES)
        {
            return Err("The iShare2 image file exceeds the 64 GiB download limit".to_owned());
        }
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)
            .await
            .map_err(|error| format!("Could not create image staging file: {error}"))?;
        let mut total = 0_u64;
        let mut md5 = Md5::new();
        let mut sha1 = Sha1::new();
        let mut response = response;
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|error| format!("Image download stream failed: {error}"))?
        {
            total = total
                .checked_add(chunk.len() as u64)
                .filter(|total| *total <= MAX_ISHARE_FILE_BYTES)
                .ok_or_else(|| {
                    "The iShare2 image file exceeds the 64 GiB download limit".to_owned()
                })?;
            md5.update(&chunk);
            sha1.update(&chunk);
            file.write_all(&chunk)
                .await
                .map_err(|error| format!("Could not write image download: {error}"))?;
        }
        file.sync_all()
            .await
            .map_err(|error| format!("Could not flush image download: {error}"))?;
        drop(file);

        if let Some(expected) = manifest["size"].as_u64()
            && expected != total
        {
            return Err(format!(
                "iShare2 image size check failed (expected {expected} bytes, received {total})"
            ));
        }
        let expected_md5 = manifest["checksum"]["md5"].as_str().unwrap_or_default();
        if !expected_md5.is_empty() && format!("{:x}", md5.finalize()) != expected_md5 {
            return Err("iShare2 image MD5 check failed".to_owned());
        }
        let expected_sha1 = manifest["checksum"]["sha1"].as_str().unwrap_or_default();
        if !expected_sha1.is_empty() && format!("{:x}", sha1.finalize()) != expected_sha1 {
            return Err("iShare2 image SHA1 check failed".to_owned());
        }
        if expected_md5.is_empty() && expected_sha1.is_empty() {
            return Err("The iShare2 manifest provides no checksum for this file".to_owned());
        }
        Ok(target.to_path_buf())
    }

    pub async fn search_gns3_appliances(&self, query: &str) -> Result<Vec<Value>, String> {
        let query = validate_query(query)?.to_lowercase();
        let entries = self.gns3_appliance_files().await?;
        let mut matches = entries
            .into_iter()
            .filter(|entry| {
                entry.kind == "file"
                    && entry.name.ends_with(".gns3a")
                    && (query.is_empty() || entry.name.to_lowercase().contains(&query))
            })
            .take(MAX_GNS3_APPLIANCE_RESULTS)
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
        let imported_names = self.imported_appliance_file_names();
        Ok(matches
            .into_iter()
            .map(|entry| {
                let slug = entry.name.trim_end_matches(".gns3a");
                let imported = imported_names.contains(&entry.name);
                json!({
                    "file_name": entry.name,
                    "name": title_from_slug(slug),
                    "registry_path": entry.path,
                    "source_url": entry.html_url,
                    "download_url": entry.download_url.unwrap_or_else(|| format!("{GNS3_RAW_APPLIANCES}/{}", entry.name)),
                    "imported": imported
                })
            })
            .collect())
    }

    pub async fn appliance_details(&self, file_name: &str) -> Result<ApplianceRecord, String> {
        let file_name = validate_appliance_file_name(file_name)?;
        let appliance = self.fetch_appliance(file_name).await?;
        let imported = self.imported_appliance_file_names().contains(file_name);
        let record = summarize_appliance(file_name, &appliance, imported)?;
        Ok(record)
    }

    pub async fn import_appliance(&self, file_name: &str) -> Result<ApplianceRecord, String> {
        let file_name = validate_appliance_file_name(file_name)?;
        let appliance = self.fetch_appliance(file_name).await?;
        let record = summarize_appliance(file_name, &appliance, true)?;
        fs::create_dir_all(&self.appliance_dir).map_err(|error| {
            format!(
                "Could not create appliance directory '{}': {error}",
                self.appliance_dir.display()
            )
        })?;
        let final_path = self.appliance_dir.join(file_name);
        if !final_path.exists() {
            let temporary_path =
                self.appliance_dir
                    .join(format!(".{}.{}.download", Uuid::new_v4(), file_name));
            let bytes = serde_json::to_vec_pretty(&appliance)
                .map_err(|error| format!("Could not encode appliance definition: {error}"))?;
            fs::write(&temporary_path, bytes)
                .map_err(|error| format!("Could not save appliance definition: {error}"))?;
            if let Err(error) = fs::hard_link(&temporary_path, &final_path) {
                let _ = fs::remove_file(&temporary_path);
                if error.kind() != ErrorKind::AlreadyExists {
                    return Err(format!("Could not install appliance definition: {error}"));
                }
            }
            let _ = fs::remove_file(temporary_path);
        }
        Ok(record)
    }

    pub fn import_appliance_file(
        &self,
        file_name: &str,
        bytes: &[u8],
    ) -> Result<ApplianceRecord, String> {
        let file_name = validate_appliance_file_name(file_name)?;
        let appliance: Value = serde_json::from_slice(bytes)
            .map_err(|error| format!("GNS3 appliance file is invalid JSON: {error}"))?;
        let mut record = summarize_appliance(file_name, &appliance, true)?;
        fs::create_dir_all(&self.appliance_dir).map_err(|error| {
            format!(
                "Could not create appliance directory '{}': {error}",
                self.appliance_dir.display()
            )
        })?;
        let final_path = self.appliance_dir.join(file_name);
        if final_path.exists() {
            let existing = fs::read(&final_path).map_err(|error| {
                format!("Could not inspect existing appliance definition: {error}")
            })?;
            let existing: Value = serde_json::from_slice(&existing).map_err(|error| {
                format!("Existing appliance definition is invalid JSON: {error}")
            })?;
            if existing.get("appliance_id").and_then(Value::as_str)
                != Some(record.appliance_id.as_str())
            {
                return Err(format!(
                    "A different appliance already uses the filename '{file_name}'"
                ));
            }
        }
        let temporary_path =
            self.appliance_dir
                .join(format!(".{}.{}.import", Uuid::new_v4(), file_name));
        fs::write(&temporary_path, bytes)
            .map_err(|error| format!("Could not save appliance definition: {error}"))?;
        fs::rename(&temporary_path, &final_path).map_err(|error| {
            let _ = fs::remove_file(&temporary_path);
            format!("Could not install appliance definition: {error}")
        })?;
        record.imported = true;
        Ok(record)
    }

    pub fn create_custom_appliance(
        &self,
        name: &str,
        source_template_id: &str,
        runtime_settings: &Value,
        image_options: &[Value],
    ) -> Result<ApplianceRecord, String> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 100 || name.chars().any(char::is_control) {
            return Err("Custom appliance name must contain 1 to 100 printable characters".into());
        }
        let source = image_options
            .iter()
            .find(|template| template["template_id"].as_str() == Some(source_template_id))
            .ok_or_else(|| "Select an image from the installed runtime inventory".to_owned())?;
        if source.get("appliance_id").is_some() {
            return Err("A custom appliance must be based on an installed runtime image".into());
        }
        let runtime = source["template_type"]
            .as_str()
            .filter(|runtime| matches!(*runtime, "qemu" | "dynamips" | "docker" | "vpcs"))
            .ok_or_else(|| {
                "The selected runtime cannot be used for a custom appliance".to_owned()
            })?;

        let properties = match runtime {
            "qemu" => {
                let disk = source["hda_disk_image"]
                    .as_str()
                    .ok_or_else(|| "The selected QEMU template has no disk image".to_owned())?;
                json!({
                    "hda_disk_image": image_file_name(disk),
                    "platform": source["platform"],
                    "ram": source["memory"].as_u64().unwrap_or(512),
                    "cpus": source["cpus"].as_u64().unwrap_or(1),
                    "adapters": source["adapters"].as_u64().unwrap_or(1),
                    "adapter_type": source["adapter_type"].as_str().unwrap_or("e1000"),
                    "hda_disk_interface": source["hda_disk_interface"].as_str().unwrap_or("ide"),
                    "boot_priority": source["boot_priority"].as_str().unwrap_or("c"),
                    "linked_clone": source["linked_clone"].as_bool().unwrap_or(true),
                    "maxcpus": source["maxcpus"].as_u64().unwrap_or(source["cpus"].as_u64().unwrap_or(1)),
                    "hdb_disk_image": source["hdb_disk_image"].as_str().unwrap_or(""),
                    "hdb_disk_interface": source["hdb_disk_interface"].as_str().unwrap_or("none"),
                    "hdc_disk_image": source["hdc_disk_image"].as_str().unwrap_or(""),
                    "hdc_disk_interface": source["hdc_disk_interface"].as_str().unwrap_or("none"),
                    "hdd_disk_image": source["hdd_disk_image"].as_str().unwrap_or(""),
                    "hdd_disk_interface": source["hdd_disk_interface"].as_str().unwrap_or("none"),
                    "cdrom_image": source["cdrom_image"].as_str().unwrap_or(""),
                    "bios_image": source["bios_image"].as_str().unwrap_or(""),
                    "kernel_image": source["kernel_image"].as_str().unwrap_or(""),
                    "initrd": source["initrd"].as_str().unwrap_or(""),
                    "kernel_command_line": source["kernel_command_line"].as_str().unwrap_or(""),
                    "console_type": source["console_type"].as_str().unwrap_or("telnet"),
                    "aux_type": source["aux_type"].as_str().unwrap_or("none"),
                    "on_close": source["on_close"].as_str().unwrap_or("power_off"),
                    "process_priority": source["process_priority"].as_str().unwrap_or("normal"),
                    "options": source["options"].as_str().unwrap_or(""),
                    "tpm": source["tpm"].as_bool().unwrap_or(false),
                    "uefi": source["uefi"].as_bool().unwrap_or(false),
                    "replicate_network_connection_state": source["replicate_network_connection_state"].as_bool().unwrap_or(true),
                    "create_config_disk": source["create_config_disk"].as_bool().unwrap_or(false),
                    "cpu_throttling": source["cpu_throttling"].as_u64().unwrap_or(0)
                })
            }
            "dynamips" => {
                let image = source["image"]
                    .as_str()
                    .ok_or_else(|| "The selected Dynamips template has no IOS image".to_owned())?;
                let platform = runtime_settings["platform"]
                    .as_str()
                    .or_else(|| source["platform"].as_str())
                    .unwrap_or("c7200");
                let chassis = runtime_settings["chassis"]
                    .as_str()
                    .or_else(|| source["chassis"].as_str())
                    .unwrap_or(match platform {
                        "c3600" => "3640",
                        "c2600" => "2610",
                        "c1700" => "1760",
                        _ => "",
                    });
                let source_slot0 = (source["platform"].as_str() == Some(platform))
                    .then(|| source["slot0"].as_str())
                    .flatten();
                let slot0 = runtime_settings["slot0"]
                    .as_str()
                    .or(source_slot0)
                    .unwrap_or_else(|| default_dynamips_slot0(platform, chassis));
                let mut settings = json!({
                    "image": image_file_name(image),
                    "platform": platform,
                    "ram": source["memory"].as_u64().unwrap_or(256),
                    "nvram": source["nvram"].as_u64().unwrap_or(match platform {
                        "c7200" => 512,
                        "c2691" | "c3725" | "c3745" => 256,
                        _ => 128,
                    }),
                    "slot0": slot0,
                    "idlepc": source["idlepc"].as_str().unwrap_or(""),
                    "adapters": source["adapters"].as_u64().unwrap_or(1),
                    "mmap": runtime_settings["mmap"].as_bool().unwrap_or(source["mmap"].as_bool().unwrap_or(true)),
                    "sparsemem": runtime_settings["sparsemem"].as_bool().unwrap_or(source["sparsemem"].as_bool().unwrap_or(!matches!(platform, "c1700" | "c2600"))),
                    "clock_divisor": runtime_settings["clock_divisor"].as_u64().unwrap_or(source["clock_divisor"].as_u64().unwrap_or(8)),
                    "idlemax": runtime_settings["idlemax"].as_u64().unwrap_or(source["idlemax"].as_u64().unwrap_or(500)),
                    "idlesleep": runtime_settings["idlesleep"].as_u64().unwrap_or(source["idlesleep"].as_u64().unwrap_or(30)),
                    "exec_area": runtime_settings["exec_area"].as_u64().unwrap_or(source["exec_area"].as_u64().unwrap_or(64)),
                    "disk0": runtime_settings["disk0"].as_u64().unwrap_or(source["disk0"].as_u64().unwrap_or(0)),
                    "disk1": runtime_settings["disk1"].as_u64().unwrap_or(source["disk1"].as_u64().unwrap_or(0)),
                    "auto_delete_disks": runtime_settings["auto_delete_disks"].as_bool().unwrap_or(source["auto_delete_disks"].as_bool().unwrap_or(false)),
                    "private_config": runtime_settings["private_config"].as_str().unwrap_or(source["private_config"].as_str().unwrap_or("")),
                    "mac_addr": runtime_settings["mac_addr"].as_str().unwrap_or(source["mac_addr"].as_str().unwrap_or("")),
                    "system_id": runtime_settings["system_id"].as_str().unwrap_or(source["system_id"].as_str().unwrap_or("FTX0945W0MY")),
                    "npe": runtime_settings["npe"].as_str().unwrap_or(source["npe"].as_str().unwrap_or("npe-400")),
                    "midplane": runtime_settings["midplane"].as_str().unwrap_or(source["midplane"].as_str().unwrap_or("vxr")),
                    "iomem": runtime_settings["iomem"].as_u64().unwrap_or(source["iomem"].as_u64().unwrap_or(match platform { "c1700" | "c2600" => 15, _ => 5 })),
                    "ghost_ios": runtime_settings["ghost_ios"].as_bool().unwrap_or(source["ghost_ios"].as_bool().unwrap_or(false)),
                    "console_type": runtime_settings["console_type"].as_str().unwrap_or(source["console_type"].as_str().unwrap_or("telnet")),
                    "aux_type": runtime_settings["aux_type"].as_str().unwrap_or(source["aux_type"].as_str().unwrap_or("none")),
                    "slot1": runtime_settings["slot1"].as_str().or(source["slot1"].as_str()),
                    "slot2": runtime_settings["slot2"].as_str().or(source["slot2"].as_str()),
                    "slot3": runtime_settings["slot3"].as_str().or(source["slot3"].as_str()),
                    "slot4": runtime_settings["slot4"].as_str().or(source["slot4"].as_str()),
                    "slot5": runtime_settings["slot5"].as_str().or(source["slot5"].as_str()),
                    "slot6": runtime_settings["slot6"].as_str().or(source["slot6"].as_str()),
                    "wic0": runtime_settings["wic0"].as_str().or(source["wic0"].as_str()),
                    "wic1": runtime_settings["wic1"].as_str().or(source["wic1"].as_str()),
                    "wic2": runtime_settings["wic2"].as_str().or(source["wic2"].as_str())
                });
                if matches!(platform, "c1700" | "c2600" | "c3600") {
                    settings["chassis"] = json!(chassis);
                }
                if !dynamips_configuration_supported(&settings) {
                    return Err(
                        "Choose a supported Dynamips platform and chassis configuration".into(),
                    );
                }
                settings
            }
            "docker" => {
                let image = source["image"].as_str().ok_or_else(|| {
                    "The selected Docker template has no image reference".to_owned()
                })?;
                json!({
                    "image": image,
                    "ram": source["memory"].as_u64().unwrap_or(0),
                    "cpus": source["cpus"].as_f64().unwrap_or(0.0),
                    "adapters": source["adapters"].as_u64().unwrap_or(1),
                    "start_command": source["start_command"].as_str().unwrap_or(""),
                    "environment": source["environment"].as_str().unwrap_or(""),
                    "extra_hosts": source["extra_hosts"].as_str().unwrap_or(""),
                    "extra_volumes": source["extra_volumes"].as_array().cloned().unwrap_or_default(),
                    "console_type": source["console_type"].as_str().unwrap_or("telnet"),
                    "console_resolution": source["console_resolution"].as_str().unwrap_or("1024x768"),
                    "console_http_port": source["console_http_port"].as_u64().unwrap_or(80),
                    "console_http_path": source["console_http_path"].as_str().unwrap_or("/"),
                    "aux_type": source["aux_type"].as_str().unwrap_or("none"),
                    "custom_adapters": source["custom_adapters"].as_array().cloned().unwrap_or_default()
                })
            }
            "vpcs" => json!({
                "startup_script": source["startup_script"].as_str().unwrap_or(""),
                "base_script_file": source["base_script_file"].as_str().unwrap_or("vpcs_base_config.txt"),
                "console_type": source["console_type"].as_str().unwrap_or("telnet"),
                "console_auto_start": source["console_auto_start"].as_bool().unwrap_or(false)
            }),
            _ => unreachable!(),
        };

        let mut required_images = Vec::new();
        let mut version_images = serde_json::Map::new();
        let image_slot = match runtime {
            "qemu" => Some("hda_disk_image"),
            "dynamips" => Some("image"),
            _ => None,
        };
        if let Some(image_slot) = image_slot {
            let filename = properties[image_slot]
                .as_str()
                .ok_or_else(|| "The selected image has no filename".to_owned())?;
            let path_key = if runtime == "qemu" {
                "hda_disk_image"
            } else {
                "image"
            };
            let image_path = source[path_key]
                .as_str()
                .ok_or_else(|| "The selected image path is unavailable".to_owned())?;
            let filesize = fs::metadata(image_path)
                .map_err(|error| format!("Could not inspect selected image: {error}"))?
                .len();
            required_images.push(json!({ "filename": filename, "filesize": filesize }));
            version_images.insert(image_slot.to_owned(), json!(filename));
        }
        let appliance_id = Uuid::new_v4();
        let file_name = format!("custom-{appliance_id}.gns3a");
        let mut definition = json!({
            "appliance_id": appliance_id.to_string(),
            "name": name,
            "description": "Custom appliance created in the appliance library.",
            "category": match runtime { "qemu" => "guest", "docker" | "vpcs" => "guest", _ => "router" },
            "registry_version": 8,
            "network_simulator": { "origin": "custom" },
            "settings": [{ "name": "default", "default": true, "template_type": runtime, "template_properties": properties }],
            "versions": [{ "name": "Default", "settings": "default", "images": version_images }],
            "images": required_images
        });
        if runtime == "vpcs" {
            definition["versions"][0]
                .as_object_mut()
                .unwrap()
                .remove("images");
        }
        let bytes = serde_json::to_vec_pretty(&definition)
            .map_err(|error| format!("Could not encode custom appliance: {error}"))?;
        self.import_appliance_file(&file_name, &bytes)
    }

    pub fn imported_appliances(&self) -> Result<Vec<ApplianceRecord>, String> {
        let entries = match fs::read_dir(&self.appliance_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(format!("Could not list imported appliances: {error}")),
        };
        let mut records = Vec::new();
        for entry in entries {
            let entry =
                entry.map_err(|error| format!("Could not read appliance entry: {error}"))?;
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("gns3a") {
                continue;
            }
            let content = fs::read(&path)
                .map_err(|error| format!("Could not read imported appliance: {error}"))?;
            let appliance: Value = serde_json::from_slice(&content)
                .map_err(|error| format!("Imported appliance JSON is invalid: {error}"))?;
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            records.push(summarize_appliance(file_name, &appliance, true)?);
        }
        records.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
        Ok(records)
    }

    pub async fn appliance_manager(
        &self,
        image_options: &[Value],
    ) -> Result<ApplianceManagerState, String> {
        let mut local_images = local_appliance_images(image_options)?;
        let mut appliances = self.imported_appliances()?;
        for appliance in &mut appliances {
            let bindings = self.read_image_bindings(&appliance.appliance_id)?;
            for version in &mut appliance.versions {
                if version.template_type.as_deref() == Some("docker")
                    && let Some(image_ref) = version.docker_image.as_deref()
                    && let Some(template) = image_options.iter().find(|template| {
                        template["template_type"] == "docker"
                            && template["image"].as_str().is_some_and(|candidate| {
                                candidate == image_ref
                                    || (!image_ref.contains(':')
                                        && candidate.starts_with(&format!("{image_ref}:")))
                            })
                    })
                {
                    version.docker_image_available = true;
                    version.docker_image_id = template["image_digest"].as_str().map(str::to_owned);
                }
                for requirement in &mut version.images {
                    if !requirement.assignable {
                        requirement.status = "runtime-slot-unsupported".to_owned();
                        continue;
                    }
                    let binding = bindings.iter().find(|binding| {
                        binding.version == version.name && binding.image_key == requirement.key
                    });
                    let preferred = binding.and_then(|binding| {
                        local_images.iter().position(|image| {
                            image.image_id == binding.image_id && image.path == binding.image_path
                        })
                    });
                    let expected = requirement
                        .md5sum
                        .as_deref()
                        .map(|value| value.to_ascii_lowercase());
                    let mut candidate_indices = Vec::new();
                    if let Some(index) = preferred {
                        candidate_indices.push(index);
                    } else {
                        candidate_indices.extend(local_images.iter().enumerate().filter_map(
                            |(index, image)| {
                                (image.template_type
                                    == version.template_type.as_deref().unwrap_or_default()
                                    && image.name.eq_ignore_ascii_case(&requirement.filename))
                                .then_some(index)
                            },
                        ));
                        if expected.is_some() {
                            let already_checked = candidate_indices.clone();
                            candidate_indices.extend(local_images.iter().enumerate().filter_map(
                                |(index, image)| {
                                    (image.template_type
                                        == version.template_type.as_deref().unwrap_or_default()
                                        && requirement
                                            .filesize
                                            .is_none_or(|size| size == image.size)
                                        && !already_checked.contains(&index))
                                    .then_some(index)
                                },
                            ));
                        }
                    }
                    let mut selected = None;
                    let mut first_candidate = None;
                    for index in candidate_indices {
                        let checksum = self
                            .image_checksum(Path::new(&local_images[index].path))
                            .await?;
                        local_images[index].size = checksum.size;
                        local_images[index].md5sum = Some(checksum.md5sum.clone());
                        local_images[index].sha1sum = Some(checksum.sha1sum.clone());
                        if first_candidate.is_none() {
                            first_candidate = Some(index);
                        }
                        let matches = requirement
                            .filesize
                            .is_none_or(|size| size == checksum.size)
                            && expected.as_deref().is_none_or(|md5| md5 == checksum.md5sum);
                        if matches {
                            selected = Some(index);
                            break;
                        }
                    }
                    let current = preferred
                        .filter(|_| binding.is_some())
                        .and_then(|index| Some((index, binding?)));
                    let chosen = current.map(|(index, _)| index).or(selected);
                    let observed = chosen.or(first_candidate);
                    if let Some(index) = observed {
                        requirement.suggested_image_id = Some(local_images[index].image_id.clone());
                        requirement.suggested_image_name = Some(local_images[index].name.clone());
                        requirement.actual_size = Some(local_images[index].size);
                        requirement.actual_md5sum = local_images[index].md5sum.clone();
                        requirement.actual_sha1sum = local_images[index].sha1sum.clone();
                    }
                    if let Some((index, binding)) = current {
                        let image = &local_images[index];
                        requirement.assigned_image_id = Some(image.image_id.clone());
                        requirement.assigned_image_name = Some(image.name.clone());
                        requirement.image_path = Some(image.path.clone());
                        requirement.allow_checksum_mismatch = binding.allow_checksum_mismatch;
                        let hash_matches = requirement
                            .md5sum
                            .as_deref()
                            .is_none_or(|md5| image.md5sum.as_deref() == Some(md5));
                        let size_matches =
                            requirement.filesize.is_none_or(|size| image.size == size);
                        requirement.status = if hash_matches && size_matches {
                            verification_status(requirement).to_owned()
                        } else if binding.allow_checksum_mismatch {
                            "override".to_owned()
                        } else {
                            "mismatch".to_owned()
                        };
                    } else if let Some(index) = selected {
                        let image = &local_images[index];
                        requirement.suggested_image_id = Some(image.image_id.clone());
                        requirement.suggested_image_name = Some(image.name.clone());
                        requirement.actual_size = Some(image.size);
                        requirement.actual_md5sum = image.md5sum.clone();
                        requirement.actual_sha1sum = image.sha1sum.clone();
                        let status = verification_status(requirement);
                        let binding = PersistedImageBinding {
                            version: version.name.clone(),
                            image_key: requirement.key.clone(),
                            image_id: image.image_id.clone(),
                            image_path: image.path.clone(),
                            allow_checksum_mismatch: false,
                        };
                        self.write_image_binding(&appliance.appliance_id, binding)
                            .await?;
                        requirement.assigned_image_id = Some(image.image_id.clone());
                        requirement.assigned_image_name = Some(image.name.clone());
                        requirement.image_path = Some(image.path.clone());
                        requirement.status = status.to_owned();
                    } else if first_candidate.is_some() {
                        requirement.status = "mismatch".to_owned();
                    } else if binding.is_some() {
                        requirement.status = "assigned-image-missing".to_owned();
                        requirement.assigned_image_name = binding
                            .map(|binding| Path::new(&binding.image_path).file_name())
                            .flatten()
                            .and_then(|name| name.to_str())
                            .map(str::to_owned);
                        requirement.image_path = binding.map(|binding| binding.image_path.clone());
                    } else {
                        requirement.status = "missing".to_owned();
                    }
                }
            }
        }
        Ok(ApplianceManagerState {
            appliances,
            local_images,
            custom_image_options: image_options.to_vec(),
        })
    }

    pub async fn set_image_binding(
        &self,
        file_name: &str,
        version_name: &str,
        image_key: &str,
        image_id: &str,
        allow_checksum_mismatch: bool,
        base_templates: &[Value],
    ) -> Result<ApplianceRecord, String> {
        let file_name = validate_appliance_file_name(file_name)?;
        let mut record = self
            .imported_appliances()?
            .into_iter()
            .find(|record| record.file_name == file_name)
            .ok_or_else(|| format!("Appliance '{file_name}' is not imported"))?;
        let version = record
            .versions
            .iter_mut()
            .find(|version| version.name == version_name)
            .ok_or_else(|| format!("Appliance configuration '{version_name}' was not found"))?;
        let requirement = version
            .images
            .iter_mut()
            .find(|image| image.key == image_key)
            .ok_or_else(|| {
                format!("Image role '{image_key}' was not found in configuration '{version_name}'")
            })?;
        if !requirement.assignable {
            return Err(format!(
                "Image role '{image_key}' is listed in the appliance, but the current runtime cannot attach this image slot"
            ));
        }
        let local_images = local_appliance_images(base_templates)?;
        let image = local_images
            .iter()
            .find(|image| image.image_id == image_id)
            .ok_or_else(|| {
                "The selected image is no longer in the local runtime library".to_owned()
            })?;
        let expected_runtime = version.template_type.as_deref().unwrap_or_default();
        if image.template_type != expected_runtime {
            return Err(format!(
                "This configuration requires a {expected_runtime} image; '{}' is a {} image",
                image.name, image.template_type
            ));
        }
        let checksum = self.image_checksum(Path::new(&image.path)).await?;
        let hash_matches = requirement
            .md5sum
            .as_deref()
            .is_none_or(|expected| expected.eq_ignore_ascii_case(&checksum.md5sum));
        let size_matches = requirement
            .filesize
            .is_none_or(|expected| expected == checksum.size);
        if (!hash_matches || !size_matches) && !allow_checksum_mismatch {
            return Err(format!(
                "Selected image does not match the appliance definition. Expected MD5: {}; actual MD5: {}. Expected size: {}; actual size: {} bytes. Enable the explicit checksum override to use this image anyway.",
                requirement.md5sum.as_deref().unwrap_or("not provided"),
                checksum.md5sum,
                requirement
                    .filesize
                    .map(|size| size.to_string())
                    .unwrap_or_else(|| "not provided".to_owned()),
                checksum.size
            ));
        }
        let binding = PersistedImageBinding {
            version: version_name.to_owned(),
            image_key: image_key.to_owned(),
            image_id: image.image_id.clone(),
            image_path: image.path.clone(),
            allow_checksum_mismatch,
        };
        self.write_image_binding(&record.appliance_id, binding)
            .await?;
        requirement.assigned_image_id = Some(image.image_id.clone());
        requirement.assigned_image_name = Some(image.name.clone());
        requirement.image_path = Some(image.path.clone());
        requirement.actual_size = Some(checksum.size);
        requirement.actual_md5sum = Some(checksum.md5sum);
        requirement.actual_sha1sum = Some(checksum.sha1sum);
        requirement.allow_checksum_mismatch = allow_checksum_mismatch;
        requirement.status = if hash_matches && size_matches {
            verification_status(requirement).to_owned()
        } else {
            "override".to_owned()
        };
        Ok(record)
    }

    pub async fn clear_image_binding(
        &self,
        file_name: &str,
        version_name: &str,
        image_key: &str,
    ) -> Result<ApplianceRecord, String> {
        let file_name = validate_appliance_file_name(file_name)?;
        let record = self
            .imported_appliances()?
            .into_iter()
            .find(|record| record.file_name == file_name)
            .ok_or_else(|| format!("Appliance '{file_name}' is not imported"))?;
        let _guard = self.binding_lock.lock().await;
        let mut bindings = self.read_image_bindings(&record.appliance_id)?;
        let previous_length = bindings.len();
        bindings
            .retain(|binding| binding.version != version_name || binding.image_key != image_key);
        if bindings.len() != previous_length {
            self.save_image_bindings(&record.appliance_id, &bindings)?;
        }
        Ok(record)
    }

    async fn image_checksum(&self, path: &Path) -> Result<CachedImageHash, String> {
        let canonical = path
            .canonicalize()
            .map_err(|error| format!("Could not resolve image '{}': {error}", path.display()))?;
        let metadata = fs::metadata(&canonical)
            .map_err(|error| format!("Could not read image '{}': {error}", canonical.display()))?;
        let size = metadata.len();
        let modified_ns = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        if let Some(cached) = self.image_hashes.lock().await.get(&canonical).cloned()
            && cached.size == size
            && cached.modified_ns == modified_ns
        {
            return Ok(cached);
        }
        let hash_path = canonical.clone();
        let (md5sum, sha1sum) = tokio::task::spawn_blocking(move || {
            let mut file = fs::File::open(&hash_path).map_err(|error| error.to_string())?;
            let mut md5 = Md5::new();
            let mut sha1 = Sha1::new();
            let mut buffer = vec![0_u8; 1024 * 1024];
            loop {
                let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
                if count == 0 {
                    break;
                }
                md5.update(&buffer[..count]);
                sha1.update(&buffer[..count]);
            }
            Ok::<_, String>((
                format!("{:x}", md5.finalize()),
                format!("{:x}", sha1.finalize()),
            ))
        })
        .await
        .map_err(|error| format!("Image checksum worker failed: {error}"))??;
        let after = fs::metadata(&canonical).map_err(|error| {
            format!("Could not recheck image '{}': {error}", canonical.display())
        })?;
        let after_modified_ns = after
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        if after.len() != size || after_modified_ns != modified_ns {
            self.image_hashes.lock().await.remove(&canonical);
            return Err(format!(
                "Image '{}' changed while its checksums were being calculated; refresh and retry",
                canonical.display()
            ));
        }
        let checksum = CachedImageHash {
            size,
            modified_ns,
            md5sum,
            sha1sum,
        };
        self.image_hashes
            .lock()
            .await
            .insert(canonical, checksum.clone());
        Ok(checksum)
    }

    fn bindings_path(&self, appliance_id: &str) -> Result<PathBuf, String> {
        let id = Uuid::parse_str(appliance_id).map_err(|_| "Appliance ID is invalid".to_owned())?;
        Ok(self.appliance_dir.join(format!("{id}.image-bindings.json")))
    }

    fn read_image_bindings(
        &self,
        appliance_id: &str,
    ) -> Result<Vec<PersistedImageBinding>, String> {
        let path = self.bindings_path(appliance_id)?;
        match fs::read(&path) {
            Ok(content) => serde_json::from_slice(&content)
                .map_err(|error| format!("Appliance image bindings are invalid: {error}")),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(format!("Could not read appliance image bindings: {error}")),
        }
    }

    async fn write_image_binding(
        &self,
        appliance_id: &str,
        binding: PersistedImageBinding,
    ) -> Result<(), String> {
        let _guard = self.binding_lock.lock().await;
        let mut bindings = self.read_image_bindings(appliance_id)?;
        if let Some(existing) = bindings.iter_mut().find(|existing| {
            existing.version == binding.version && existing.image_key == binding.image_key
        }) {
            *existing = binding;
        } else {
            bindings.push(binding);
        }
        self.save_image_bindings(appliance_id, &bindings)
    }

    fn save_image_bindings(
        &self,
        appliance_id: &str,
        bindings: &[PersistedImageBinding],
    ) -> Result<(), String> {
        fs::create_dir_all(&self.appliance_dir).map_err(|error| {
            format!("Could not create appliance configuration directory: {error}")
        })?;
        let path = self.bindings_path(appliance_id)?;
        if bindings.is_empty() {
            match fs::remove_file(&path) {
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
                Err(error) => {
                    return Err(format!(
                        "Could not remove appliance image bindings: {error}"
                    ));
                }
            }
        }
        let temporary_path = path.with_extension(format!("json.{}.tmp", Uuid::new_v4()));
        let bytes = serde_json::to_vec_pretty(bindings)
            .map_err(|error| format!("Could not encode appliance image bindings: {error}"))?;
        fs::write(&temporary_path, bytes)
            .map_err(|error| format!("Could not write appliance image bindings: {error}"))?;
        fs::rename(&temporary_path, &path).map_err(|error| {
            let _ = fs::remove_file(&temporary_path);
            format!("Could not install appliance image bindings: {error}")
        })
    }

    pub fn templates(&self, base_templates: Vec<Value>) -> Vec<Value> {
        let records = match self.imported_appliances() {
            Ok(records) => records,
            Err(error) => {
                tracing::warn!(%error, "could not load imported GNS3 appliances");
                return Vec::new();
            }
        };
        let mut result = Vec::new();
        for record in records {
            let path = self.appliance_dir.join(&record.file_name);
            let Ok(content) = fs::read(path) else {
                continue;
            };
            let Ok(appliance) = serde_json::from_slice::<Value>(&content) else {
                continue;
            };
            let bindings = match self.read_image_bindings(&record.appliance_id) {
                Ok(bindings) => bindings,
                Err(error) => {
                    tracing::warn!(%error, appliance = %record.file_name, "could not load appliance image bindings");
                    Vec::new()
                }
            };
            result.extend(appliance_runtime_templates(
                &record,
                &appliance,
                &base_templates,
                &bindings,
            ));
        }
        result.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
        result
    }

    async fn ishare_index(&self) -> Result<Value, String> {
        {
            let cache = self.cache.lock().await;
            if let Some(cached) = &cache.ishare_index
                && cached.loaded_at.elapsed() < CACHE_TTL
            {
                return Ok(cached.value.clone());
            }
        }
        let release: GithubRelease = self
            .client
            .get(ISHARE_RELEASE_API)
            .send()
            .await
            .map_err(|error| format!("Could not reach the iShare2 catalog: {error}"))?
            .error_for_status()
            .map_err(|error| format!("iShare2 catalog request failed: {error}"))?
            .json()
            .await
            .map_err(|error| format!("iShare2 release metadata was invalid: {error}"))?;
        let index_url = release
            .assets
            .into_iter()
            .find(|asset| asset.name == "labhub.json")
            .map(|asset| asset.browser_download_url)
            .ok_or_else(|| "The latest iShare2 release has no labhub.json catalog".to_owned())?;
        let bytes = self.fetch_limited(&index_url, MAX_INDEX_BYTES).await?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|error| format!("The iShare2 image index is invalid JSON: {error}"))?;
        let mut cache = self.cache.lock().await;
        cache.ishare_index = Some(Cached {
            loaded_at: Instant::now(),
            value: value.clone(),
        });
        Ok(value)
    }

    async fn gns3_appliance_files(&self) -> Result<Vec<GithubContent>, String> {
        {
            let cache = self.cache.lock().await;
            if let Some(cached) = &cache.gns3_files
                && cached.loaded_at.elapsed() < CACHE_TTL
            {
                return Ok(cached.value.clone());
            }
        }
        let entries: Vec<GithubContent> = self
            .client
            .get(GNS3_APPLIANCES_API)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|error| format!("Could not reach the GNS3 appliance registry: {error}"))?
            .error_for_status()
            .map_err(|error| format!("GNS3 appliance registry request failed: {error}"))?
            .json()
            .await
            .map_err(|error| format!("GNS3 registry listing was invalid: {error}"))?;
        let mut cache = self.cache.lock().await;
        cache.gns3_files = Some(Cached {
            loaded_at: Instant::now(),
            value: entries.clone(),
        });
        Ok(entries)
    }

    async fn fetch_appliance(&self, file_name: &str) -> Result<Value, String> {
        let url = format!("{GNS3_RAW_APPLIANCES}/{file_name}");
        let bytes = self.fetch_limited(&url, MAX_APPLIANCE_BYTES).await?;
        let appliance: Value = serde_json::from_slice(&bytes)
            .map_err(|error| format!("GNS3 appliance file is invalid JSON: {error}"))?;
        summarize_appliance(file_name, &appliance, false)?;
        Ok(appliance)
    }

    async fn fetch_limited(&self, url: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|error| format!("Marketplace download failed: {error}"))?
            .error_for_status()
            .map_err(|error| format!("Marketplace request failed: {error}"))?;
        if response
            .content_length()
            .is_some_and(|length| length > max_bytes as u64)
        {
            return Err("Marketplace response exceeded its size limit".to_owned());
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| format!("Could not read marketplace response: {error}"))?;
        if bytes.len() > max_bytes {
            return Err("Marketplace response exceeded its size limit".to_owned());
        }
        Ok(bytes.to_vec())
    }

    fn imported_appliance_file_names(&self) -> HashSet<String> {
        self.imported_appliances()
            .unwrap_or_default()
            .into_iter()
            .map(|record| record.file_name)
            .collect()
    }
}

fn ishare_row_matches_query(row: &Value, needle: &str) -> bool {
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

fn validate_query(query: &str) -> Result<&str, String> {
    if query.len() > 120 {
        return Err("Catalog searches are limited to 120 characters".to_owned());
    }
    Ok(query.trim())
}

fn safe_catalog_component(value: &str) -> Result<String, String> {
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

fn safe_download_filename(filename: &str) -> bool {
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

fn safe_ishare_path(path: &str) -> bool {
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

fn valid_ishare_host(host: &str) -> bool {
    host == "labhub.eu.org" || host.ends_with(".labhub.eu.org")
}

fn valid_ishare_prefix(prefix: &str) -> bool {
    prefix.starts_with('/')
        && !prefix.contains("..")
        && prefix.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b':' | b'-' | b'_' | b'.')
        })
}

fn is_supported_qemu_extension(extension: &str) -> bool {
    matches!(extension, "qcow2" | "qcow" | "raw" | "img" | "vmdk" | "vdi")
}

fn validate_appliance_file_name(file_name: &str) -> Result<&str, String> {
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

fn title_from_slug(slug: &str) -> String {
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

fn image_file_name(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
}

fn summarize_appliance(
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

fn appliance_versions(
    appliance: &Value,
    required_files: &[ApplianceFile],
) -> Vec<ApplianceVersion> {
    let versions = appliance
        .get("versions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let versions = if versions.is_empty() {
        vec![Value::Null]
    } else {
        versions
    };
    versions
        .iter()
        .map(|version| {
            let version_name = version
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Default")
                .to_owned();
            let settings_name = version
                .get("settings")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let runtime = version
                .get("settings")
                .and_then(Value::as_str)
                .and_then(|settings_name| {
                    appliance
                        .get("settings")
                        .and_then(Value::as_array)
                        .and_then(|settings| {
                            settings.iter().find(|setting| {
                                setting.get("name").and_then(Value::as_str) == Some(settings_name)
                            })
                        })
                        .and_then(|setting| setting.get("template_type"))
                        .and_then(Value::as_str)
                })
                .or_else(|| appliance_runtime_type(appliance));
            let properties = runtime.and_then(|runtime| {
                appliance_settings(appliance, runtime, settings_name.as_deref())
            });
            let mut images = Vec::new();
            if let Some(configured_images) = version.get("images").and_then(Value::as_object) {
                for (key, configured) in configured_images {
                    let filename = configured
                        .as_str()
                        .or_else(|| configured.get("filename").and_then(Value::as_str));
                    let Some(filename) = filename else {
                        continue;
                    };
                    let requirement = required_files
                        .iter()
                        .find(|image| image.filename.eq_ignore_ascii_case(filename));
                    images.push(ApplianceImageRequirement {
                        key: key.clone(),
                        filename: filename.to_owned(),
                        assignable: matches!(
                            (runtime, key.as_str()),
                            (Some("qemu"), "hda_disk_image") | (Some("dynamips"), "image")
                        ),
                        filesize: requirement.and_then(|image| image.size),
                        md5sum: requirement.and_then(|image| image.md5sum.clone()),
                        assigned_image_id: None,
                        assigned_image_name: None,
                        suggested_image_id: None,
                        suggested_image_name: None,
                        image_path: None,
                        actual_size: None,
                        actual_md5sum: None,
                        actual_sha1sum: None,
                        status: "missing".to_owned(),
                        allow_checksum_mismatch: false,
                    });
                }
            } else if let (Some(runtime), Some(properties)) = (runtime, properties) {
                let role = match runtime {
                    "qemu" => "hda_disk_image",
                    "dynamips" => "image",
                    _ => "",
                };
                if !role.is_empty()
                    && let Some(filename) = properties.get(role).and_then(Value::as_str)
                {
                    let requirement = required_files
                        .iter()
                        .find(|image| image.filename.eq_ignore_ascii_case(filename));
                    images.push(ApplianceImageRequirement {
                        key: role.to_owned(),
                        filename: filename.to_owned(),
                        assignable: true,
                        filesize: requirement.and_then(|image| image.size),
                        md5sum: requirement.and_then(|image| image.md5sum.clone()),
                        assigned_image_id: None,
                        assigned_image_name: None,
                        suggested_image_id: None,
                        suggested_image_name: None,
                        image_path: None,
                        actual_size: None,
                        actual_md5sum: None,
                        actual_sha1sum: None,
                        status: "missing".to_owned(),
                        allow_checksum_mismatch: false,
                    });
                }
            }
            let platform = properties
                .and_then(|properties| properties.get("platform"))
                .and_then(Value::as_str)
                .or_else(|| {
                    appliance
                        .get("dynamips")
                        .and_then(|value| value["platform"].as_str())
                });
            let runtime_supported = match runtime {
                Some("qemu") => {
                    version["images"]["hda_disk_image"]
                        .as_str()
                        .or_else(|| {
                            properties.and_then(|properties| properties["hda_disk_image"].as_str())
                        })
                        .is_some_and(|value| !value.is_empty())
                        && images.iter().all(|image| image.assignable)
                }
                Some("docker") => properties
                    .and_then(|properties| properties.get("image"))
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.is_empty()),
                Some("dynamips") => properties
                    .map(dynamips_configuration_supported)
                    .unwrap_or_else(|| platform == Some("c7200")),
                Some("vpcs") => true,
                _ => false,
            };
            let docker_image = (runtime == Some("docker"))
                .then(|| properties.and_then(|properties| properties["image"].as_str()))
                .flatten()
                .map(str::to_owned);
            ApplianceVersion {
                name: version_name,
                settings: settings_name,
                template_type: runtime.map(str::to_owned),
                runtime_supported,
                docker_image,
                docker_image_id: None,
                docker_image_available: false,
                images,
            }
        })
        .collect()
}

fn local_appliance_images(base_templates: &[Value]) -> Result<Vec<LocalApplianceImage>, String> {
    let mut images = Vec::new();
    for template in base_templates {
        let Some(template_type) = template["template_type"].as_str() else {
            continue;
        };
        let path_field = match template_type {
            "qemu" => "hda_disk_image",
            "dynamips" => "image",
            _ => continue,
        };
        let Some(path) = template[path_field].as_str() else {
            continue;
        };
        let path = Path::new(path).canonicalize().map_err(|error| {
            format!("Could not resolve local {template_type} image '{path}': {error}")
        })?;
        let metadata = fs::metadata(&path).map_err(|error| {
            format!(
                "Could not inspect local image '{}': {error}",
                path.display()
            )
        })?;
        if !metadata.is_file() {
            continue;
        }
        let image_id = template["template_id"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("Local image")
            .to_owned();
        images.push(LocalApplianceImage {
            image_id,
            name,
            template_type: template_type.to_owned(),
            path: path.display().to_string(),
            size: metadata.len(),
            md5sum: None,
            sha1sum: None,
        });
    }
    images.sort_by(|left, right| {
        left.template_type
            .cmp(&right.template_type)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(images)
}

fn verification_status(requirement: &ApplianceImageRequirement) -> &'static str {
    if requirement.md5sum.is_some() {
        "verified"
    } else if requirement.filesize.is_some() {
        "size-verified"
    } else {
        "available-unverified"
    }
}

fn appliance_runtime_type(appliance: &Value) -> Option<&str> {
    if let Some(settings) = appliance.get("settings").and_then(Value::as_array) {
        let preferred = settings
            .iter()
            .find(|settings| settings.get("default").and_then(Value::as_bool) == Some(true))
            .or_else(|| settings.first());
        if let Some(runtime) = preferred?.get("template_type").and_then(Value::as_str) {
            return Some(runtime);
        }
    }
    ["qemu", "dynamips", "docker", "iou", "vpcs"]
        .into_iter()
        .find(|key| appliance.get(*key).is_some())
}

fn appliance_runtime_templates(
    record: &ApplianceRecord,
    appliance: &Value,
    base_templates: &[Value],
    bindings: &[PersistedImageBinding],
) -> Vec<Value> {
    let versions = appliance
        .get("versions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| vec![Value::Null]);
    let id = Uuid::parse_str(&record.appliance_id).ok();
    let mut templates = Vec::new();
    for version in versions {
        let version_name = version
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let version_settings = version.get("settings").and_then(Value::as_str);
        let version_summary = record.versions.iter().find(|variant| {
            variant.name
                == if version_name.is_empty() {
                    "Default"
                } else {
                    version_name
                }
        });
        let runtime = version_summary
            .and_then(|version| version.template_type.as_deref())
            .or_else(|| {
                version_settings
                    .and_then(|settings_name| {
                        appliance
                            .get("settings")
                            .and_then(Value::as_array)
                            .and_then(|settings| {
                                settings.iter().find(|setting| {
                                    setting.get("name").and_then(Value::as_str)
                                        == Some(settings_name)
                                })
                            })
                            .and_then(|setting| setting.get("template_type"))
                            .and_then(Value::as_str)
                    })
                    .or_else(|| appliance_runtime_type(appliance))
            });
        let Some(runtime) =
            runtime.filter(|runtime| matches!(*runtime, "qemu" | "dynamips" | "docker" | "vpcs"))
        else {
            continue;
        };
        // Do not create a runnable QEMU palette entry when the appliance
        // requires image slots this runtime cannot attach yet (HDB/HDC/HDD,
        // BIOS, or other auxiliary files). Showing only HDA would look ready
        // while silently discarding part of the registry configuration.
        if runtime == "qemu"
            && version_summary
                .is_some_and(|summary| summary.images.iter().any(|image| !image.assignable))
        {
            continue;
        }
        let properties = appliance_properties(appliance, runtime);
        let settings_properties =
            appliance_settings(appliance, runtime, version_settings).unwrap_or(properties);
        if runtime == "dynamips" && !dynamips_configuration_supported(settings_properties) {
            continue;
        }
        let image_name = match runtime {
            "qemu" => version["images"]["hda_disk_image"]
                .as_str()
                .or_else(|| properties["hda_disk_image"].as_str()),
            "dynamips" => version["images"]["image"]
                .as_str()
                .or_else(|| properties["image"].as_str()),
            "docker" => properties["image"].as_str(),
            "vpcs" => Some(""),
            _ => None,
        };
        let Some(image_name) = image_name else {
            continue;
        };
        let image_key = if runtime == "qemu" {
            "hda_disk_image"
        } else if runtime == "dynamips" {
            "image"
        } else {
            ""
        };
        let binding = bindings.iter().find(|binding| {
            binding.version
                == if version_name.is_empty() {
                    "Default"
                } else {
                    version_name
                }
                && binding.image_key == image_key
        });
        let local_template = base_templates.iter().find(|template| {
            template["template_type"].as_str() == Some(runtime)
                && if runtime == "vpcs" {
                    true
                } else if let Some(binding) = binding {
                    let path = if runtime == "qemu" {
                        template["hda_disk_image"].as_str()
                    } else {
                        template["image"].as_str()
                    };
                    path.is_some_and(|path| path == binding.image_path)
                } else {
                    let has_expected_hash = record
                        .versions
                        .iter()
                        .find(|variant| {
                            variant.name
                                == if version_name.is_empty() {
                                    "Default"
                                } else {
                                    version_name
                                }
                        })
                        .and_then(|variant| {
                            variant.images.iter().find(|image| image.key == image_key)
                        })
                        .and_then(|image| image.md5sum.as_ref())
                        .is_some();
                    (!has_expected_hash && template_image_matches(template, runtime, image_name))
                        || (runtime == "docker"
                            && template_image_matches(template, runtime, image_name))
                }
        });
        let Some(base_template) = local_template else {
            continue;
        };
        let mut template = base_template.clone();
        let display_name = if version_name.is_empty() {
            record.name.clone()
        } else {
            format!("{} · {}", record.name, version_name)
        };
        template["name"] = json!(display_name);
        if let Some(id) = id {
            template["template_id"] = json!(Uuid::new_v5(
                &id,
                format!("{runtime}:{}:{version_name}", base_template["template_id"]).as_bytes()
            ));
        }
        template["appliance_id"] = json!(record.appliance_id);
        template["appliance_file"] = json!(record.file_name);
        template["appliance_description"] = json!(record.description);
        template["appliance_version"] = json!(version_name);
        if let Some(adapters) = settings_properties["adapters"].as_u64() {
            template["adapters"] = json!(adapters);
        }
        if let Some(memory) = settings_properties["ram"].as_u64() {
            template["memory"] = json!(memory);
        }
        if let Some(cpus) = settings_properties["cpus"].as_u64() {
            template["cpus"] = json!(cpus);
        }
        if runtime == "qemu" {
            for key in [
                "platform",
                "adapter_type",
                "hda_disk_interface",
                "boot_priority",
                "linked_clone",
                "maxcpus",
                "hdb_disk_image",
                "hdb_disk_interface",
                "hdc_disk_image",
                "hdc_disk_interface",
                "hdd_disk_image",
                "hdd_disk_interface",
                "cdrom_image",
                "bios_image",
                "kernel_image",
                "initrd",
                "kernel_command_line",
                "console_type",
                "aux_type",
                "on_close",
                "process_priority",
                "options",
                "tpm",
                "uefi",
                "replicate_network_connection_state",
                "create_config_disk",
                "cpu_throttling",
            ] {
                if let Some(value) = settings_properties.get(key) {
                    template[key] = value.clone();
                }
            }
            if settings_properties["ram"].is_u64() {
                template["memory"] = settings_properties["ram"].clone();
            }
        } else if runtime == "docker" {
            for key in [
                "environment",
                "extra_hosts",
                "extra_volumes",
                "start_command",
                "console_type",
                "console_resolution",
                "console_http_port",
                "console_http_path",
                "aux_type",
                "custom_adapters",
            ] {
                if let Some(value) = settings_properties.get(key) {
                    template[key] = value.clone();
                }
            }
            if settings_properties["ram"].is_u64() {
                template["memory"] = settings_properties["ram"].clone();
            }
            if let Some(image) = settings_properties["image"].as_str() {
                template["image"] = json!(image);
            }
        }
        if runtime == "dynamips" {
            if let Some(platform) = settings_properties["platform"].as_str() {
                template["platform"] = json!(platform);
                template["nvram"] = json!(
                    settings_properties["nvram"]
                        .as_u64()
                        .unwrap_or(if platform == "c3600" { 256 } else { 128 })
                );
                for key in [
                    "chassis",
                    "slot0",
                    "slot1",
                    "slot2",
                    "slot3",
                    "slot4",
                    "slot5",
                    "slot6",
                    "wic0",
                    "wic1",
                    "wic2",
                    "mmap",
                    "sparsemem",
                    "clock_divisor",
                    "idlemax",
                    "idlesleep",
                    "exec_area",
                    "disk0",
                    "disk1",
                    "auto_delete_disks",
                    "private_config",
                    "mac_addr",
                    "system_id",
                    "npe",
                    "midplane",
                    "iomem",
                    "ghost_ios",
                    "console_type",
                    "aux_type",
                ] {
                    if let Some(value) = settings_properties.get(key) {
                        template[key] = value.clone();
                    }
                }
                if settings_properties.get("slot0").is_none() {
                    let chassis = settings_properties["chassis"]
                        .as_str()
                        .or_else(|| template["chassis"].as_str())
                        .unwrap_or_default();
                    template["slot0"] = json!(default_dynamips_slot0(platform, chassis));
                }
            }
            let idlepc = settings_properties["idlepc"]
                .as_str()
                .or_else(|| version["idlepc"].as_str());
            if let Some(idlepc) = idlepc {
                template["idlepc"] = json!(idlepc);
            }
        } else if runtime == "vpcs" {
            for key in [
                "startup_script",
                "base_script_file",
                "console_type",
                "console_auto_start",
            ] {
                if let Some(value) = settings_properties.get(key) {
                    template[key] = value.clone();
                }
            }
        }
        templates.push(template);
    }
    templates
}

fn dynamips_configuration_supported(properties: &Value) -> bool {
    let platform = properties["platform"].as_str().unwrap_or_default();
    let chassis = properties["chassis"].as_str().unwrap_or_default();
    let slot0 = properties["slot0"].as_str().unwrap_or_default();
    let chassis_valid = match platform {
        "c1700" => matches!(chassis, "" | "1720" | "1721" | "1750" | "1751" | "1760"),
        "c2600" => matches!(
            chassis,
            "" | "2610"
                | "2611"
                | "2620"
                | "2610XM"
                | "2620XM"
                | "2650XM"
                | "2621"
                | "2611XM"
                | "2621XM"
                | "2651XM"
        ),
        "c3600" => matches!(chassis, "3620" | "3640" | "3660"),
        "c2691" | "c3725" | "c3745" | "c7200" => chassis.is_empty(),
        _ => false,
    };
    matches!(
        platform,
        "c1700" | "c2600" | "c2691" | "c3600" | "c3725" | "c3745" | "c7200"
    ) && chassis_valid
        && dynamips_module_supported(platform, chassis, 0, slot0)
        && (1..=6).all(|slot| {
            properties
                .get(format!("slot{slot}"))
                .and_then(Value::as_str)
                .is_none_or(|module| dynamips_module_supported(platform, chassis, slot, module))
        })
}

fn default_dynamips_slot0(platform: &str, chassis: &str) -> &'static str {
    match platform {
        "c7200" => "C7200-IO-FE",
        "c1700" => "C1700-MB-1FE",
        "c2600" => match chassis {
            "2611" => "C2600-MB-2E",
            "2620" | "2610XM" | "2620XM" | "2650XM" => "C2600-MB-1FE",
            "2621" | "2611XM" | "2621XM" | "2651XM" => "C2600-MB-2FE",
            _ => "C2600-MB-1E",
        },
        "c2691" | "c3725" | "c3745" => "GT96100-FE",
        "c3600" if chassis == "3660" => "Leopard-2FE",
        "c3600" => "NM-1FE-TX",
        _ => "",
    }
}

fn dynamips_module_supported(platform: &str, chassis: &str, slot: usize, module: &str) -> bool {
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
        "C1700-MB-WIC1" => platform == "c1700" && slot == 1 && matches!(chassis, "1751" | "1760"),
        "GT96100-FE" => matches!(platform, "c2691" | "c3725" | "c3745") && slot == 0,
        "Leopard-2FE" => platform == "c3600" && chassis == "3660" && slot == 0,
        "NM-16ESW" | "NM-1E" | "NM-1FE-TX" | "NM-4E" | "NM-4T" => {
            matches!(platform, "c2600" | "c2691" | "c3600" | "c3725" | "c3745")
                && (slot > 0 || (platform == "c3600" && chassis != "3660"))
        }
        _ => false,
    }
}

fn appliance_settings<'a>(
    appliance: &'a Value,
    runtime: &str,
    version_settings: Option<&str>,
) -> Option<&'a Value> {
    appliance
        .get("settings")
        .and_then(Value::as_array)
        .and_then(|settings| {
            settings
                .iter()
                .find(|setting| {
                    setting.get("name").and_then(Value::as_str) == version_settings
                        && setting.get("template_type").and_then(Value::as_str) == Some(runtime)
                })
                .or_else(|| {
                    settings.iter().find(|setting| {
                        setting.get("default").and_then(Value::as_bool) == Some(true)
                            && setting.get("template_type").and_then(Value::as_str) == Some(runtime)
                    })
                })
                .or_else(|| {
                    settings.iter().find(|setting| {
                        setting.get("template_type").and_then(Value::as_str) == Some(runtime)
                    })
                })
        })
        .and_then(|setting| setting.get("template_properties"))
        .or_else(|| appliance.get(runtime))
}

fn appliance_properties<'a>(appliance: &'a Value, runtime: &str) -> &'a Value {
    static NULL_VALUE: Value = Value::Null;
    appliance_settings(appliance, runtime, None).unwrap_or(&NULL_VALUE)
}

fn template_image_matches(template: &Value, runtime: &str, image_name: &str) -> bool {
    let candidate = if runtime == "qemu" {
        template["hda_disk_image"].as_str()
    } else {
        template["image"].as_str()
    };
    let Some(candidate) = candidate else {
        return false;
    };
    if runtime == "docker" {
        candidate == image_name
            || (!image_name.contains(':') && candidate.starts_with(&format!("{image_name}:")))
    } else {
        let candidate = Path::new(candidate)
            .file_name()
            .and_then(|name| name.to_str());
        let image_name = Path::new(image_name)
            .file_name()
            .and_then(|name| name.to_str());
        match (candidate, image_name) {
            (Some(candidate), Some(image_name)) => {
                candidate.eq_ignore_ascii_case(image_name)
                    || candidate
                        .to_ascii_lowercase()
                        .ends_with(&format!("-{}", image_name.to_ascii_lowercase()))
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MarketplaceCatalog, appliance_runtime_templates, is_supported_qemu_extension,
        ishare_row_matches_query, safe_catalog_component, safe_download_filename, safe_ishare_path,
        summarize_appliance, valid_ishare_host, validate_appliance_file_name,
    };
    use serde_json::json;
    use sha1::Digest;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    #[test]
    fn ishare_search_matches_image_manifest_filenames_as_well_as_titles() {
        let row = json!({
            "name": "Alpine virt",
            "files": [{"filename": "alpine-virt-3.19.1-x86_64.qcow2"}]
        });
        assert!(ishare_row_matches_query(
            &row,
            "alpine-virt-3.19.1-x86_64.qcow2"
        ));
        assert!(ishare_row_matches_query(&row, "alpine"));
        assert!(!ishare_row_matches_query(&row, "c7200"));
    }

    #[test]
    fn validates_registry_appliance_json_and_reports_files() {
        let appliance = json!({
            "appliance_id": "fc520ae2-a4e5-48c3-9a13-516bb2e94668",
            "name": "Alpine Linux",
            "category": "guest",
            "registry_version": 4,
            "docker": {"image": "alpine"},
            "images": [{"filename": "rootfs.qcow2", "filesize": 42, "md5sum": "abc"}]
        });
        let record = summarize_appliance("alpine.gns3a", &appliance, true).unwrap();
        assert_eq!(record.template_type.as_deref(), Some("docker"));
        assert_eq!(record.required_files[0].filename, "rootfs.qcow2");
        assert!(record.imported);
        assert!(summarize_appliance("broken.gns3a", &json!({}), false).is_err());
    }

    #[test]
    fn runtime_inventory_is_not_a_project_template_without_an_appliance_definition() {
        use tempfile::tempdir;

        let directory = tempdir().unwrap();
        let catalog = MarketplaceCatalog::with_directory(directory.path().join("appliances"));
        let runtime_inventory = vec![json!({
            "template_id": "alpine-image",
            "name": "alpine:latest",
            "template_type": "docker",
            "image": "alpine:latest"
        })];

        assert!(catalog.templates(runtime_inventory).is_empty());
    }

    #[tokio::test]
    async fn custom_appliance_is_persisted_and_becomes_the_project_template() {
        use std::fs;
        use tempfile::tempdir;

        let directory = tempdir().unwrap();
        let appliance_dir = directory.path().join("appliances");
        let image_path = directory.path().join("images/mikrotik-chr.qcow2");
        fs::create_dir_all(image_path.parent().unwrap()).unwrap();
        fs::write(&image_path, b"CHR disk").unwrap();
        let image_options = vec![json!({
            "template_id": "qemu-mikrotik",
            "name": "mikrotik-chr.qcow2",
            "template_type": "qemu",
            "hda_disk_image": image_path,
            "memory": 256,
            "cpus": 1,
            "adapters": 2,
            "adapter_type": "virtio-net-pci",
            "hda_disk_interface": "virtio",
            "boot_priority": "c"
        })];
        let catalog = MarketplaceCatalog::with_directory(&appliance_dir);

        assert!(catalog.templates(image_options.clone()).is_empty());
        let created = catalog
            .create_custom_appliance(
                "MikroTik CHR lab",
                "qemu-mikrotik",
                &json!({}),
                &image_options,
            )
            .unwrap();

        assert_eq!(created.origin, "custom");
        assert!(appliance_dir.join(&created.file_name).is_file());
        let manager = catalog.appliance_manager(&image_options).await.unwrap();
        assert_eq!(manager.appliances.len(), 1);
        assert_eq!(manager.appliances[0].origin, "custom");
        assert_eq!(manager.appliances[0].name, "MikroTik CHR lab");

        let templates = catalog.templates(image_options);
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0]["appliance_id"], created.appliance_id);
        assert_eq!(templates[0]["appliance_version"], "Default");
        assert_eq!(
            templates[0]["hda_disk_image"],
            image_path.display().to_string()
        );
        assert_eq!(templates[0]["name"], "MikroTik CHR lab · Default");
    }

    #[tokio::test]
    async fn imported_appliance_versions_bind_images_with_checksums_and_explicit_override() {
        use std::fs;
        use tempfile::tempdir;

        let directory = tempdir().unwrap();
        let appliance_dir = directory.path().join("appliances");
        let image_path = directory.path().join("images/custom-router.qcow2");
        fs::create_dir_all(image_path.parent().unwrap()).unwrap();
        let actual_image = b"custom router bytes";
        fs::write(&image_path, actual_image).unwrap();
        let expected_image = b"official router bytes";
        let expected_md5 = format!("{:x}", md5::Md5::digest(expected_image));
        let definition = json!({
            "appliance_id": "38aa3135-f169-4131-9b64-73605787b5ef",
            "name": "Test Router",
            "category": "router",
            "registry_version": 8,
            "settings": [{
                "name": "default",
                "default": true,
                "template_type": "qemu",
                "template_properties": {"hda_disk_image": "official-router.qcow2", "ram": 1024, "cpus": 2, "adapters": 4}
            }],
            "images": [{
                "filename": "official-router.qcow2",
                "filesize": actual_image.len(),
                "md5sum": expected_md5
            }],
            "versions": [{
                "name": "1.0",
                "settings": "default",
                "images": {"hda_disk_image": "official-router.qcow2"}
            }]
        });
        let catalog = MarketplaceCatalog::with_directory(&appliance_dir);
        let record = catalog
            .import_appliance_file(
                "test-router.gns3a",
                &serde_json::to_vec(&definition).unwrap(),
            )
            .unwrap();
        assert_eq!(record.versions.len(), 1);
        assert_eq!(record.versions[0].name, "1.0");
        assert_eq!(record.versions[0].images[0].key, "hda_disk_image");

        let local_templates = vec![json!({
            "template_id": "qemu-custom",
            "name": "custom-router.qcow2",
            "template_type": "qemu",
            "hda_disk_image": image_path,
            "memory": 512,
            "cpus": 1,
            "adapters": 8
        })];
        let manager = catalog.appliance_manager(&local_templates).await.unwrap();
        assert_eq!(manager.custom_image_options.len(), 1);
        assert_eq!(
            manager.custom_image_options[0]["template_id"],
            "qemu-custom"
        );
        let image = &manager.appliances[0].versions[0].images[0];
        assert_eq!(image.status, "mismatch");
        assert_eq!(
            image.actual_md5sum.as_deref(),
            Some(format!("{:x}", md5::Md5::digest(actual_image)).as_str())
        );
        let rejected = catalog
            .set_image_binding(
                "test-router.gns3a",
                "1.0",
                "hda_disk_image",
                "qemu-custom",
                false,
                &local_templates,
            )
            .await;
        assert!(matches!(rejected, Err(error) if error.contains("explicit checksum override")));

        catalog
            .set_image_binding(
                "test-router.gns3a",
                "1.0",
                "hda_disk_image",
                "qemu-custom",
                true,
                &local_templates,
            )
            .await
            .unwrap();
        let manager = catalog.appliance_manager(&local_templates).await.unwrap();
        let image = &manager.appliances[0].versions[0].images[0];
        assert_eq!(image.status, "override");
        assert_eq!(image.assigned_image_id.as_deref(), Some("qemu-custom"));
        assert!(image.allow_checksum_mismatch);
        let templates = catalog.templates(local_templates.clone());
        let appliance_template = templates
            .iter()
            .find(|template| template["appliance_id"] == record.appliance_id)
            .unwrap();
        assert_eq!(appliance_template["appliance_version"], "1.0");
        assert_eq!(
            appliance_template["hda_disk_image"],
            image_path.display().to_string()
        );
    }

    #[tokio::test]
    async fn appliance_manager_matches_a_renamed_image_by_published_md5() {
        use std::fs;
        use tempfile::tempdir;

        let directory = tempdir().unwrap();
        let appliance_dir = directory.path().join("appliances");
        let image_path = directory.path().join("images/renamed-router.qcow2");
        fs::create_dir_all(image_path.parent().unwrap()).unwrap();
        let image_bytes = b"same image content";
        fs::write(&image_path, image_bytes).unwrap();
        let md5sum = format!("{:x}", md5::Md5::digest(image_bytes));
        let definition = json!({
            "appliance_id": "ea9b6e26-bd7c-4d11-9ec0-6b9942296528",
            "name": "Hash Matched Router",
            "category": "router",
            "registry_version": 8,
            "settings": [{
                "name": "default",
                "default": true,
                "template_type": "qemu",
                "template_properties": {"hda_disk_image": "official-name.qcow2", "ram": 512, "cpus": 1, "adapters": 4}
            }],
            "images": [{
                "filename": "official-name.qcow2",
                "filesize": image_bytes.len(),
                "md5sum": md5sum
            }],
            "versions": [{
                "name": "v1",
                "settings": "default",
                "images": {"hda_disk_image": "official-name.qcow2"}
            }]
        });
        let catalog = MarketplaceCatalog::with_directory(&appliance_dir);
        catalog
            .import_appliance_file(
                "hash-matched-router.gns3a",
                &serde_json::to_vec(&definition).unwrap(),
            )
            .unwrap();
        let local_templates = vec![json!({
            "template_id": "qemu-renamed",
            "name": "renamed-router.qcow2",
            "template_type": "qemu",
            "hda_disk_image": image_path.display().to_string(),
            "memory": 512,
            "cpus": 1,
            "adapters": 4
        })];

        let manager = catalog.appliance_manager(&local_templates).await.unwrap();
        let requirement = &manager.appliances[0].versions[0].images[0];
        assert_eq!(requirement.status, "verified");
        assert_eq!(
            requirement.assigned_image_id.as_deref(),
            Some("qemu-renamed")
        );
        assert_eq!(requirement.actual_md5sum.as_deref(), Some(md5sum.as_str()));
        let templates = catalog.templates(local_templates);
        assert!(templates.iter().any(|template| {
            template["appliance_id"] == "ea9b6e26-bd7c-4d11-9ec0-6b9942296528"
                && template["hda_disk_image"] == image_path.display().to_string()
        }));
    }

    #[test]
    fn imported_gns3a_maps_only_to_matching_installed_runtime_images() {
        let appliance = json!({
            "appliance_id": "38aa3135-f169-4131-9b64-73605787b5ef",
            "name": "Cisco 7200",
            "registry_version": 4,
            "dynamips": {"platform": "c7200", "ram": 512, "idlepc": "0x60630d08"},
            "versions": [{"name": "15.3", "images": {"image": "c7200.image"}}]
        });
        let record = summarize_appliance("cisco-7200.gns3a", &appliance, true).unwrap();
        let installed = vec![json!({
            "template_id": "base-template",
            "name": "ishare-42-c7200.image · c7200",
            "template_type": "dynamips",
            "platform": "c7200",
            "image": "/images/dynamips/ishare-42-c7200.image",
            "adapters": 1,
            "memory": 256,
            "idlepc": ""
        })];
        let templates = appliance_runtime_templates(&record, &appliance, &installed, &[]);
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0]["name"], "Cisco 7200 · 15.3");
        assert_eq!(templates[0]["memory"], 512);
        assert_eq!(templates[0]["idlepc"], "0x60630d08");
        assert_eq!(
            templates[0]["image"],
            "/images/dynamips/ishare-42-c7200.image"
        );
    }

    #[test]
    fn qemu_appliances_with_unimplemented_required_image_slots_are_not_runnable_templates() {
        let appliance = json!({
            "appliance_id": "77246218-e9f0-4df9-89dd-1cebf77e36de",
            "name": "Multi-disk appliance",
            "registry_version": 4,
            "qemu": {"ram": 512, "adapters": 2},
            "images": [
                {"filename": "system.qcow2", "filesize": 1024},
                {"filename": "data.qcow2", "filesize": 2048}
            ],
            "versions": [{"name": "1.0", "images": {
                "hda_disk_image": "system.qcow2",
                "hdb_disk_image": "data.qcow2"
            }}]
        });
        let record = summarize_appliance("multi-disk.gns3a", &appliance, true).unwrap();
        assert!(!record.versions[0].runtime_supported);
        assert!(
            record.versions[0]
                .images
                .iter()
                .any(|image| { image.key == "hdb_disk_image" && !image.assignable })
        );
        let installed = vec![json!({
            "template_id": "qemu-base",
            "name": "system.qcow2",
            "template_type": "qemu",
            "hda_disk_image": "/images/qemu/system.qcow2",
            "adapters": 1,
            "memory": 512,
            "cpus": 1
        })];

        assert!(appliance_runtime_templates(&record, &appliance, &installed, &[]).is_empty());
    }

    #[test]
    fn imports_a_cisco_3620_as_a_supported_c3600_template_with_ethernet() {
        let appliance = json!({
            "appliance_id": "7ada6336-7280-4306-9f32-6b1e242ae989",
            "name": "Cisco 3620",
            "registry_version": 4,
            "dynamips": {
                "platform": "c3600",
                "chassis": "3620",
                "ram": 192,
                "nvram": 256,
                "startup_config": "ios_base_startup-config.txt"
            },
            "images": [{"filename": "c3620.image", "filesize": 100}],
            "versions": [{"name": "122-26c", "images": {"image": "c3620.image"}, "idlepc": "0x603a8bac"}]
        });
        let record = summarize_appliance("cisco-3620.gns3a", &appliance, true).unwrap();
        assert!(record.versions[0].runtime_supported);
        let installed = vec![json!({
            "template_id": "dynamips-base",
            "name": "c3620.image · c7200",
            "template_type": "dynamips",
            "platform": "c7200",
            "image": "/images/dynamips/c3620.image",
            "adapters": 1,
            "memory": 256,
            "nvram": 128,
            "slot0": "C7200-IO-FE",
            "idlepc": ""
        })];

        let templates = appliance_runtime_templates(&record, &appliance, &installed, &[]);
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0]["platform"], "c3600");
        assert_eq!(templates[0]["chassis"], "3620");
        assert_eq!(templates[0]["slot0"], "NM-1FE-TX");
        assert_eq!(templates[0]["memory"], 192);
        assert_eq!(templates[0]["nvram"], 256);
        assert_eq!(templates[0]["idlepc"], "0x603a8bac");
        assert_eq!(templates[0]["image"], "/images/dynamips/c3620.image");
    }

    #[test]
    fn marketplace_appliance_names_cannot_be_used_as_paths() {
        assert_eq!(
            validate_appliance_file_name("cisco-7200.gns3a").unwrap(),
            "cisco-7200.gns3a"
        );
        assert!(validate_appliance_file_name("../cisco-7200.gns3a").is_err());
        assert!(validate_appliance_file_name("cisco-7200.json").is_err());
    }

    #[test]
    fn ishare_download_paths_and_filenames_are_confined_to_the_global_library() {
        assert_eq!(
            safe_catalog_component("vios 15.5/../../etc").unwrap(),
            "vios-15.5-..-..-etc"
        );
        assert!(safe_download_filename("virtioa.qcow2"));
        assert!(safe_download_filename("c7200-[10].image"));
        assert!(!safe_download_filename("../payload.image"));
        assert!(!safe_download_filename("payload;touch-root.image"));
        assert!(safe_ishare_path("/addons/qemu/vios/virtioa.qcow2"));
        assert!(safe_ishare_path("/addons/dynamips/c7200-%5B10%5D.image"));
        assert!(!safe_ishare_path("/addons/qemu/../../etc/passwd"));
        assert!(!safe_ishare_path("/addons/qemu/%2e%2e/etc/passwd"));
        assert!(valid_ishare_host("drive.labhub.eu.org"));
        assert!(!valid_ishare_host("127.0.0.1"));
        assert!(is_supported_qemu_extension("qcow2"));
        assert!(!is_supported_qemu_extension("sh"));
    }

    #[tokio::test]
    async fn downloads_an_image_file_and_checks_the_published_hashes() {
        let body = b"verified emulator image";
        let md5 = format!("{:x}", md5::Md5::digest(body));
        let sha1 = format!("{:x}", sha1::Sha1::digest(body));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = socket.read(&mut request).await.unwrap();
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            socket.write_all(body).await.unwrap();
        });
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("verified.img");
        let catalog = MarketplaceCatalog::with_directory(directory.path());
        let manifest = json!({
            "size": body.len(),
            "checksum": { "md5": md5, "sha1": sha1 }
        });

        catalog
            .download_verified_file(&format!("http://{address}/image"), &target, &manifest)
            .await
            .unwrap();
        server.await.unwrap();

        assert_eq!(tokio::fs::read(target).await.unwrap(), body);
    }

    #[tokio::test]
    #[ignore = "requires internet access to the public iShare2 and GNS3 registries"]
    async fn searches_live_ishare2_and_gns3_appliance_catalogs() {
        let directory = tempfile::tempdir().unwrap();
        let catalog = MarketplaceCatalog::with_directory(directory.path());
        let image_results = catalog.search_ishare2("c7200", "dynamips").await.unwrap();
        assert!(!image_results.is_empty());
        let appliance_results = catalog.search_gns3_appliances("cisco-7200").await.unwrap();
        assert!(!appliance_results.is_empty());
        let imported = catalog.import_appliance("cisco-7200.gns3a").await.unwrap();
        assert!(imported.imported);
        assert!(!imported.required_files.is_empty());
        assert!(directory.path().join("cisco-7200.gns3a").is_file());
    }
}
