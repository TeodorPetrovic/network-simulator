# Development handoff

Updated: 2026-10-15

## Project objective

Build a reliable Rust network-lab controller by reverse engineering the GNS3 3.1 server and Web UI. The evidence and scope decisions are in [docs/reverse-engineering.md](docs/reverse-engineering.md); the compact REA report is in [docs/evidence/gns3-web-ui-rea-summary.json](docs/evidence/gns3-web-ui-rea-summary.json).

## Current implementation

- The Axum controller persists projects and topology in SQLite. It exposes a selected GNS3-shaped `/v3` API, atomic topology snapshots, and replayable project events over WebSockets.
- Docker nodes use real local images and containers, lifecycle operations, and isolated bridge networks for links.
- QEMU nodes use local disk images, private copy-on-write disks, QMP lifecycle control, and KVM with TCG fallback. Current QEMU links join two stopped VMs point-to-point.
- The browser UI uses TanStack Start in SPA mode with typed project routes. React remains the component library. The Rust server serves the built static shell and assets alongside the API; Node is needed to build and develop, not to run the deployed controller.
- Previous `#/projects/<id>` URLs redirect to `/projects/<id>`. The frontend uses the target server's `/v3` API; the supplied upstream UI snapshot calls `/v2`.
- Unsupported node types are rejected. There are no fake runtime implementations for unported hypervisors.

## Build and verification

From the repository root:

```sh
npm ci --prefix frontend
npm run build --prefix frontend
npm test --prefix frontend -- --run
npm run typecheck --prefix frontend
cargo fmt --all -- --check
cargo build --locked
cargo test --locked
```

Last verified on 2026-10-14: frontend build/typecheck passed, 16 frontend tests passed, Rust format/build passed, and 12 Rust tests passed. HTTP checks returned the SPA shell for `/` and `/projects/<id>` while unknown `/v3` routes remained 404.

Start the combined service with `cargo run` after building the frontend. It listens on `127.0.0.1:3080` by default. For UI development, run `cargo run` and `npm run dev --prefix frontend`; the Vite server proxies API and WebSocket calls to the Rust service.

## Next development work

1. Expand GNS3 compatibility from observed source/API evidence and captured fixtures. Keep `/v2` compatibility decisions explicit rather than assuming the upstream UI and 3.1 server are aligned.
2. Add another real node adapter, likely VPCS or Dynamips, with executable detection, templates, lifecycle, links, cleanup, and tests. Do not surface a node type until its runtime works end to end.
3. Replace QEMU's point-to-point link topology with a shared Ethernet segment model so multiple VMs can share a network.
4. Continue migrating consoles, drawings editing, project import/export, and remaining controller endpoints against upstream behavior.
5. Run adapter integration tests on hosts with Docker and QEMU installed; this environment's earlier check lacked QEMU binaries and `/dev/kvm`.

## Known scope limits

Authentication, VPCS, Dynamips, VMware, VirtualBox, IOU, appliance installation, console sessions, `.gns3` import/export, and full `/v3` coverage are not implemented yet. See the README roadmap before taking on a new adapter or API area.
