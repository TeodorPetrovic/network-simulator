use std::{
    collections::HashSet,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
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
const MAX_ISHARE_FILE_BYTES: u64 = 64 * 1024 * 1024 * 1024;

#[derive(Clone)]
pub struct MarketplaceCatalog {
    client: Client,
    download_client: Client,
    appliance_dir: PathBuf,
    cache: Arc<Mutex<CatalogCache>>,
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
    catalog: &'static str,
}

#[derive(Serialize)]
pub struct IshareDownloadResult {
    pub id: u64,
    pub name: String,
    pub image_type: String,
    pub files: Vec<String>,
    pub image_directory: String,
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
    pub category: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub registry_version: Option<u64>,
    pub template_type: Option<String>,
    pub imported: bool,
    pub source_url: String,
    pub required_files: Vec<ApplianceFile>,
}

impl MarketplaceCatalog {
    pub fn new() -> Self {
        Self::with_directory(crate::image_paths::appliance_definitions())
    }

    fn with_directory(appliance_dir: impl Into<PathBuf>) -> Self {
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
                if !needle.is_empty() && !name.to_lowercase().contains(&needle) {
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
                        files: installed_files,
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
    ) -> Result<Vec<String>, String> {
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
                        let base = safe_name
                            .strip_suffix(&format!(".{extension}"))
                            .unwrap_or(safe_name);
                        (format!("ishare-{image_id}-{base}.{extension}"), true)
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
                to_install.push((staged_path, destination));
            }
            if to_install.is_empty() {
                return Err(format!(
                    "The selected iShare2 {image_type} package contains no supported disk image file"
                ));
            }
            let mut installed_paths = Vec::new();
            for (staged_path, destination) in &to_install {
                if let Err(error) = fs::hard_link(staged_path, destination) {
                    for installed in &installed_paths {
                        let _ = fs::remove_file(installed);
                    }
                    return Err(format!("Could not install verified image: {error}"));
                }
                installed_paths.push(destination.clone());
            }
            let installed = to_install
                .into_iter()
                .map(|(_, path)| path.file_name().unwrap_or_default().to_string_lossy().into_owned())
                .collect();
            let _ = fs::remove_dir_all(&staging);
            Ok(installed)
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
            .take(MAX_SEARCH_RESULTS)
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

    pub fn templates(&self, base_templates: Vec<Value>) -> Vec<Value> {
        let records = match self.imported_appliances() {
            Ok(records) => records,
            Err(error) => {
                tracing::warn!(%error, "could not load imported GNS3 appliances");
                return base_templates;
            }
        };
        let mut result = base_templates.clone();
        for record in records {
            let path = self.appliance_dir.join(&record.file_name);
            let Ok(content) = fs::read(path) else {
                continue;
            };
            let Ok(appliance) = serde_json::from_slice::<Value>(&content) else {
                continue;
            };
            result.extend(appliance_runtime_templates(
                &record,
                &appliance,
                &base_templates,
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
                    .get("md5sum")
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
    Ok(ApplianceRecord {
        file_name: file_name.to_owned(),
        appliance_id: appliance_id.to_owned(),
        name: name.to_owned(),
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
        source_url: format!(
            "https://github.com/GNS3/gns3-registry/blob/master/appliances/{file_name}"
        ),
        required_files,
    })
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
) -> Vec<Value> {
    let versions = appliance
        .get("versions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_else(|| vec![Value::Null]);
    let runtime = match appliance_runtime_type(appliance) {
        Some("qemu") => "qemu",
        Some("dynamips") => "dynamips",
        Some("docker") => "docker",
        _ => return Vec::new(),
    };
    let properties = appliance_properties(appliance, runtime);
    let id = Uuid::parse_str(&record.appliance_id).ok();
    let mut templates = Vec::new();
    for version in versions {
        let version_name = version
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let version_settings = version.get("settings").and_then(Value::as_str);
        let settings_properties =
            appliance_settings(appliance, runtime, version_settings).unwrap_or(properties);
        let image_name = match runtime {
            "qemu" => version["images"]["hda_disk_image"]
                .as_str()
                .or_else(|| properties["hda_disk_image"].as_str()),
            "dynamips" => version["images"]["image"]
                .as_str()
                .or_else(|| properties["image"].as_str()),
            "docker" => properties["image"].as_str(),
            _ => None,
        };
        let Some(image_name) = image_name else {
            continue;
        };
        let local_template = base_templates.iter().find(|template| {
            template["template_type"].as_str() == Some(runtime)
                && template_image_matches(template, runtime, image_name)
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
        if runtime == "dynamips" {
            if let Some(platform) = settings_properties["platform"].as_str() {
                template["platform"] = json!(platform);
            }
            let idlepc = settings_properties["idlepc"]
                .as_str()
                .or_else(|| version["idlepc"].as_str());
            if let Some(idlepc) = idlepc {
                template["idlepc"] = json!(idlepc);
            }
        }
        templates.push(template);
    }
    templates
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
        Path::new(candidate)
            .file_name()
            .and_then(|name| name.to_str())
            == Path::new(image_name)
                .file_name()
                .and_then(|name| name.to_str())
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MarketplaceCatalog, appliance_runtime_templates, is_supported_qemu_extension,
        safe_catalog_component, safe_download_filename, safe_ishare_path, summarize_appliance,
        valid_ishare_host, validate_appliance_file_name,
    };
    use serde_json::json;
    use sha1::Digest;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

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
            "name": "c7200.image · c7200",
            "template_type": "dynamips",
            "platform": "c7200",
            "image": "/images/dynamips/c7200.image",
            "adapters": 1,
            "memory": 256,
            "idlepc": ""
        })];
        let templates = appliance_runtime_templates(&record, &appliance, &installed);
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0]["name"], "Cisco 7200 · 15.3");
        assert_eq!(templates[0]["memory"], 512);
        assert_eq!(templates[0]["idlepc"], "0x60630d08");
        assert_eq!(templates[0]["image"], "/images/dynamips/c7200.image");
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
