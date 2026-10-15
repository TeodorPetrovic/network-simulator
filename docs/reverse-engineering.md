# Reverse-engineering notes

## Source snapshots

Inspected these upstream snapshots on 2026-10-14:

| Source | Revision | License observed |
| --- | --- | --- |
| [REA](https://github.com/morluto/rea) | `e79e014f23cf5b5d59aa5e5681897dbcad73b44e` | MIT |
| [GNS3 Web UI](https://github.com/GNS3/gns3-web-ui) | `87fe78074efa631c72fbf8f29907198f3b803aa4` | GPL-3.0 |
| [GNS3 server, branch 3.1](https://github.com/GNS3/gns3-server/tree/3.1) | `149549e02986aa76d806b92b04cb36ad91786223` | GPL-3.0 |

The GNS3 server describes itself as the manager for emulator and virtualization processes, controlled by clients over an HTTP REST API. The Web UI README says its development server is separate and directs users to the UI bundled with GNS3 for normal use. See the [server README](https://github.com/GNS3/gns3-server/blob/149549e02986aa76d806b92b04cb36ad91786223/README.md) and [Web UI README](https://github.com/GNS3/gns3-web-ui/blob/87fe78074efa631c72fbf8f29907198f3b803aa4/README.md).

## REA analysis

REA's `analyze-javascript-application` command was run against a focused six-file slice of the Web UI: the project map, D3 map, project/node/notification services, and project event handler. The resulting artifact SHA-256 is `7a45dab7967f485fc2d45798295b4cbf996ad37224f8a9555d0ad3c14ddbd4f7`; its REA Evidence ID is `ev_1dbf7e815ee577354e04bdbc656f1fc6042a73f5847c5ca7feac1ecba823ba42`.

REA read 63,835 bytes across six files, parsed 9,656 AST nodes, reported 301 findings, and reported no parse failures. Its summary marked the application inventory complete for this selected slice. Semantic coverage remained `unknown`; there were no recovered module records. REA parses this source as inert text and does not establish runtime behavior. I therefore treat the static analysis as a navigation aid and use the checked-in source and server API implementation as the primary evidence for behavior. The full Evidence remains in `/tmp/gns3-ui-rea-evidence.json`; a compact summary is retained at [`docs/evidence/gns3-web-ui-rea-summary.json`](evidence/gns3-web-ui-rea-summary.json).

This follows REA's own distinction between static relationships and runtime execution. See [REA's application-analysis guide](https://github.com/morluto/rea/blob/e79e014f23cf5b5d59aa5e5681897dbcad73b44e/docs/javascript-application-workflows.md) and [CLI Evidence guide](https://github.com/morluto/rea/blob/e79e014f23cf5b5d59aa5e/docs/cli.md).

## Observed behavior

### API version alignment

The GNS3 server 3.1 snapshot registers its controller routes under `/v3` in [`api/server.py`](https://github.com/GNS3/gns3-server/blob/149549e02986aa76d806b92b04cb36ad91786223/gns3server/api/server.py#L108-L116), and its OpenAPI document lists controller routes under `/v3`. The supplied Web UI `master` snapshot prepends `/v2` to HTTP requests in [`http-server.service.ts`](https://github.com/GNS3/gns3-web-ui/blob/87fe78074efa631c72fbf8f29907198f3b803aa4/src/app/services/http-server.service.ts#L174-L194) and uses `/v2` WebSocket URLs in [`notification.service.ts`](https://github.com/GNS3/gns3-web-ui/blob/87fe78074efa631c72fbf8f29907198f3b803aa4/src/app/services/notification.service.ts#L7-L23).

These two branch snapshots do not line up for direct client/server use. The first API increment targets the server 3.1 `/v3` contract. A `/v2` compatibility shim for this Web UI snapshot is a separate follow-up, after the `/v3` behavior is in place.

### Project and topology data

- The Web UI requests project nodes, then links, then drawings when loading a project. It installs the project WebSocket after those requests finish. See [`project-map.component.ts`](https://github.com/GNS3/gns3-web-ui/blob/87fe78074efa631c72fbf8f29907198f3b803aa4/src/app/components/project-map/project-map.component.ts#L440-L477) and [`project.service.ts`](https://github.com/GNS3/gns3-web-ui/blob/87fe78074efa631c72fbf8f29907198f3b803aa4/src/app/services/project.service.ts#L26-L56).
- Project notification frames carry an `action` and an `event`; the UI applies `node.*`, `link.*`, and `drawing.*` actions to in-memory data sources. See [`project-web-service-handler.ts`](https://github.com/GNS3/gns3-web-ui/blob/87fe78074efa631c72fbf8f29907198f3b803aa4/src/app/handlers/project-web-service-handler.ts#L9-L75).
- The server serializes project metadata and topology collections (`nodes`, `links`, `computes`, `drawings`) into its `.gns3` JSON format with a format revision. See [`topology.py`](https://github.com/GNS3/gns3-server/blob/149549e02986aa76d806b92b04cb36ad91786223/gns3server/controller/topology.py#L73-L128).
- Node deletion removes attached links before destroying the node, then saves the topology and emits a deletion notification. Node creation is idempotent for an existing node ID and persists/emits after creation. See [`project.py`](https://github.com/GNS3/gns3-server/blob/149549e02986aa76d806b92b04cb36ad91786223/gns3server/controller/project.py#L666-L750).
- The server writes a temporary topology file and moves it into place after JSON serialization. Starting all nodes uses a bounded concurrency pool. See [`project.py`](https://github.com/GNS3/gns3-server/blob/149549e02986aa76d806b92b04cb36ad91786223/gns3server/controller/project.py#L2270-L2297).
- The server exposes a project notification WebSocket; clients receive one JSON notification per frame. See [`projects.py`](https://github.com/GNS3/gns3-server/blob/149549e02986aa76d806b92b04cb36ad91786223/gns3server/api/routes/controller/projects.py#L640-L670).

### Runtime templates and node lifecycle

- The server lists templates through `GET /v3/templates` and creates a project node from a template through `POST /v3/projects/{project_id}/templates/{template_id}` with a `TemplateUsage` body (`x`, `y`, optional name and compute ID). See [`templates.py`](https://github.com/GNS3/gns3-server/blob/149549e02986aa76d806b92b04cb36ad91786223/gns3server/api/routes/controller/templates.py#L187-L235) and [`projects.py`](https://github.com/GNS3/gns3-server/blob/149549e02986aa76d806b92b04cb36ad91786223/gns3server/api/routes/controller/projects.py#L1017-L1040).
- Node power operations are separate `start`, `stop`, `suspend`, and `reload` routes. The controller waits for a target state only when the caller asks it to; unsupported suspend is an error. See [`nodes.py`](https://github.com/GNS3/gns3-server/blob/149549e02986aa76d806b92b04cb36ad91786223/gns3server/api/routes/controller/nodes.py#L449-L525).
- Docker nodes use an image and adapter count, then the compute side owns actual container lifecycle and networking. QEMU templates instead refer to a disk image, memory, CPU count, and adapter configuration; QEMU process startup and QMP control live in the compute implementation. See the pinned [`qemu_vm.py` lifecycle](https://github.com/GNS3/gns3-server/blob/149549e02986aa76d806b92b04cb36ad91786223/gns3server/compute/qemu/qemu_vm.py#L1094-L1170) and [network argument construction](https://github.com/GNS3/gns3-server/blob/149549e02986aa76d806b92b04cb36ad91786223/gns3server/compute/qemu/qemu_vm.py#L2590-L2640). This migration now implements real Docker containers and real QEMU processes; its QEMU network path is currently limited to direct two-VM Ethernet links.

### Engineering implications

**Observation:** the UI installs its event stream after separately fetching nodes, links, and drawings. **Inference:** a concurrent edit between the snapshot reads and WebSocket subscription could leave a client with stale state unless another refresh occurs. The source does not prove this race occurs in normal use. The Rust design should make initial snapshots and event cursors consistent, then support replay from a durable sequence number.

**Observation:** each ordinary topology mutation can serialize and replace the full `.gns3` file. **Inference:** frequent edits in large projects may do avoidable disk work. A transactional store can commit the topology change and event record together, then checkpoint snapshots asynchronously. The existing temporary-file replacement is a useful crash-safety behavior to preserve for exports/backups.

**Observation:** the server delegates device work to several external runtimes (QEMU/KVM, Docker, VPCS, Dynamips, and others). A faithful replacement therefore has a much larger scope than reimplementing the graph model. The Rust controller currently uses separate Docker and QEMU runtime services; a shared capability/lifecycle interface should follow before adding more adapters.

## Architecture and next boundaries

The original architecture proposal is partially implemented. Current code keeps runtime process details out of the topology store and persists the metadata needed to reattach to each VM/container after restart. The next boundaries to extract are:

1. **Domain core:** projects, nodes, interfaces, links, runtime state transitions, validation, and typed errors. No HTTP or database types in the core.
2. **Persistence:** SQLite with WAL and migrations for project/topology state, a monotonically increasing per-project revision, and an outbox/event sequence committed in the same transaction as each mutation.
3. **API and event transport:** async HTTP plus WebSocket or SSE. Mutations support idempotency keys; clients can resume from an event cursor. A snapshot includes the cursor it represents.
4. **Runtime adapters:** async lifecycle interfaces for supported emulator/container types, with explicit timeouts, cancellation, health checks, and cleanup on partial failure.
5. **UI:** use TanStack Start with React and TypeScript for a same-origin static SPA; retain a Rust/WASM module only for measured CPU-heavy browser work.

The main performance strategy is bounded parallelism for independent node lifecycle work, indexed topology lookups, batched persistence, and avoiding full topology serialization on every UI interaction. The main reliability strategy is transactionally coupled state/events, replayable events, validated graph mutations, bounded external process calls, and recovery from partially completed adapter operations.

## Implemented first slice

The initial Rust service binds to loopback by default and exposes a selected `/v3` controller surface:

- Project list/create/read/update/delete and open/close.
- Node, link, and drawing list/create/read/update/delete.
- Link endpoint validation and automatic removal of attached links when deleting a node.
- `GET /v3/projects/{id}/topology` reads the project, topology collections, and event cursor in one SQLite read transaction.
- Project WebSocket notifications persist to an event table in the same write transaction as the topology change. Clients can resume with `?after=<event_cursor>`; broadcast lag falls back to replay from SQLite.

SQLite uses WAL with full synchronous commits. Topology objects retain their GNS3-shaped JSON fields, including fields the controller does not interpret yet. This makes the early API extensible while keeping common lookups indexed. The topology subset now has two executable local runtimes:

- **Docker:** installed-image templates, stopped-container creation, optional per-node command arguments, lifecycle operations, isolated internal bridge networks for Ethernet links, runtime state refresh, and cleanup.
- **QEMU:** disk-image templates under a configurable directory, `qemu-img` copy-on-write overlays per node, daemonized processes with pidfiles and logs, QMP status/start/stop/suspend/resume/reset operations, KVM when `/dev/kvm` is accessible with TCG fallback, and direct two-VM Ethernet links through QEMU's loopback datagram backend. QEMU links use the configured adapter number and must be created or deleted while both endpoint VMs are stopped.

QEMU process and QMP paths are persisted with each node so the controller can reconnect after a service restart. Disk selection is constrained to canonical files under the configured image directory. Authentication, `/v2`, `.gns3` import/export, remote compute registration, additional hypervisors, virtual Ethernet switches, and console sessions remain future work.

## Frontend migration

The frontend uses TanStack Start with React and TypeScript in SPA mode. Its build emits `frontend/dist/client/index.html` and static assets; the Rust service serves the app shell for browser routes and serves the app and `/v3` API from the same origin. Node is a build-time dependency only. The Vite development server proxies API and WebSocket traffic to the Rust controller.

The first UI flow follows the observed Web UI sequence: list projects, create or open a project, load nodes/links/drawings from one topology snapshot, then connect to its event stream using that snapshot's cursor. The upstream project map currently reads the three topology collections in sequence and installs its WebSocket after those reads. The Rust UI uses the atomic snapshot and replay cursor already implemented on the controller. Its event reducer handles `node.*`, `link.*`, and `drawing.*` create/update/delete messages in the shape used by the observed project event handler.

The browser supports project creation/list/open/close/delete; adding Docker containers from installed Docker images or QEMU VMs from local disk images; per-container command overrides; Docker and QEMU start/stop/suspend/reload controls; creating and deleting Docker bridge links and stopped-QEMU direct links; dragging nodes; and applying live server events. Stored drawings render after parsing and rebuilding their SVG through an allowlist of elements and attributes; scripts, event handlers, external references, and unsupported elements are removed. The UI does not yet create or edit drawings. Template and lifecycle flows use GNS3-shaped `/v3` routes.

The UI intentionally uses the local server's `/v3` routes rather than copying the supplied UI snapshot's `/v2` base URL. Keep browser interactions in TypeScript while they are event and network bound. Consider a Rust/WASM module only after measurements show a CPU-bound browser task such as layout on large graphs; the current canvas does not warrant a WASM dependency.

Frontend tests cover API route/body formation, notification cursor URLs, project create/open, snapshot loading, Docker and QEMU template-backed node creation, live node events, SVG sanitization, and event reducer idempotence. Rust API tests cover Docker/QEMU node creation, QEMU private disk overlays and QMP lifecycle, generated QEMU link endpoints/arguments, Docker link lifecycle calls, rejection of unsupported node types, duplicate port rejection, consistent snapshots, and resource cleanup on project deletion.

## Initial roadmap

1. Expand QEMU Ethernet from direct point-to-point links to GNS3's shared switch/bridge model, and add guest console sessions and fuller template settings.
2. Add VPCS and Dynamips adapters with their real external processes, console protocols, link handling, and cleanup.
3. Add VMware, VirtualBox, and IOU behind explicit capability-checked adapters.
4. Expand endpoint compatibility, compute registration, and `.gns3` import/export using the server 3.1 OpenAPI schema and sample project files.
5. Decide whether a later client compatibility layer needs `/v2` aliases or should track a Web UI revision aligned with server 3.1.

The notes record upstream source evidence and the migration direction. No GNS3 source code has been copied into this workspace.
