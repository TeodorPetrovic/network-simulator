# GNS3 → Rust/TanStack port: parity audit and work plan

Written 2026-10-18. Goal: a **one-to-one** port of GNS3 3.1 (server + Web UI) into this Rust controller and TanStack Start frontend. Improvements over GNS3 are fine, but missing GNS3 features and wrong behavior are defects.

**Trust level:** the audit below comes from reading the repo layout, route table, `HANDOFF.md`, and `docs/reverse-engineering.md`, and comparing them with the upstream trees. It is not a line-by-line proof. Every workstream must re-verify its own claims against code before acting, and fix this file where it is wrong.

## Upstream references (local, may vanish: `/tmp` is volatile)

| What | Path |
| --- | --- |
| GNS3 server 3.1 | `/tmp/gns3-server-3.1/gns3server` (also `/tmp/gns3-research/gns3-server`) |
| GNS3 Web UI | `/tmp/gns3-research/gns3-web-ui/src/app` (also `/tmp/gns3-web-ui-reference`) |
| VPCS source | `/tmp/GNS3-vpcs` |

If a path is gone, re-clone `GNS3/gns3-server` branch `3.1`, `GNS3/gns3-web-ui` and `GNS3/vpcs`. GPL-3.0 upstream: read for behavior, **do not copy code**; reimplement.

## How GNS3 devices communicate (the model we must match)

- Every topology link is a controller `Link` object (`controller/link.py`, `udp_link.py`). Creating a link allocates a UDP port pair on the source/destination computes, then calls `add_nio` on **each node's adapter/port**. Every runtime therefore plugs into the link the same way: through a NIO (UDP tunnel, or tap/ethernet/unix for cloud/NAT).
- **uBridge** is spawned per node (`compute/ubridge/`) and owns one bridge per adapter. It joins the VM/container side (QEMU via UDP/tap, Docker via veth/tap, VPCS and Dynamips via UDP NIO) to the link's UDP tunnel. It is also where **packet capture**, **link filters** (delay/jitter/loss/corrupt/BPF) and **suspend** (drop all frames) are implemented.
- Switching devices are first-class **nodes**, not link features: `ethernet_switch` (access/dot1q/qinq ports, VLAN), `ethernet_hub`, `cloud` (host interfaces, UDP/tap NIOs, remote console), `nat` (libvirt/virbr0), plus `frame_relay_switch` and `atm_switch`. All of them are Dynamips-backed or builtin and connect via NIOs exactly like VMs.
- A link has 2 endpoints (`node_id`, `adapter_number`, `port_number`, optional label), `suspend`, `filters`, `capturing`, `capture_file_name`, `link_style`, `nodes[]` and per-link capture endpoints.

## Initial parity gaps seen before the workstreams (historical snapshot)

The communication findings below describe the implementation before NET. The NET implementation note at the end of this audit is the current code-verified status and supersedes items 1, 2, 4, and 5.

1. **No uniform link engine.** `src/api.rs` (6k lines) picks a transport by *runtime-pair*: QEMU-only → multicast bus, VPCS/Dynamips-only → a hand-written Rust UDP frame switch (`vpcs.rs` `ensure_segment_switch`), anything involving Docker or mixed → one shared uBridge Linux bridge per link, QEMU via TAP, Docker via moved veth. That is three ad-hoc fabrics with special cases (`create_shared_ethernet_link`, `link_runtime_endpoints`, `attach_*_shared_links`, `cleanup_link_runtime`) instead of one NIO model. A "link" is really a *segment*; links with more than two endpoints/shared multi-node segments exist only because of this design and have no GNS3 equivalent.
2. **Links require stopped nodes** (`ensure_link_nodes_stopped`, `ensure_link_qemu_nodes_stopped`) and adapter count changes are blocked by existing links. GNS3 allows hot-adding links to running Docker/VPCS/Dynamips nodes (QEMU hot-add via ubridge NIO) and reconnecting on start/stop.
3. **No switch/hub/cloud/NAT nodes.** Without them a topology cannot have more than two devices on one broadcast domain except through the non-GNS3 multi-endpoint segment trick. This is the biggest functional gap.
4. **Link features:** suspend, filters, packet capture and stream, and Web Wireshark backend are implemented; browser-side Web Wireshark launch remains open.
5. **Node start/stop ordering** does not follow GNS3 (start ubridge → add NIOs → start VM; stop tears down NIOs). Links are attached from several places in `api.rs` rather than inside runtime adapters. Check restart-recovery of ubridge/veth/tap state.
6. **Dynamips**: only c7200 + `C7200-IO-FE`; GNS3 supports c1700/c2600/c2691/c3600/c3725/c3745/c7200 with per-slot WIC/NM/PA adapters, idle-PC, ghost ios, sparse mem, mac_addr, `iomem`, `disk0/1`, `auto_delete_disks`, etc. The `dynamips` binary talks through a hypervisor TCP protocol and each router port (slot/port) maps to a different NIO — port naming must follow `adapter_number` slots.
7. **Docker**: console `http`/`vnc`/`telnet`/`ssh` modes, aux console, custom adapters (`custom_adapters` with MAC), `start_command`, `environment`, `extra_volumes`, headless, resize. Currently main-TTY only.
8. **QEMU**: one HDA disk only; missing HDB/HDC/HDD, CD-ROM, kernel/initrd/append, BIOS/UEFI/TPM, first-boot options, cpu/mac, `options`, `process_priority`, `replicate_network_connection_state`, `on_close` behavior, console type vnc/spice/telnet/none, hot-add NIC, disk interfaces, and node snapshot-based "linked clones".
9. **VPCS**: one adapter, no suspend, startup script only on stopped node, console telnet-only OK.

## What is right (keep)

- SQLite WAL topology store with atomic snapshots + replayable event cursor; GNS3-shaped JSON for nodes/links/drawings; node CRUD cascade to links.
- Docker with `NetworkDisabled`, port N → ethN, privileged container, TTY console bridged to xterm.js.
- QEMU lifecycle via QMP, KVM with TCG fallback, per-node copy-on-write overlays, shared global image library, serial socket → WebSocket console.
- VPCS process management and Telnet-negotiated console; Dynamips hypervisor transport and IOS ELF validation.
- Appliance (`.gns3a`) registry import, image binding/checksum manager, template profiles.
- `/v3` route shape for projects/nodes/links/drawings/templates and project notification WebSocket.

## Server API gap (GNS3 3.1 controller has ≈210 routes, this repo ≈45)

Missing controller areas (see `gns3server/api/routes/controller/`): `controller.py` (version, iou_license, statistics, shutdown, `/notifications` stream, check version), `computes`, `projects` (export/import `.gns3project`, duplicate, lock, stats, load, `/files`, `/notifications`, `/locked`, nodes `/start`/`/stop`/`/suspend`/`/reload` bulk, `/nodes/reload`, `/nodes/isolate`/`unisolate`, `/nodes/console/reset`), `nodes` (duplicate, `/isolate`, `/unisolate`, `/console/reset`, `/files`, `/dynamips/auto_idlepc`, `/dynamips/idlepc_proposals`, `/qemu/disk_image` create/resize/delete, `/links` listing), `drawings`, `snapshots` (create/restore/delete/list), `symbols` (+ upload, raw, dimensions, default symbols/themes), `templates` (full CRUD + duplicate), `appliances` (list/install), `images` (list/upload/delete/prune per type), `settings`, `groups/users/roles/privileges/acl/pools/api_keys` (auth), `gns3vm`, `llm`/`copilot`/`chat`/`netmiko` (optional/low priority). Link creation, filters, capture, stream, and `available_filters` have since been implemented; full compute adapter/NIO routes and the browser Wireshark launch remain open.

Missing compute areas: ethernet_switch / hub / cloud / nat / frame_relay_switch / atm_switch, iou, virtualbox, vmware, and per-runtime `adapters/{n}/ports/{m}/nio`, `capture`, `/console/ws`, `reload`, `suspend`, `duplicate` endpoints.

## Web UI gap

Existing: `/` projects list, `/projects/$projectId` workspace (map, palette, summary, properties, console dock, template settings dialog, marketplace), `/appliances`.

GNS3 Web UI (see `gns3-web-ui/src/app/components`): `projects` (list/search/import/export/duplicate/delete/clone, project settings/readme/locked, project snapshots via `snapshots/`), `project-map` (context menu with start/stop/suspend/reload/console/aux console/duplicate/isolate/lock/rename/symbol/config/delete, **draw-link tool**, **node-select-interface**, drawings toolbar+editors (rect/ellipse/line/text/image), link context menus + link style + filters + packet capture + Wireshark, nodes menu, bulk start/stop/suspend/reload/console, screenshot, help, info dialog, log console, topology summary with filter, grid/snap, lock project, zoom, select-box multi-select, undo for move, project-map-menu, change hostname/symbol, packet-capture viewer), `preferences` (docker/dynamips/ios-on-unix/qemu/virtualbox/vmware/vpcs/traceng/built-in nodes/general/gns3vm templates CRUD with per-runtime config forms and tabs), `template` (new-template wizard), `servers`, `settings`, `system-status`, `installed-software`, `web-console-full-window`, `snapshots`, `direct-link`, `help`, `adbutler` (skip), and the node-editors for every runtime including ethernet_switch/hub/cloud/nat.

This repo implements a fraction of the map interactions and none of the preferences, snapshot, symbol, import/export or server/settings pages. Link features have controller/API support, but the full GNS3 link context menus, capture controls, and Wireshark launch are not yet ported in the browser.

## Work plan (parallel workstreams)

Each workstream is delegated to a Codex Luna xhigh agent. The checkout is shared, so owners are defined below. **Do not revert or reformat files you do not own; keep the tree compiling (`cargo check --locked`, `npm run typecheck --prefix frontend`); run only the tests relevant to your area.** Put new routes in new modules and add a one-line `.merge(...)` in `router()`; touch `src/api.rs` surgically.

| ID | Owner area | Files owned | Goal |
| --- | --- | --- | --- |
| NET | link/communication core | new `src/links/**`, `src/ubridge.rs`; link functions in `api.rs` | Replace the per-runtime-pair fabrics with one NIO/uBridge link engine as in GNS3; hot-add on running nodes; suspend, filters, capture; proper start/stop sequencing and restart recovery. Live-prove Docker↔Docker, Docker↔VPCS, QEMU↔Docker, QEMU↔VPCS, VPCS↔VPCS, Dynamips↔VPCS (if image). |
| BUILTIN | builtin nodes | new `src/builtin/**` | Ethernet switch (VLAN access/dot1q/qinq), hub, cloud, NAT; template + node + UI palette entries. **Starts after NET lands the engine API.** |
| API | controller API parity | new route modules | All missing controller endpoints above except auth/LLM, backed by SQLite migrations: snapshots, `.gns3project` import/export, symbols, templates CRUD, images, settings, computes, controller stats, node isolate/duplicate/console reset/files, project lock/stats/duplicate. |
| MAP | project-map UI | `frontend/src/SimulatorWorkspace.tsx` + new map components | Port project-map: link tool, interface picker, drawings editor, link/node context menus, bulk actions, symbols, lock, snapshots dialog, packet-capture UI, log console, screenshot, multi-select. |
| PAGES | other UI pages | new route files, `ApplianceManagerPage.tsx`, `TemplateSettingsDialog.tsx` | Projects page parity (import/export/duplicate/search), Preferences per-runtime template CRUD forms, general settings, system status, new-template wizard, servers page. |
| RUNTIME | per-runtime fidelity | `qemu.rs`, `dynamips.rs`, `runtime.rs`, `vpcs.rs` (not NIC attach code) | Close the QEMU/Docker/Dynamips/VPCS option gaps listed above, with launch-path tests. |

Auth, GNS3 VM, LLM/copilot, VirtualBox, VMware and IOU are explicit later phases.

## Definition of done per feature

1. Behavior matches the cited upstream file/function (cite it in the commit/PR text).
2. Rust test or live integration test (ignored-by-default is fine for needing root/images) and frontend test where UI.
3. `cargo fmt`, `cargo test --locked`, `npm run typecheck --prefix frontend`, `npm test --prefix frontend -- --run` pass.
4. `HANDOFF.md` updated with what changed and what remains.

## NET implementation note — 2026-10-18

Verified against the requested GNS3 3.1 sources in `/tmp/gns3-server-3.1/gns3server` (`controller/link.py`, `controller/udp_link.py`, `compute/base_node.py`, `compute/ubridge/*`, and the Docker, QEMU, VPCS, and Dynamips NIO implementations): a controller Ethernet link has exactly two endpoints. `LinkCreate.nodes` is constrained to two entries and `LinkUpdate.nodes` cannot exceed two; links are point-to-point NIO tunnels. Multi-endpoint segments are an artifact of this port's earlier shared-bridge/Rust-switch implementation and are rejected. A switch or hub must be represented as a node before that topology is supported.

The former parity statement that links are transport-specific segments is now closed. `src/links/mod.rs` owns the point-to-point link engine and a runtime-neutral adapter-NIO description implemented for Docker, QEMU, VPCS, and Dynamips. Each link endpoint has a stable runtime endpoint ID and two reciprocal UDP tunnel ports. The shared local uBridge control process owns an isolated named bridge for each link endpoint; upstream runs node-scoped uBridge processes, so process scope differs while adapter-to-tunnel behavior stays the same. Docker uses an Ethernet NIO on a per-port veth; QEMU uses a QMP socket netdev; VPCS uses its local `-s`/`-c` UDP interface; Dynamips uses a hypervisor UDP NIO. No link pair chooses multicast, a Rust frame switch, TAP, or a shared Linux bridge. The only remaining Linux veth is Docker's guest adapter.

Link creation now permits active endpoints and attaches them immediately. Node start reattaches saved endpoints; stop detaches NIOs and relays before stopping; reload paths that recreate a runtime reattach afterward. Controller startup scans persisted links and reattaches endpoints whose runtimes survived (notably Docker containers). Stopped runtimes attach when they are next started. Adapter indices and UDP assignments are stored on each endpoint so controller restart does not change the topology wiring; the recovery code is implemented but controller-process restart was not exercised live. QEMU hot-add uses QMP `netdev_add`/`device_add`; the installed CHR guest did not expose NICs added after full boot, so QEMU packet checks created links before node start. VPCS ports are reserved before launch; Dynamips binds before IOS starts.

The controller persists `suspend`, filter values, `link_style`, and endpoint labels. The relay applies `frequency_drop`, `packet_loss`, `delay` (latency plus optional jitter and distribution), `corrupt`, and BPF filters; suspend is represented by the upstream drop-all `frequency_drop -1` rule. `available_filters`, capture start/stop, capture file, and active capture stream routes are provided. Capture stream reads the growing PCAP until capture is stopped. The stream and file responses use `application/vnd.tcpdump.pcap`; the Web Wireshark backend lazily creates one Docker container per project and serves xpra through a binary WebSocket proxy. Persisted filter/suspend values have API test coverage, but non-empty filters and packet capture were not exercised live. BPF text is passed to uBridge for compilation and is not syntax-checked by the controller before attachment.

`UbridgeRuntime::info` probes creation and removal of a uBridge userspace relay. The earlier bridge/TAP helpers used to exercise shared Linux segments have been removed; they are not a fallback link transport. Existing Docker per-node networking remains disabled with `--network none`.

## Dynamips/IOS live verification note — 2026-10-18

The historical Dynamips gap above is closed for the runtime path and live-tested platforms. The adapter supports the GNS3 registry families c1700, c2600, c2691, c3600, c3725, c3745, and c7200. The image inventory, exact registry filenames/versions/MD5s, LabHub offers, downloads, and full verification results are in [docs/ios-images.md](ios-images.md). IOS images are kept in the global image library and ignored by Git.

The opt-in API lab passed with c3620, c3725, and c7200 images under upstream Dynamips 0.2.25: all three reached IOS CLI prompts; c3725↔c7200 pinged 5/5; c3620↔VPCS ping passed; and c3600 + VPCS + Alpine Docker exchanged traffic through an Ethernet switch. Reload, suspend/resume, running link deletion/recreation, auto-selected Idle-PC persistence and restart application, graceful shutdown, and cleanup passed. The installed 0.2.14 binary satisfies the minimum version but crashes in glibc `vscanf` when launching IOS; 0.2.25 was used as an environment override and was not installed over it. The latest Idle-PC `0x60c086a8` run measured 40.8% process CPU before restart and 36.4% after a 5-second settle plus 8-second sample; the later process check was 49.0%. An earlier run measured 32.8% / 39.8% with a 49.6% final average. Two earlier 2.5-second post-restart samples read 52.0% and 54.8% and failed the strict 50% check before later topology tests. The final cleanup left no test hypervisors, uBridge processes, transient files, or node runtime directories. Remaining work is to use a compatible default binary and live-test the other platforms and Dynamips slots.
