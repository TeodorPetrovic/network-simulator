# Rust network simulator

This project is a Rust migration of a selected GNS3 server 3.1 `/v3` control surface, with a TanStack Start and TypeScript frontend. REA maps the supplied Web UI before implementing equivalent behavior; see [the reverse-engineering notes](docs/reverse-engineering.md).

## Current state

The controller persists projects, topology, and replayable events in SQLite. It exposes installed Docker images and local QEMU disk images as GNS3-style templates. Docker nodes run as real containers with start, stop, suspend, restart, and removal. QEMU nodes run as real VM processes with private copy-on-write disks and QMP start, stop, suspend, resume, and reset. Docker Ethernet links use isolated Engine bridges. QEMU Ethernet links connect exactly two VMs through QEMU's loopback datagram backend. Closing a project stops running nodes; deleting a node or project removes its runtime resources. The canvas edits nodes and links, applies live events, and renders stored SVG drawings through an SVG allowlist.

Docker and QEMU are the executable node runtimes in this increment. The fake Ethernet-switch node path has been removed: unsupported GNS3 node types return an API error instead of appearing as devices that cannot run. QEMU links are point-to-point and must be created while both VMs are stopped; Docker and QEMU nodes cannot be mixed on a link yet. Dynamips, VPCS, VMware, VirtualBox, IOU, appliance installation, console sessions, import/export, and full `/v3` coverage remain to be migrated.

The minimal stable Rust toolchain is installed in this environment.

## Run

Build the static frontend, then start the combined service:

```sh
npm ci --prefix frontend
npm run build --prefix frontend
cargo run
```

Open `http://127.0.0.1:3080`. TanStack Start builds a client-only SPA and static assets under `frontend/dist/client`; the Rust service serves those files, typed project routes, and the `/v3` API from the same origin. Node.js is needed to build the frontend but not to run it. During UI development, run `cargo run` and `npm run dev --prefix frontend` in separate terminals; Vite proxies `/v3` and `/healthz` to the Rust service. `npm run preview --prefix frontend` serves the static frontend build for UI-only checks.

The API listens on `127.0.0.1:3080` and stores state in `./data/network-simulator.sqlite` by default. Set `NETWORK_SIMULATOR_BIND` or `NETWORK_SIMULATOR_DB` to override those values. Docker node execution requires a local Docker Engine and CLI accessible to the service account. Set `NETWORK_SIMULATOR_DOCKER` if the CLI is not named `docker` or is outside `PATH`. Install an image on that host (for example, `docker pull alpine:latest`) before adding a Docker node.

QEMU execution requires `qemu-system-x86_64` and `qemu-img`; set `NETWORK_SIMULATOR_QEMU` and `NETWORK_SIMULATOR_QEMU_IMG` for custom executable paths. Put supported disk images (`qcow2`, `qcow`, `raw`, `img`, `vmdk`, or `vdi`) in `./images/qemu`, or set `NETWORK_SIMULATOR_QEMU_IMAGES`. QEMU node disks and logs go under `./data/qemu` by default, overridable with `NETWORK_SIMULATOR_QEMU_STATE`. Accessible `/dev/kvm` enables KVM acceleration; QEMU falls back to TCG when KVM is unavailable. The UI lists images found locally and does not download appliance images.

## Verify

```sh
npm test --prefix frontend
npm run typecheck --prefix frontend
npm run build --prefix frontend
cargo test
```

Implemented endpoints include `GET /v3/version`, `GET /v3/runtimes`, combined Docker/QEMU `GET /v3/templates`, project CRUD and open/close, Docker or QEMU node creation from `POST /v3/projects/{id}/templates/{template_id}`, node start/stop/suspend/reload, node/link/drawing CRUD, `GET /v3/projects/{id}/topology`, and `GET /v3/projects/{id}/notifications/ws`. The topology response includes an `event_cursor`; connect to the WebSocket with `?after=<event_cursor>` to replay changes since that snapshot. Link endpoints must currently be changed by deleting and recreating the link so runtime networks stay aligned with topology.

This is an early compatibility slice rather than the full GNS3 API. It does not yet authenticate clients, import/export `.gns3` files, configure Dynamips, VPCS, VMware, VirtualBox, or IOU, provide console sessions, or implement all server endpoints. The supplied Web UI source calls `/v2`; this frontend uses the server 3.1 `/v3` API.

## Direction under consideration

Keep topology and project control in Rust and use TanStack Start for the browser routes and static SPA build. The current SVG canvas handles the early topology editing flows without a graph framework dependency. Add Rust/WASM only when profiling identifies CPU-heavy browser work, such as layout of very large topologies; no such workload needs WASM in this slice. The supplied Web UI snapshot calls `/v2`, so matching it will require a separate compatibility shim.
