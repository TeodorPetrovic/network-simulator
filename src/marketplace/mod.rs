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

mod bindings;
mod cache;
mod gns3;
mod ishare;
mod parsing;
mod templates;
#[cfg(test)]
mod tests;
mod validation;

use parsing::*;
use templates::*;
use validation::*;
