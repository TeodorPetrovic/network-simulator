use super::*;

impl MarketplaceCatalog {
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

    pub(super) async fn ishare_index(&self) -> Result<Value, String> {
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

    pub(super) async fn gns3_appliance_files(&self) -> Result<Vec<GithubContent>, String> {
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

    pub(super) async fn fetch_appliance(&self, file_name: &str) -> Result<Value, String> {
        let url = format!("{GNS3_RAW_APPLIANCES}/{file_name}");
        let bytes = self.fetch_limited(&url, MAX_APPLIANCE_BYTES).await?;
        let appliance: Value = serde_json::from_slice(&bytes)
            .map_err(|error| format!("GNS3 appliance file is invalid JSON: {error}"))?;
        summarize_appliance(file_name, &appliance, false)?;
        Ok(appliance)
    }

    pub(super) async fn fetch_limited(
        &self,
        url: &str,
        max_bytes: usize,
    ) -> Result<Vec<u8>, String> {
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

    pub(super) fn imported_appliance_file_names(&self) -> HashSet<String> {
        self.imported_appliances()
            .unwrap_or_default()
            .into_iter()
            .map(|record| record.file_name)
            .collect()
    }
}
