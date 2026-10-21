use super::MarketplaceCatalog;
use super::parsing::{
    is_supported_qemu_extension, ishare_row_matches_query, safe_catalog_component,
    safe_download_filename, safe_ishare_path, summarize_appliance, valid_ishare_host,
    validate_appliance_file_name,
};
use super::templates::appliance_runtime_templates;
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
