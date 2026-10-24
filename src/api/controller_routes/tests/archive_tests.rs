use std::io::{Cursor, Write};

use super::{
    archive_project_file_path, image_dimensions, is_qemu_image_upload, normalize_imported_project,
    project_api_to_gns3, read_project_archive, strip_node_runtime_fields,
    unique_project_image_name, write_image_upload, write_project_archive,
};
use axum::{body::Body, http::StatusCode};
use serde_json::json;
use std::collections::HashSet;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

fn compressed_project_archive(method: CompressionMethod) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            "project.gns3",
            SimpleFileOptions::default().compression_method(method),
        )
        .unwrap();
    writer
        .write_all(br#"{"name":"Compressed","nodes":[],"links":[],"drawings":[]}"#)
        .unwrap();
    writer.finish().unwrap().into_inner()
}

#[test]
fn stored_project_archive_round_trips() {
    let value = br#"{"name":"Lab","nodes":[],"links":[],"drawings":[]}"#;
    let archive = write_project_archive(value.to_vec(), Vec::new())
        .ok()
        .unwrap();
    assert_eq!(
        read_project_archive(&archive).ok().unwrap().project["name"],
        "Lab"
    );
}

#[test]
fn deflate_and_upstream_zstandard_archives_are_supported() {
    for method in [CompressionMethod::Deflated, CompressionMethod::Zstd] {
        let imported = read_project_archive(&compressed_project_archive(method))
            .ok()
            .unwrap();
        assert_eq!(imported.project["name"], "Compressed");
    }
}

#[test]
fn project_files_round_trip_and_reject_unsafe_archive_paths() {
    let archive = write_project_archive(
        br#"{"name":"Files","nodes":[],"links":[],"drawings":[]}"#.to_vec(),
        vec![("project-files/readme.md".into(), b"hello".to_vec())],
    )
    .ok()
    .unwrap();
    let imported = read_project_archive(&archive).ok().unwrap();
    assert_eq!(
        imported.project_files,
        vec![("readme.md".to_owned(), b"hello".to_vec())]
    );
    assert!(archive_project_file_path("project-files/../../outside").is_err());
    assert!(archive_project_file_path("project-files/../outside").is_err());
}

#[test]
fn gns3_topology_and_properties_round_trip() {
    let document = project_api_to_gns3(
        json!({"project_id":"old-project","name":"Router lab","status":"opened"}),
        vec![json!({
            "node_id":"node-1",
            "project_id":"old-project",
            "node_type":"qemu",
            "name":"Router",
            "status":"started",
            "x":12,
            "y":-4,
            "hda_disk_image":"router.qcow2",
            "properties":{"adapter_type":"virtio-net-pci"},
            "runtime_overlay":"/old/overlay.qcow2"
        })],
        vec![json!({"link_id":"link-1","project_id":"old-project","nodes":[]})],
        vec![json!({"drawing_id":"drawing-1","project_id":"old-project"})],
    );
    assert!(document.get("project_id").is_none());
    assert_eq!(document["topology"]["nodes"][0]["status"], "stopped");
    assert_eq!(
        document["topology"]["nodes"][0]["properties"]["hda_disk_image"],
        "router.qcow2"
    );
    assert_eq!(
        document["topology"]["nodes"][0]["properties"]["adapter_type"],
        "virtio-net-pci"
    );
    assert!(document["topology"]["nodes"][0]["properties"]["runtime_overlay"].is_null());
    assert!(document["topology"]["links"][0].get("project_id").is_none());

    let normalized = normalize_imported_project(document);
    assert_eq!(normalized["nodes"][0]["hda_disk_image"], "router.qcow2");
    assert_eq!(normalized["nodes"][0]["adapter_type"], "virtio-net-pci");
    assert_eq!(normalized["nodes"][0]["properties"], json!({}));
    assert!(normalized.get("topology").is_none());
}

#[test]
fn project_archive_reads_upstream_image_directories() {
    let archive = write_project_archive(
        br#"{"name":"Image lab","topology":{"nodes":[],"links":[],"drawings":[]}}"#.to_vec(),
        vec![("images/QEMU/router.qcow2".into(), b"qemu bytes".to_vec())],
    )
    .unwrap();
    let imported = read_project_archive(&archive).unwrap();
    assert_eq!(imported.project["name"], "Image lab");
    assert_eq!(imported.images.len(), 1);
    assert_eq!(imported.images[0].image_type, "qemu");
    assert_eq!(imported.images[0].file_name, "router.qcow2");
    assert_eq!(imported.images[0].contents, b"qemu bytes");
}

#[test]
fn image_exports_choose_unique_archive_names_for_colliding_basenames() {
    let mut names = HashSet::<(String, String)>::new();
    assert_eq!(
        unique_project_image_name("router.qcow2", "qemu", &mut names),
        "router.qcow2"
    );
    assert_eq!(
        unique_project_image_name("router.qcow2", "qemu", &mut names),
        "router-2.qcow2"
    );
    assert_eq!(
        unique_project_image_name("router.qcow2", "dynamips", &mut names),
        "router.qcow2"
    );
}

#[test]
fn malformed_zip_headers_return_errors_without_panicking() {
    let mut oversized_name_length = vec![0_u8; 30];
    oversized_name_length[..4].copy_from_slice(b"PK\x03\x04");
    oversized_name_length[26..28].copy_from_slice(&u16::MAX.to_le_bytes());
    assert_eq!(
        read_project_archive(&oversized_name_length)
            .err()
            .unwrap()
            .status,
        StatusCode::BAD_REQUEST
    );

    let valid_archive = compressed_project_archive(CompressionMethod::Stored);
    let central_header = valid_archive
        .windows(4)
        .position(|window| window == b"PK\x01\x02")
        .unwrap();
    for field_offset in [26, 28] {
        let mut malformed = valid_archive.clone();
        malformed[field_offset..field_offset + 2].copy_from_slice(&u16::MAX.to_le_bytes());
        assert_eq!(
            read_project_archive(&malformed).err().unwrap().status,
            StatusCode::BAD_REQUEST,
            "malformed local-header length at {field_offset}"
        );
    }
    for field_offset in [28, 30, 32] {
        let mut malformed = valid_archive.clone();
        let field = central_header + field_offset;
        malformed[field..field + 2].copy_from_slice(&u16::MAX.to_le_bytes());
        assert_eq!(
            read_project_archive(&malformed).err().unwrap().status,
            StatusCode::BAD_REQUEST,
            "malformed central-header length at {field_offset}"
        );
    }

    let mut truncated_central_header = compressed_project_archive(CompressionMethod::Stored);
    truncated_central_header.truncate(truncated_central_header.len() - 10);
    assert_eq!(
        read_project_archive(&truncated_central_header)
            .err()
            .unwrap()
            .status,
        StatusCode::BAD_REQUEST
    );

    let mut truncated_local_header = vec![0_u8; 29];
    truncated_local_header[..4].copy_from_slice(b"PK\x03\x04");
    assert_eq!(
        read_project_archive(&truncated_local_header)
            .err()
            .unwrap()
            .status,
        StatusCode::BAD_REQUEST
    );
}

#[test]
fn imported_runtime_fields_are_removed_from_node_objects() {
    let mut node = json!({
        "node_id":"source",
        "container_id":"abc",
        "runtime_overlay":"/source/overlay.qcow2",
        "runtime_qmp_socket":"/source/qmp.sock",
        "runtime_serial_socket":"/source/serial.sock",
        "runtime_pid_file":"/source/vm.pid",
        "runtime_log_file":"/source/vm.log",
        "runtime_working_dir":"/source",
        "runtime_hypervisor_port":7200,
        "runtime_hdb_overlay":"/source/disk.qcow2",
        "runtime_ghost_file":"/source/ghost.iso",
        "image_digest":"sha256:source",
        "dynamips_id":1,
        "qmp_path":"/source/qmp",
        "serial_path":"/source/serial",
        "overlay_path":"/source/overlay",
        "log_path":"/source/log",
        "working_directory":"/source",
        "qemu_pid":123,
        "pid_file":"/source/legacy.pid",
        "console":1234,
        "name":"Router"
    });
    strip_node_runtime_fields(&mut node);
    assert_eq!(node["name"], "Router");
    for key in [
        "container_id",
        "image_digest",
        "dynamips_id",
        "runtime_overlay",
        "runtime_qmp_socket",
        "runtime_serial_socket",
        "runtime_pid_file",
        "runtime_log_file",
        "runtime_working_dir",
        "runtime_hypervisor_port",
        "runtime_hdb_overlay",
        "runtime_ghost_file",
        "qmp_path",
        "serial_path",
        "overlay_path",
        "log_path",
        "working_directory",
        "qemu_pid",
        "pid_file",
        "console",
    ] {
        assert!(node.get(key).is_none(), "{key} was retained");
    }
}

#[tokio::test]
async fn image_upload_stream_is_bounded_and_cleans_temporary_files() {
    let directory = tempfile::tempdir().unwrap();
    let temporary_path = directory.path().join(".test.upload");
    let error = write_image_upload(Body::from(vec![1, 2, 3, 4]), &temporary_path, 3)
        .await
        .unwrap_err();
    assert_eq!(error.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert!(!temporary_path.exists());

    let failed_body = Body::from_stream(futures_util::stream::iter([
        Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"partial")),
        Err(std::io::Error::other("client disconnected")),
    ]));
    let error = write_image_upload(failed_body, &temporary_path, 64)
        .await
        .unwrap_err();
    assert_eq!(error.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!temporary_path.exists());
    assert!(is_qemu_image_upload("router.vmdk"));
    assert!(is_qemu_image_upload("disk.vdi"));
    assert!(is_qemu_image_upload("disk.QCOW2"));
    assert!(!is_qemu_image_upload("c7200.bin"));
}
#[test]
fn png_dimensions_are_read_from_ihdr() {
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    png.extend_from_slice(&640u32.to_be_bytes());
    png.extend_from_slice(&480u32.to_be_bytes());
    assert_eq!(image_dimensions(&png), Some((640, 480)));
}
