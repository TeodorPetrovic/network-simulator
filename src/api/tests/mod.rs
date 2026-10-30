use axum::{
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::Value;
use tempfile::TempDir;
use tokio::time::timeout;
use tower::ServiceExt;

use super::{
    ApiError, AppState, attach_runtime_nio_with_rollback, auth, canonical_capture_file_path_in,
    dynamips_options_from_template, link_filters_for_endpoint, parse_docker_resize_control, router,
    safe_capture_file_name, suspend_node, validate_link_filters, validate_node_configuration_patch,
    vpcs_startup_script,
};
use crate::storage::Store;
use crate::{
    dynamips::DynamipsRuntime, qemu::QemuRuntime, runtime::DockerRuntime, vpcs::VpcsRuntime,
};

#[tokio::test]
async fn client_routes_fall_back_to_index_from_a_temporary_fixture() {
    let directory = tempfile::tempdir().expect("temporary frontend directory");
    std::fs::write(directory.path().join("index.html"), "fixture app shell")
        .expect("write fixture app shell");

    let response = super::spa_fallback_router::<()>(directory.path())
        .with_state(())
        .oneshot(
            Request::builder()
                .uri("/projects/fixture-id")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"fixture app shell");
}

async fn test_router() -> (axum::Router, TempDir) {
    test_router_with_vpcs(false).await
}

async fn test_router_with_vpcs(vpcs_available: bool) -> (axum::Router, TempDir) {
    let directory = TempDir::new().expect("temporary test directory");
    let client_dir = directory.path().join("client");
    std::fs::create_dir_all(client_dir.join("assets")).expect("frontend fixture assets directory");
    std::fs::write(
        client_dir.join("index.html"),
        "<html>Network Studio<link href=\"/assets/app.css\"></html>",
    )
    .expect("write frontend fixture shell");
    std::fs::write(client_dir.join("assets/app.css"), "body{}")
        .expect("write frontend fixture asset");
    let database = directory.path().join("controller.sqlite");
    let store = Store::connect(database.to_str().expect("UTF-8 temporary path"))
        .await
        .expect("test database connects");
    let docker = directory.path().join("docker-test-double");
    let state_file = directory.path().join("container-state");
    let docker_arguments = directory.path().join("docker-arguments.log");
    let script = format!(
        "#!/bin/sh\nstate_file='{}'\nprintf '%s\\n' \"$*\" >> '{}'\ncase \"$1\" in\n  info) echo 29.6.2 ;;\n  image) if [ \"$2\" = ls ]; then printf '{{\"Repository\":\"alpine\",\"Tag\":\"latest\",\"ID\":\"sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"}}\\n'; else echo sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef; fi ;;\n  create) echo 0123456789abcdef; echo created > \"$state_file\" ;;\n  inspect) cat \"$state_file\" ;;\n  start|unpause|restart) echo running > \"$state_file\" ;;\n  stop) echo exited > \"$state_file\" ;;\n  pause) echo paused > \"$state_file\" ;;\n  network) if [ \"$2\" = create ]; then echo test-network-id; fi; exit 0 ;;\n  ps) exit 0 ;;\n  *) exit 0 ;;\nesac\n",
        state_file.display(),
        docker_arguments.display()
    );
    let script = script.replace(
        "  ps) exit 0 ;;",
        "  attach) printf 'DOCKER-ATTACHED\\n'; cat ;;\n  ps) exit 0 ;;",
    );
    std::fs::write(&docker, script).expect("write Docker test double");
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755))
        .expect("make Docker test double executable");
    let qemu_images = directory.path().join("qemu-images");
    std::fs::create_dir_all(&qemu_images).expect("QEMU image directory exists");
    std::fs::write(qemu_images.join("router.qcow2"), b"base disk").expect("QEMU base disk exists");
    let qemu = directory.path().join("qemu-system-x86_64");
    let qemu_img = directory.path().join("qemu-img-test-double");
    let argument_log = directory.path().join("qemu-arguments.log");
    let test_binary = std::env::current_exe().expect("Rust test executable is available");
    let qemu_script = format!(
        "#!/bin/sh\nif [ \"$1\" = -version ]; then echo 'QEMU emulator version 9.0'; exit 0; fi\nprintf '%s\\n' \"$*\" >> '{}'\nqmp=''\npidfile=''\nwhile [ $# -gt 0 ]; do case \"$1\" in -qmp) qmp=\"$2\"; shift 2 ;; -pidfile) pidfile=\"$2\"; shift 2 ;; *) shift ;; esac; done\nqmp=${{qmp#unix:}}\nqmp=${{qmp%%,*}}\nNETWORK_SIMULATOR_TEST_QMP_SOCKET=\"$qmp\" NETWORK_SIMULATOR_TEST_QMP_PIDFILE=\"$pidfile\" '{}' --exact qemu::tests::qmp_process_fixture --nocapture >/dev/null 2>&1 &\necho $! > \"$pidfile\"\n",
        argument_log.display(),
        test_binary.display()
    );
    let qemu_img_script = "#!/bin/sh\ncase \"$1\" in\n  --version) echo 'qemu-img version 9.0' ;;\n  info) echo '{\"format\":\"qcow2\",\"virtual-size\":1048576}' ;;\n  create) for target do :; done; touch \"$target\" ;;\n  *) exit 1 ;;\nesac\n";
    std::fs::write(&qemu, qemu_script).expect("write QEMU test double");
    std::fs::write(&qemu_img, qemu_img_script).expect("write qemu-img test double");
    std::fs::set_permissions(&qemu, std::fs::Permissions::from_mode(0o755))
        .expect("make QEMU test double executable");
    std::fs::set_permissions(&qemu_img, std::fs::Permissions::from_mode(0o755))
        .expect("make qemu-img test double executable");
    let vpcs_binary = if vpcs_available {
        let binary = directory.path().join("vpcs-test-double");
        std::fs::write(
            &binary,
            "#!/bin/sh\nif [ \"$1\" = -v ]; then echo 'VPCS version 0.8.5'; exit 0; fi\nexit 1\n",
        )
        .expect("write VPCS test double");
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755))
            .expect("make VPCS test double executable");
        binary
    } else {
        std::env::var_os("NETWORK_SIMULATOR_TEST_VPCS")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| directory.path().join("vpcs-test-double-not-installed"))
    };
    (
        super::router_with_client_dir(
            AppState::with_runtimes(
                store,
                DockerRuntime::with_binary(docker),
                QemuRuntime::with_paths(
                    qemu,
                    qemu_img,
                    qemu_images,
                    directory.path().join("qemu-state"),
                    directory.path().join("qemu-sockets"),
                ),
                VpcsRuntime::with_paths(vpcs_binary, directory.path().join("vpcs-state")),
                DynamipsRuntime::with_paths(
                    directory.path().join("dynamips-test-double-not-installed"),
                    directory.path().join("dynamips-images"),
                    directory.path().join("dynamips-state"),
                ),
                crate::marketplace::MarketplaceCatalog::with_directory(
                    directory.path().join("appliances"),
                ),
            ),
            &client_dir,
        ),
        directory,
    )
}

pub(super) async fn request(
    app: &axum::Router,
    method: Method,
    uri: String,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(uri);
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(builder.body(body).expect("request builds"))
        .await
        .expect("router returns a response");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("response body reads")
        .to_bytes();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or_else(|error| {
            panic!(
                "response is not JSON ({error}), status={status}, body={:?}",
                String::from_utf8_lossy(&bytes)
            )
        })
    };
    (status, json)
}

pub(super) async fn request_bytes(
    app: &axum::Router,
    method: Method,
    uri: String,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router returns a response");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("response body reads")
        .to_bytes()
        .to_vec();
    (status, headers, bytes)
}

pub(super) async fn create_custom_from_runtime(
    app: &axum::Router,
    runtime: &str,
    name: &str,
) -> Value {
    let (status, manager) = request(
        app,
        Method::GET,
        "/v3/marketplace/gns3/manager".into(),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "appliance manager response: {manager}"
    );
    let source_template_id =
        manager["custom_image_options"]
            .as_array()
            .unwrap()
            .iter()
            .find(|template| template["template_type"] == runtime)
            .unwrap_or_else(|| panic!("test runtime {runtime} has no installed image option"))
            ["template_id"]
            .as_str()
            .expect("runtime image has a template ID");
    let (status, appliance) = request(
        app,
        Method::POST,
        "/v3/marketplace/gns3/custom".into(),
        Some(serde_json::json!({
            "name": name,
            "source_template_id": source_template_id,
            "runtime_settings": {}
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    appliance
}

pub(super) async fn websocket_status(app: &axum::Router, uri: String) -> StatusCode {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        time::{Duration, timeout},
    };

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("temporary HTTP listener binds");
    let address = listener.local_addr().expect("HTTP listener has an address");
    let server_app = app.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, server_app)
            .await
            .expect("test HTTP server runs");
    });
    let mut client = TcpStream::connect(address)
        .await
        .expect("WebSocket client connects");
    client
        .write_all(
            format!(
                "GET {uri} HTTP/1.1\r\nHost: localhost\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .expect("WebSocket upgrade request writes");
    let mut response = [0_u8; 512];
    let length = timeout(Duration::from_secs(2), client.read(&mut response))
        .await
        .expect("HTTP response arrives before timeout")
        .expect("HTTP response reads");
    server.abort();
    let first_line = String::from_utf8_lossy(&response[..length]);
    first_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .and_then(|status| StatusCode::from_u16(status).ok())
        .expect("HTTP response contains a status code")
}

#[path = "api_flows.rs"]
mod api_flows;
#[path = "builtin_lifecycle.rs"]
mod builtin_lifecycle;
#[path = "live_chr.rs"]
mod live_chr;
#[path = "live_hub.rs"]
mod live_hub;
#[path = "live_network_setup.rs"]
mod live_network_setup;
#[path = "live_switch.rs"]
mod live_switch;
#[path = "runtime_consoles.rs"]
mod runtime_consoles;
#[path = "template_routes.rs"]
mod template_routes;
