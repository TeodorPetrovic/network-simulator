# Deploying a classroom server

This service runs the Rust controller on a Linux host with Docker, QEMU/KVM,
VPCS, uBridge, and Dynamips installed. The browser UI is served by the
controller. Put a TLS reverse proxy in front of its loopback listener; do not
publish the emulator console ports or uBridge UDP sockets through the proxy.

The upstream GNS3 3.1 server checkout used for the parity work is kept at
`~/.cache/gns3-server-3.1` (branch `3.1`).

## Install and prepare the host

1. Install Docker Engine, QEMU (`qemu-system-x86_64` and `qemu-img`), VPCS,
   uBridge, Dynamips 0.2.23 or newer, libvirt's default network, and `sqlite3`.
   The classroom service account needs access to the Docker socket and
   `/dev/kvm`; `deploy/preflight.sh` checks those requirements.
2. Create a `network-simulator` service account. Add it to the `docker` and
   `kvm` groups. Install the release binary and built `frontend/dist` under
   `/opt/network-simulator`, then install
   `deploy/network-simulator.service` as
   `/etc/systemd/system/network-simulator.service`.
3. Install `deploy/network-simulator.env.example` as
   `/etc/network-simulator/network-simulator.env` and edit it. The unit
   creates `/var/lib/network-simulator`, `/run/network-simulator`, and
   `/var/log/network-simulator` with service-account ownership. Keep the
   database, images, appliance definitions, and runtime state on persistent
   storage with enough free space for classroom images.
4. Run `deploy/preflight.sh` as the service account with the same environment
   values used by systemd. The controller TCP port must be free. You can add
   other TCP ports to the check with `NETWORK_SIMULATOR_PREFLIGHT_PORTS`, for
   example `80,443` before installing a proxy. The script checks for 20 GiB
   free by default; set `PREFLIGHT_MIN_FREE_GB` to change that threshold.
5. Start it with `systemctl daemon-reload && systemctl enable --now
   network-simulator.service`.

The systemd unit keeps the server on `127.0.0.1:3080`, grants only
`CAP_NET_ADMIN` and `CAP_NET_RAW` to the service and its children for uBridge
and TAP operations, restricts writable host paths, and sends SIGINT on stop.
SIGINT enters the controller's graceful shutdown path; `KillMode=mixed` and
`TimeoutStopSec=60s` then make systemd remove any remaining children. The
runtime state directories are separate from shared QEMU and IOS image
directories. Docker containers are owned by Docker Engine and stop through the
controller's node lifecycle before the service exits.

## Reverse proxy and TLS

Choose one sample and replace `labs.example.edu` with the public DNS name.

- `deploy/Caddyfile` uses Caddy-managed TLS certificates and proxies HTTP and
  WebSocket upgrades to `127.0.0.1:3080`.
- `deploy/nginx.conf` expects certificates from a TLS issuer such as
  Certbot. Put its `map` directive in the `http` context and the `server`
  blocks in the appropriate include file.

Both examples forward the original host, scheme, and client address. WebSocket
console streams and project notifications use the same proxied route as HTTP.
The examples retain bearer-token authentication. Set
`NETWORK_SIMULATOR_AUTH_ENABLED=true` for a classroom deployment and leave
`NETWORK_SIMULATOR_TRUST_PROXY_HEADERS_UNAUTH` unset or false. With auth
enabled, `X-Forwarded-*` headers do not bypass bearer-token checks. If auth is
disabled on a loopback-only controller, forwarded headers trigger the
fail-closed rule and require auth unless the operator explicitly enables the
unsafe `NETWORK_SIMULATOR_TRUST_PROXY_HEADERS_UNAUTH` override. Do not use that
override for a shared classroom server.

The proxy only handles the controller UI/API and WebSockets. Runtime console
TCP ports and the UDP ports used by links are allocated by runtimes and are
not reverse-proxied. Keep the backend bound to loopback and firewall those
runtime ports from untrusted networks.

## Backup and restore

`deploy/backup-restore.sh` creates a tarball containing a transactionally
consistent SQLite backup made with `sqlite3 .backup`, plus QEMU images,
Dynamips images, and appliance definitions. It uses the same path environment
variables as the service. Example:

```sh
set -a
. /etc/network-simulator/network-simulator.env
set +a
sudo -E deploy/backup-restore.sh backup /srv/backups/lab-$(date +%F).tar.gz
sudo -E deploy/backup-restore.sh restore /srv/backups/lab-2026-10-19.tar.gz
```

Restore validates the archive paths and database integrity before replacing
the database and image directories. If the service is active, it stops the
unit for the restore and starts it again afterward. Take backups while the
controller is healthy and store them outside the server's image volume.
This does not back up Docker image layers or external libvirt/Docker state;
keep a record of the images required by your course and make those available
to the target host before starting imported labs.

## Environment variables

`deploy/network-simulator.env.example` is the service template. The variables
below are all `NETWORK_SIMULATOR_*` names referenced by `src/`; test-only
entries are commented out in the template.

| Variable | Meaning |
| --- | --- |
| `NETWORK_SIMULATOR_ALLOW_HOST_INTERFACES` | Comma-separated host interface allowlist for cloud nodes. |
| `NETWORK_SIMULATOR_APPLIANCES` | Directory for appliance definition JSON files; defaults below the application data directory. |
| `NETWORK_SIMULATOR_AUTH_ENABLED` | Explicit auth setting override. For a shared server, use `true`. |
| `NETWORK_SIMULATOR_BIND` | Controller listen address, default `127.0.0.1:3080`. |
| `NETWORK_SIMULATOR_CAPTURE_HOME` | Directory for link capture files. |
| `NETWORK_SIMULATOR_CPULIMIT` | Optional `cpulimit` executable used for Dynamips process throttling. |
| `NETWORK_SIMULATOR_DATA_HOME` | Application data directory for local binaries and default appliance/runtime data. |
| `NETWORK_SIMULATOR_DB` | SQLite database file path. |
| `NETWORK_SIMULATOR_DOCKER` | Docker CLI executable. |
| `NETWORK_SIMULATOR_DOCKER_STATE` | Docker runtime ownership and per-node persistent volume state. |
| `NETWORK_SIMULATOR_DYNAMIPS` | Dynamips executable. |
| `NETWORK_SIMULATOR_DYNAMIPS_IMAGES` | Shared IOS image directory. |
| `NETWORK_SIMULATOR_DYNAMIPS_STATE` | Dynamips hypervisor state directory. |
| `NETWORK_SIMULATOR_IDLE_STOP_MINUTES` | Optional inactivity limit; stop active nodes after this many idle minutes. Unset disables it. |
| `NETWORK_SIMULATOR_IMAGE_HOME` | Shared parent directory for default QEMU and Dynamips image directories. |
| `NETWORK_SIMULATOR_NAT_INTERFACE` | Host interface used by NAT nodes; defaults to `virbr0`. |
| `NETWORK_SIMULATOR_OVMF_DIR` | Optional directory containing OVMF firmware. |
| `NETWORK_SIMULATOR_QEMU` | QEMU system executable. Select the binary matching the guest platform. |
| `NETWORK_SIMULATOR_QEMU_CONFIG_DISK` | Optional raw config disk copied to nodes that request one. |
| `NETWORK_SIMULATOR_QEMU_IMAGES` | Shared QEMU disk image directory. |
| `NETWORK_SIMULATOR_QEMU_IMG` | `qemu-img` executable. |
| `NETWORK_SIMULATOR_QEMU_SOCKETS` | QMP and serial socket directory. |
| `NETWORK_SIMULATOR_QEMU_STATE` | QEMU per-node runtime state directory. |
| `NETWORK_SIMULATOR_SWTPM` | Optional `swtpm` executable for TPM-enabled guests. |
| `NETWORK_SIMULATOR_TRUST_PROXY_HEADERS_UNAUTH` | Unsafe override to trust forwarded headers while auth is otherwise disabled on loopback. Keep false/unset for classroom use. |
| `NETWORK_SIMULATOR_UBRIDGE` | uBridge executable. |
| `NETWORK_SIMULATOR_UBRIDGE_STATE` | uBridge socket and runtime state directory. |
| `NETWORK_SIMULATOR_VPCS` | VPCS executable. |
| `NETWORK_SIMULATOR_VPCS_STATE` | VPCS process and startup script state directory. |
| `NETWORK_SIMULATOR_X11VNC` | Optional `x11vnc` executable for Docker VNC consoles. |
| `NETWORK_SIMULATOR_XVFB` | Optional `Xvfb` executable for Docker VNC consoles. |
| `NETWORK_SIMULATOR_TEST_C3600_IMAGE` | Test-only Cisco 3600 IOS image path for the ignored Dynamips integration. |
| `NETWORK_SIMULATOR_TEST_DOCKER` | Test-only Docker CLI override for ignored runtime tests. |
| `NETWORK_SIMULATOR_TEST_DOCKER_IMAGE` | Test-only Docker image used by ignored integration tests. |
| `NETWORK_SIMULATOR_TEST_DYNAMIPS` | Test-only Dynamips executable for ignored integration tests. |
| `NETWORK_SIMULATOR_TEST_QEMU` | Test-only QEMU executable for ignored integration tests. |
| `NETWORK_SIMULATOR_TEST_QEMU_IMG` | Test-only `qemu-img` executable for ignored integration tests. |
| `NETWORK_SIMULATOR_TEST_QMP_PIDFILE` | Test fixture path used by the QMP subprocess test. |
| `NETWORK_SIMULATOR_TEST_QMP_SOCKET` | Test fixture socket path used by the QMP subprocess test. |
| `NETWORK_SIMULATOR_TEST_UBRIDGE` | Test-only uBridge executable for ignored link/runtime tests. |
| `NETWORK_SIMULATOR_TEST_VPCS` | Test-only VPCS executable for ignored integration tests. |

The preflight script also reads `PREFLIGHT_REQUIRE_KVM`,
`PREFLIGHT_MIN_FREE_GB`, and `NETWORK_SIMULATOR_PREFLIGHT_PORTS`; these are
preflight-only controls and are not read by the server.

## Idle stop

Set `NETWORK_SIMULATOR_IDLE_STOP_MINUTES` to a positive integer to stop running
nodes in a project after the API has seen no project request and no project
WebSocket has remained open for that duration. Any project API request resets
its idle timer. The default is disabled. This setting is server-wide; it does
not impose per-user quotas.
