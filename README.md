# Rust network simulator

This project is a Rust migration of a selected GNS3 server 3.1 `/v3` control surface, with a TanStack Start and TypeScript frontend. REA maps the supplied Web UI before implementing equivalent behavior; see [the reverse-engineering notes](docs/reverse-engineering.md).

## Current state

The controller persists projects, topology, and replayable events in SQLite. It exposes installed Docker images, local QEMU disk images, the installed VPCS executable, and eligible Cisco IOS images as GNS3-style templates. Docker nodes run as interactive TTY containers; QEMU nodes run as real VMs with private copy-on-write disks and QMP lifecycle control; VPCS nodes run the upstream simulator process; classic IOS c7200 routers run through the Dynamips hypervisor. Same-runtime Docker, QEMU, and VPCS/Dynamips links use their native Ethernet transports. Links that cross runtimes use one uBridge Linux bridge with TAP, container veth, and UDP-to-TAP endpoints, so packets share a real Ethernet segment across the selected node types. Closing a project stops running nodes; deleting a node or project removes its runtime resources. The canvas edits nodes and links, applies live events, and renders stored SVG drawings through an SVG allowlist.

Docker, QEMU, VPCS, and Dynamips are executable node runtimes. A link between different runtime types is built as a real shared Linux bridge: QEMU attaches a TAP, Docker joins through a veth moved into its network namespace, and VPCS/Dynamips use uBridge UDP-to-TAP bridges. The bridge is brought up when created, and attached nodes must be stopped while the link is created. Docker guests receive `NET_ADMIN` and `NET_RAW` so they can configure interfaces and send packets. This host's uBridge has `CAP_NET_ADMIN` and `CAP_NET_RAW`; `/v3/runtimes` reports it available, and a live controller API test verified Alpine Docker ↔ VPCS ICMP with no packet loss. QEMU/VPCS and licensed Dynamips mixed-runtime packet tests remain. Same-runtime links retain their tested backends. The current Dynamips template is a c7200 with one FastEthernet interface. VPCS currently has one Ethernet adapter and does not support process suspension. Docker console sessions require the `script` utility (provided by util-linux on Ubuntu); terminal resizing and other console modes remain to be migrated. VMware, VirtualBox, IOU, `.gns3` project import/export, and full `/v3` coverage remain.

Images and appliance definitions are global to the controller and shared by every project. On Linux the default image root is `$XDG_DATA_HOME/network-simulator/images`, or `~/.local/share/network-simulator/images` when `XDG_DATA_HOME` is unset. Startup creates `qemu/` and `dynamips/` folders there. Set `NETWORK_SIMULATOR_IMAGE_HOME` to move that root, or set `NETWORK_SIMULATOR_QEMU_IMAGES` and `NETWORK_SIMULATOR_DYNAMIPS_IMAGES` independently. QEMU and Dynamips nodes use those shared base images; QEMU gives each node a private copy-on-write disk under its runtime state directory. Docker images stay in the Docker Engine's global image store, and VPCS is an installed executable rather than a disk image. No vendor images are bundled.

The image catalog searches the official GNS3 appliance registry and iShare2/LabHub. GNS3 appliance definitions are `.gns3a` files; imported definitions are stored under `$XDG_DATA_HOME/network-simulator/appliances` by default (override with `NETWORK_SIMULATOR_APPLIANCES`) and make compatible templates from images that are already installed. iShare2 QEMU disk files and single-file Dynamips IOS images can be downloaded from the catalog into the global runtime directories. The controller verifies the catalog's file size and MD5/SHA1 checksums; Dynamips files also pass the local ELF check. This does not invoke iShare2's root-only EVE-NG installer. IOL images and QEMU archive packages are not supported by the downloader. Review image licensing before starting a download.

Classic Cisco IOS images run under Dynamips; Cisco IOSv and IOSvL2 appliances run under QEMU. The current Dynamips implementation supports the c7200 platform and requires a user-supplied, licensed, 32-bit big-endian ELF IOS image. Hypervisor control, router configuration, console access, image upload, and UDP Ethernet NIO setup are implemented. A real IOS router boot and router-to-VPCS ping still need to be verified with an operator-provided image; the repository contains no Cisco image.

The minimal stable Rust toolchain is installed in this environment.

## Run

Build the static frontend, then start the combined service:

```sh
npm ci --prefix frontend
npm run build --prefix frontend
cargo run
```

Open `http://127.0.0.1:3080`. TanStack Start builds a client-only SPA and static assets under `frontend/dist/client`; the Rust service serves those files, typed project routes, and the `/v3` API from the same origin. Node.js is needed to build the frontend but not to run it. During UI development, run `cargo run` and `npm run dev --prefix frontend` in separate terminals; Vite proxies `/v3` and `/healthz` to the Rust service. `npm run preview --prefix frontend` serves the static frontend build for UI-only checks.

The API listens on `127.0.0.1:3080` and stores state in `./data/network-simulator.sqlite` by default. Set `NETWORK_SIMULATOR_BIND` or `NETWORK_SIMULATOR_DB` to override those values. For remote browser use, forward only TCP 3080: an SSH tunnel (`ssh -L 3080:127.0.0.1:3080 user@server`) keeps the service loopback-only; direct router forwarding requires binding `NETWORK_SIMULATOR_BIND=0.0.0.0:3080` and forwarding TCP 3080 to the host. The controller has no authentication yet, so keep direct exposure on a trusted network. API, UI, and console WebSockets all use this same port. Docker node execution requires a local Docker Engine and CLI accessible to the service account. Set `NETWORK_SIMULATOR_DOCKER` if the CLI is not named `docker` or is outside `PATH`. Install an image on that host (for example, `docker pull alpine:latest`) before adding a Docker node.

Guest-to-guest traffic stays on the simulator host; no extra router-forwarded UDP ports are needed. uBridge's UDP NIO ports are loopback-only and are internal adapters between emulators and the shared bridge.

QEMU execution requires `qemu-system-x86_64` and `qemu-img`; set `NETWORK_SIMULATOR_QEMU` and `NETWORK_SIMULATOR_QEMU_IMG` for custom executable paths. Put supported disk images (`qcow2`, `qcow`, `raw`, `img`, `vmdk`, or `vdi`) in the global QEMU image folder shown in the UI, or upload one in Add node. Uploads are validated with `qemu-img` and limited to 64 GiB. QEMU node disks and logs go under `./data/qemu` by default, overridable with `NETWORK_SIMULATOR_QEMU_STATE`; these are per-node runtime files, while source images remain global. The service checks whether its own account can read and write `/dev/kvm`, reports the reason if access is denied, and uses TCG until KVM is accessible. Marketplace import adds the `.gns3a` definition, not its VM disk files.

VPCS execution uses the upstream `vpcs` executable from `PATH`, `NETWORK_SIMULATOR_VPCS`, or `$XDG_DATA_HOME/network-simulator/bin/vpcs` (falling back to `~/.local/share/network-simulator/bin/vpcs`). On Ubuntu, install the distribution package or build the BSD-licensed source with `make -C <vpcs-source>` and place it in that user-local bin directory. Node state and logs go under `./data/vpcs`, overridable with `NETWORK_SIMULATOR_VPCS_STATE`. The executable is checked at runtime; the VPCS template only appears when the version check succeeds.

Dynamips execution uses the upstream `dynamips` executable from `PATH`, `NETWORK_SIMULATOR_DYNAMIPS`, or `$XDG_DATA_HOME/network-simulator/bin/dynamips` (falling back to `~/.local/share/network-simulator/bin/dynamips`). The implementation requires Dynamips 0.2.11 or newer. Upload a compatible licensed IOS image from the UI or place it under the global Dynamips image folder shown in the UI; set `NETWORK_SIMULATOR_DYNAMIPS_IMAGES` to use another image directory. Uploaded images are checked for the Dynamips-supported ELF header and limited to 512 MiB. Hypervisor state and logs go under `./data/dynamips`, overridable with `NETWORK_SIMULATOR_DYNAMIPS_STATE`.

Mixed-runtime links require the upstream [GNS3 uBridge](https://github.com/GNS3/ubridge) executable. Set `NETWORK_SIMULATOR_UBRIDGE` to its path if it is not on `PATH`. On Ubuntu, build it from the upstream source and run `sudo make install`; upstream installation sets the required `cap_net_admin,cap_net_raw` file capabilities. The controller prefers an executable found on `PATH` (such as the root-owned system install) over its user-local fallback. Confirm the installed file's capabilities with `getcap "$(command -v ubridge)"`. The runtime panel checks whether uBridge can create and remove a Linux bridge, rather than only checking whether its executable exists.

## Verify

```sh
npm test --prefix frontend
npm run typecheck --prefix frontend
npm run build --prefix frontend
cargo test
```

The live VPCS, Dynamips, Docker, and uBridge checks are ignored by default. VPCS tests build real upstream VPCS processes and verify ICMP across the Rust UDP segment. The Dynamips test starts the real hypervisor, creates a c7200 through its TCP command protocol, and configures its UDP Ethernet NIO; it does not boot IOS because that test requires an operator-provided image. The mixed-runtime tests use real Alpine and VPCS processes; one verifies the uBridge data path directly and one creates, starts, links, and cleans up the nodes through the controller API before checking ICMP. They require Docker, the local Alpine image, VPCS, and uBridge with the capabilities above. Run the available live checks with:

```sh
NETWORK_SIMULATOR_TEST_VPCS=/path/to/vpcs cargo test vpcs_ -- --ignored --nocapture
NETWORK_SIMULATOR_TEST_DYNAMIPS=/path/to/dynamips cargo test dynamips::tests::upstream_dynamips_creates_and_reports_a_real_c7200_hypervisor_instance -- --ignored --nocapture
cargo test runtime::tests::alpine_containers_exchange_packets_over_a_real_link_network -- --ignored --nocapture
NETWORK_SIMULATOR_TEST_UBRIDGE=/path/to/ubridge NETWORK_SIMULATOR_TEST_VPCS=/path/to/vpcs cargo test ubridge::tests::alpine_and_vpcs_exchange_icmp_over_a_real_ubridge_segment -- --ignored --exact --nocapture
NETWORK_SIMULATOR_UBRIDGE=/path/to/ubridge NETWORK_SIMULATOR_TEST_VPCS=/path/to/vpcs cargo test api::tests::alpine_and_vpcs_exchange_icmp_over_a_real_mixed_link_through_the_controller -- --ignored --exact --nocapture
```

Implemented endpoints include `GET /v3/version`, `GET /v3/runtimes`, combined Docker/QEMU/VPCS/Dynamips `GET /v3/templates`, image upload for QEMU and Dynamips, `GET /v3/marketplace/ishare2/images`, `POST /v3/marketplace/ishare2/images/{image_type}/{image_id}/download`, GNS3 registry search/detail/import under `/v3/marketplace/gns3/appliances`, project CRUD and open/close, template-backed node creation, node lifecycle, node/link/drawing CRUD, `GET /v3/projects/{id}/topology`, project event replay at `/v3/projects/{id}/notifications/ws`, QEMU serial streaming, Docker TTY streaming, and VPCS/Dynamips console streaming at their runtime-specific `/v3/compute/.../console/ws` paths. The topology response includes an `event_cursor`; connect to the event WebSocket with `?after=<event_cursor>` to replay changes since that snapshot. Link endpoints must currently be changed by deleting and recreating the link so runtime networks stay aligned with topology.

This is an early compatibility slice rather than the full GNS3 API. It does not yet authenticate clients, import/export `.gns3` files, support Dynamips platforms other than c7200, VMware, VirtualBox, or IOU, support VNC console modes or terminal resizing, or implement all server endpoints. The supplied Web UI source calls `/v2`; this frontend uses the server 3.1 `/v3` API.

## Direction under consideration

Keep topology and project control in Rust and use TanStack Start for the browser routes and static SPA build. The current SVG canvas handles the early topology editing flows without a graph framework dependency. Add Rust/WASM only when profiling identifies CPU-heavy browser work, such as layout of very large topologies; no such workload needs WASM in this slice. The supplied Web UI snapshot calls `/v2`, so matching it will require a separate compatibility shim.
