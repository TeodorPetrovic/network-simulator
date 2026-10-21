use super::*;

    use super::{
        DockerCustomAdapter, DockerExtraConfig, DockerRuntime, DockerTemplateOptions,
        docker_aux_exec_command, docker_hostname, docker_vnc_tools_error, node_network_name,
        normalize_docker_state, parse_environment, parse_extra_hosts, parse_start_command,
        proxy_docker_http, validate_container_volume, validate_docker_extra_configs,
        validate_docker_runtime_options,
    };
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
        time::Duration,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::{TcpStream, UnixListener},
    };

    fn docker_test_double(directory: &std::path::Path) -> (PathBuf, PathBuf) {
        let binary = directory.join("docker-test-double");
        let log = directory.join("docker-args.log");
        fs::write(
            &binary,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\ncase \"$1\" in image) echo sha256:0123456789abcdef;; create) echo 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef;; esac\n",
                log.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
        (binary, log)
    }

    #[test]
    fn maps_docker_states_to_controller_states() {
        assert_eq!(normalize_docker_state("running"), "started");
        assert_eq!(normalize_docker_state("paused"), "suspended");
        assert_eq!(normalize_docker_state("exited"), "stopped");
    }

    #[test]
    fn derives_stable_safe_names_for_node_networks() {
        assert_eq!(node_network_name("ab12-cd34"), "ns-node-ab12cd34");
    }

    #[test]
    fn docker_hostnames_are_valid_and_stable_for_node_names() {
        assert_eq!(
            docker_hostname("Alpine Router 1", "01234567"),
            "alpine-router-1"
        );
        assert_eq!(docker_hostname("---", "01234567"), "node-01234567");
    }

    #[test]
    fn parses_docker_command_without_running_a_host_shell() {
        assert_eq!(
            parse_start_command("/bin/sh -c 'sleep 60'").unwrap(),
            ["/bin/sh", "-c", "sleep 60"]
        );
        assert!(parse_start_command("/bin/sh -c 'sleep 60").is_err());
    }

    #[test]
    fn validates_docker_template_environment_hosts_and_persistent_paths() {
        assert_eq!(
            parse_environment("_LAB=router\nMODE=ospf").unwrap(),
            ["_LAB=router", "MODE=ospf"]
        );
        assert!(parse_environment("1INVALID=value").is_err());
        assert_eq!(
            parse_extra_hosts("gateway:192.0.2.1").unwrap(),
            ["gateway:192.0.2.1"]
        );
        assert!(parse_extra_hosts("bad host:192.0.2.1").is_err());
        assert!(validate_container_volume("/var/lib/router").is_ok());
        assert!(validate_container_volume("/var/../etc").is_err());
    }

    #[test]
    fn validates_docker_console_and_custom_adapter_schema() {
        let mut options = DockerTemplateOptions::default();
        options.console_type = "https".to_owned();
        options.console_resolution = "1600x900".to_owned();
        options.aux_type = "ssh".to_owned();
        options.custom_adapters.push(DockerCustomAdapter {
            adapter_number: 2,
            mac_address: Some("02:00:00:00:00:02".to_owned()),
            ..DockerCustomAdapter::default()
        });
        assert!(validate_docker_runtime_options(&options).is_ok());
        options.console_resolution = "wide".to_owned();
        assert!(validate_docker_runtime_options(&options).is_err());
    }

    #[test]
    fn validates_docker_injected_config_paths_content_and_conflicts() {
        let config = DockerExtraConfig {
            target: "/etc/router/router.conf".to_owned(),
            content: "hostname lab".to_owned(),
        };
        assert!(validate_docker_extra_configs(&[config], None).is_ok());
        assert!(
            validate_docker_extra_configs(
                &[DockerExtraConfig {
                    target: "/etc/../outside".to_owned(),
                    content: "bad".to_owned(),
                }],
                None
            )
            .is_err()
        );
        assert!(
            validate_docker_extra_configs(
                &[DockerExtraConfig {
                    target: "/startup-config".to_owned(),
                    content: "file".to_owned(),
                }],
                Some("inline")
            )
            .is_err()
        );
        assert!(
            validate_docker_extra_configs(
                &[
                    DockerExtraConfig {
                        target: "/etc/router".to_owned(),
                        content: "directory mount".to_owned(),
                    },
                    DockerExtraConfig {
                        target: "/etc/router/router.conf".to_owned(),
                        content: "nested file".to_owned(),
                    },
                ],
                None
            )
            .is_err()
        );
        assert!(
            validate_docker_extra_configs(
                &[DockerExtraConfig {
                    target: "/startup-config/nested.conf".to_owned(),
                    content: "nested file".to_owned(),
                }],
                Some("inline")
            )
            .is_err()
        );
    }

    #[test]
    fn imported_docker_container_ids_require_a_runtime_state_ownership_marker() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = DockerRuntime::with_paths("docker", directory.path().join("state"));
        let container_id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(runtime.ensure_managed_container(container_id).is_err());
        runtime
            .mark_container_managed(
                "00000000-0000-4000-8000-000000000001",
                "00000000-0000-4000-8000-000000000002",
                container_id,
            )
            .unwrap();
        runtime.ensure_managed_container(container_id).unwrap();
        runtime.forget_managed_container(container_id);
        runtime.ensure_managed_container(container_id).unwrap();
    }

    #[tokio::test]
    async fn docker_none_console_type_is_valid_and_allocates_no_console_host() {
        let directory = tempfile::tempdir().unwrap();
        let runtime = DockerRuntime::with_paths("docker", directory.path().join("state"));
        let options = DockerTemplateOptions {
            console_type: "none".to_owned(),
            ..DockerTemplateOptions::default()
        };
        validate_docker_runtime_options(&options).unwrap();
        assert!(
            runtime
                .prepare_console_host(&options)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn prepares_http_and_https_console_endpoints_with_configured_web_port_and_path() {
        let directory = tempfile::tempdir().unwrap();
        let (binary, _) = docker_test_double(directory.path());
        let runtime = DockerRuntime::with_binary(binary);
        for console_type in ["http", "https"] {
            let options = DockerTemplateOptions {
                console_type: console_type.to_owned(),
                console_resolution: "1600x900".to_owned(),
                console_http_port: Some(8443),
                console_http_path: Some("/admin/".to_owned()),
                ..DockerTemplateOptions::default()
            };
            let host = runtime
                .prepare_console_host(&options)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(host.console_type, console_type);
            assert_eq!(host.http_port, 8443);
            assert_eq!(host.http_path, "/admin/");
            assert_eq!(host.resolution, "1600x900");
            assert_ne!(host.console_port, 0);
        }
    }

    #[test]
    fn docker_vnc_reports_missing_host_tools_as_unsupported() {
        let error = docker_vnc_tools_error(false, false).unwrap();
        assert!(error.message().contains("unsupported"));
        assert!(error.message().contains("Xvfb and x11vnc"));
        assert!(docker_vnc_tools_error(true, true).is_none());
    }

    #[test]
    fn builds_auxiliary_console_pty_command_with_docker_exec() {
        let command = docker_aux_exec_command(
            Path::new("/usr/bin/docker"),
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        assert!(command.contains("docker exec -i -t 0123456789abcdef"));
        assert!(command.contains("/gns3/bin/busybox sh"));
    }

    #[tokio::test]
    async fn docker_http_console_rewrites_container_host_port_in_responses() {
        let directory = tempfile::tempdir().unwrap();
        let docker = directory.path().join("docker-http-test-double");
        fs::write(
            &docker,
            "#!/bin/sh\ncr=$(printf '\\r')\nwhile IFS= read -r line; do [ \"$line\" = \"$cr\" ] && break; done\nprintf 'Location: http://127.0.0.1:8080/ui\\r\\n\\r\\n'\n",
        )
        .unwrap();
        fs::set_permissions(&docker, fs::Permissions::from_mode(0o755)).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            proxy_docker_http(stream, &docker, "0123456789abcdef", 8080, 9999)
                .await
                .unwrap();
        });
        let mut client = tokio::net::TcpStream::connect(address).await.unwrap();
        client.write_all(b"GET /ui HTTP/1.1\r\n\r\n").await.unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        server.await.unwrap();
        let response = String::from_utf8(response).unwrap();
        assert!(response.contains("http://127.0.0.1:9999/ui"));
    }

    #[tokio::test]
    #[ignore = "requires a running Docker Engine and alpine:latest"]
    async fn docker_live_http_console_bridges_to_container_web_port() {
        let image = std::env::var("NETWORK_SIMULATOR_TEST_DOCKER_IMAGE")
            .unwrap_or_else(|_| "alpine:latest".to_owned());
        let binary = std::env::var_os("NETWORK_SIMULATOR_TEST_DOCKER")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("docker"));
        let directory = tempfile::tempdir().unwrap();
        let runtime = DockerRuntime::with_paths_and_socket(
            binary,
            directory.path().join("state"),
            std::env::var_os("DOCKER_HOST")
                .and_then(|host| host.into_string().ok())
                .and_then(|host| host.strip_prefix("unix://").map(PathBuf::from))
                .unwrap_or_else(|| PathBuf::from("/var/run/docker.sock")),
        );
        let node_id = uuid::Uuid::new_v4().to_string();
        let project_id = uuid::Uuid::new_v4().to_string();
        let command = "/bin/sh -c \"mkdir -p /gns3/bin; ln -sf /bin/busybox /gns3/bin/busybox; while true; do printf 'HTTP/1.1 200 OK\\r\\nConnection: close\\r\\nContent-Length: 26\\r\\n\\r\\nhttp://127.0.0.1:8080/path' | /bin/busybox nc -l -p 8080; done\"";
        let options = DockerTemplateOptions {
            console_type: "http".to_owned(),
            console_http_port: Some(8080),
            console_http_path: Some("/path".to_owned()),
            ..DockerTemplateOptions::default()
        };
        let (container_id, _) = runtime
            .create_container_with_options(
                &project_id,
                &node_id,
                &image,
                "Runtime HTTP console test",
                Some(command),
                None,
                None,
                &options,
            )
            .await
            .expect("Docker creates the live console container");
        let result = async {
            runtime
                .lifecycle(&container_id, crate::runtime::NodeAction::Start)
                .await
                .map_err(|error| error.message().to_owned())?;
            let port = runtime
                .console_port(&container_id)
                .await
                .ok_or_else(|| "HTTP console did not allocate a host port".to_owned())?;
            let mut client = TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
                .await
                .map_err(|error| error.to_string())?;
            client
                .write_all(b"GET /path HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .await
                .map_err(|error| error.to_string())?;
            let response = tokio::time::timeout(Duration::from_secs(5), async {
                let mut response = Vec::new();
                let mut buffer = [0u8; 1024];
                loop {
                    let length = client
                        .read(&mut buffer)
                        .await
                        .map_err(|error| error.to_string())?;
                    if length == 0 {
                        return Err(format!(
                            "HTTP console closed before returning a response body: {}",
                            String::from_utf8_lossy(&response)
                        ));
                    }
                    response.extend_from_slice(&buffer[..length]);
                    if response
                        .windows(b"/path".len())
                        .any(|window| window == b"/path")
                    {
                        return Ok::<_, String>(response);
                    }
                }
            })
            .await
            .map_err(|_| "Docker HTTP console response timed out".to_owned())??;
            let response = String::from_utf8_lossy(&response);
            if !response.contains(&format!("http://127.0.0.1:{port}/path"))
                || response.contains("http://127.0.0.1:8080/path")
            {
                return Err(format!(
                    "HTTP console returned an untranslated response: {response}"
                ));
            }
            let mut auxiliary = runtime
                .attach_aux_process(&container_id)
                .map_err(|error| error.message().to_owned())?;
            let mut auxiliary_input = auxiliary
                .stdin
                .take()
                .ok_or_else(|| "Docker aux process has no input stream".to_owned())?;
            let mut auxiliary_output = auxiliary
                .stdout
                .take()
                .ok_or_else(|| "Docker aux process has no output stream".to_owned())?;
            auxiliary_input
                .write_all(b"echo AUX_RUNTIME_OK\n")
                .await
                .map_err(|error| error.to_string())?;
            let mut output = Vec::new();
            let read = async {
                let mut buffer = [0u8; 1024];
                loop {
                    let length = auxiliary_output
                        .read(&mut buffer)
                        .await
                        .map_err(|error| error.to_string())?;
                    if length == 0 {
                        return Err("Docker aux process closed before responding".to_owned());
                    }
                    output.extend_from_slice(&buffer[..length]);
                    if output
                        .windows(b"AUX_RUNTIME_OK".len())
                        .any(|window| window == b"AUX_RUNTIME_OK")
                    {
                        return Ok(());
                    }
                }
            };
            tokio::time::timeout(Duration::from_secs(5), read)
                .await
                .map_err(|_| "Docker auxiliary console shell timed out".to_owned())??;
            let _ = auxiliary.kill().await;
            let _ = auxiliary.wait().await;
            Ok::<(), String>(())
        }
        .await;
        let _ = runtime
            .remove_node_resources(&node_id, Some(&container_id))
            .await;
        if let Err(error) = result {
            panic!("Docker live console check failed: {error}");
        }
    }

    #[tokio::test]
    async fn docker_create_and_tty_resize_reach_the_runtime_cli() {
        let directory = tempfile::tempdir().unwrap();
        let (binary, log) = docker_test_double(directory.path());
        let socket_path = directory.path().join("docker.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();
        let resize_server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut chunk = [0_u8; 512];
            loop {
                let length = stream.read(&mut chunk).await.unwrap();
                request.extend_from_slice(&chunk[..length]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            stream
                .write_all(
                    b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            String::from_utf8(request).unwrap()
        });
        let runtime = DockerRuntime::with_paths_and_socket(
            binary,
            directory.path().join("state"),
            socket_path,
        );
        let options = DockerTemplateOptions {
            environment: "MODE=ospf".to_owned(),
            extra_hosts: "gateway:192.0.2.1".to_owned(),
            extra_volumes: vec!["/var/lib/router".to_owned()],
            extra_configs: vec![DockerExtraConfig {
                target: "/etc/router/router.conf".to_owned(),
                content: "router-id 192.0.2.1\n".to_owned(),
            }],
            startup_config_content: Some("hostname %h\n".to_owned()),
            ..DockerTemplateOptions::default()
        };
        let (container_id, _) = runtime
            .create_container_with_options(
                "00000000-0000-0000-0000-000000000001",
                "00000000-0000-0000-0000-000000000002",
                "router:latest",
                "Router 1",
                Some("/bin/sh -c 'sleep 60'"),
                Some(512),
                Some(1.5),
                &options,
            )
            .await
            .unwrap();
        assert_eq!(container_id.len(), 64);
        runtime.resize_tty(&container_id, 40, 120).await.unwrap();
        let calls = fs::read_to_string(log).unwrap();
        assert!(calls.contains("--env MODE=ospf"));
        assert!(calls.contains("--add-host gateway:192.0.2.1"));
        assert!(calls.contains("/var/lib/router"));
        assert!(calls.contains(":/etc/router/router.conf:ro"));
        assert!(calls.contains(":/startup-config:ro"));
        assert!(calls.contains("/bin/sh -c sleep 60"));
        let config_root = directory
            .path()
            .join("state/00000000-0000-0000-0000-000000000001/00000000-0000-0000-0000-000000000002/configs");
        assert_eq!(
            fs::read_to_string(config_root.join("etc/router/router.conf")).unwrap(),
            "router-id 192.0.2.1\n"
        );
        assert_eq!(
            fs::read_to_string(config_root.join("startup-config")).unwrap(),
            "hostname %h\n"
        );
        let resize_request = resize_server.await.unwrap();
        assert!(resize_request.starts_with(&format!(
            "POST /containers/{container_id}/resize?h=40&w=120 HTTP/1.1"
        )));
    }
