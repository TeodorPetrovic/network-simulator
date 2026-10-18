# Review 1 — static review of API, UI, and runtime workstreams

Scope: the requested API, PAGES/MAP, and RUNTIME files, checked against `docs/PORT-PARITY.md`, `HANDOFF.md`, and the GNS3 3.1 trees under `/tmp`. This is a read-only review; I did not run tests. Findings below are ranked by impact and marked **verified by reading** or **suspected**. Line numbers refer to the reviewed working tree and may move while concurrent edits continue.

The findings below record the initial review state. The Fix status section at the end supersedes those findings where implementation work changed the code or documentation.

## Verified by reading

### P1 — Imported runtime paths can unlink or overwrite arbitrary files

[`save_imported_project`](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:664>) copies each imported node object and only rewrites its ID, project ID, and status ([lines 690–703](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:690>)). It does not discard runtime-owned paths. [`QemuRuntime::vm_from_node`](</home/teodor/projects/network-simulator/src/qemu.rs:801>) accepts `runtime_overlay`, QMP, serial, PID, and log paths directly from that JSON. On project deletion, the API stops and cleans every node ([`src/api.rs:1614`](</home/teodor/projects/network-simulator/src/api.rs:1614>), [`src/api.rs:3340`](</home/teodor/projects/network-simulator/src/api.rs:3340>)); QEMU stop/removal unlinks those paths ([`src/qemu.rs:756`](</home/teodor/projects/network-simulator/src/qemu.rs:756>), [`src/qemu.rs:1675`](</home/teodor/projects/network-simulator/src/qemu.rs:1675>)). Dynamips also accepts imported `runtime_working_dir` and `runtime_hypervisor_port` without confinement ([`dynamips.rs:499`](</home/teodor/projects/network-simulator/src/dynamips.rs:499>)); starting a node uses that directory as its working directory and trusts `runtime_log_file` for log writes ([`dynamips.rs:1208`](</home/teodor/projects/network-simulator/src/dynamips.rs:1208>), [`dynamips.rs:1214`](</home/teodor/projects/network-simulator/src/dynamips.rs:1214>), [`dynamips.rs:1228`](</home/teodor/projects/network-simulator/src/dynamips.rs:1228>)).

**Failure scenario:** POST an imported raw project or `.gns3project` containing a valid QEMU node UUID and `runtime_pid_file` set to a file writable by the service, plus a nonexistent QMP path. DELETE the imported project. The stop path treats the VM as stopped and removes the caller-selected PID path; cleanup also removes the supplied log/runtime paths. Likewise, an imported Dynamips node with a caller-selected `runtime_log_file` can truncate that file when started: the runtime opens it with `File::create`. This is topology JSON trust, not ZIP member extraction: the importer does not extract archive members.

### P1 — Project/node duplication keeps references to the source runtime

[`duplicate_project`](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:716>) changes project and object IDs and marks nodes stopped, but keeps runtime paths and Docker `container_id` ([lines 763–768](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:763>)). [`duplicate_node`](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:962>) clears some runtime fields but omits QEMU overlay/QMP/serial/PID paths ([lines 978–988](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:978>)). Project deletion then stops and removes the duplicated node's runtime resources; Docker cleanup uses the retained container ID ([`src/api.rs:3373`](</home/teodor/projects/network-simulator/src/api.rs:3373>)).

**Failure scenario:** duplicate a project containing a running Docker or QEMU node, then delete the copy. The copied Docker ID can stop/remove the source container; the QEMU copy can target the source QMP socket and overlay paths. Duplicating a QEMU node in place has the same path-alias issue. This exceeds the documented limitation that duplicate nodes are not provisioned independently ([`HANDOFF.md:70`](</home/teodor/projects/network-simulator/HANDOFF.md:70>)): it can affect the original runtime.

### P1 — The generic image upload buffers as much as 64 GiB in memory

[`upload_image`](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:1545>) calls `to_bytes` with `MAX_QEMU_IMAGE_BYTES` before it dispatches the upload ([lines 1554–1564](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:1554>)). The constant is 64 GiB. This controller router is merged separately from the router carrying the normal request body limit ([`src/api.rs:336`](</home/teodor/projects/network-simulator/src/api.rs:336>), [`src/api.rs:346`](</home/teodor/projects/network-simulator/src/api.rs:346>)). Other image upload handlers stream chunks to disk and clean up on failure ([`src/api.rs:837`](</home/teodor/projects/network-simulator/src/api.rs:837>)).

**Failure scenario:** a client sends a large body to `/v3/images/upload/...`; the handler accumulates the entire body in memory before writing it. One request can exhaust service memory, and concurrent uploads multiply the pressure. If validation/finalization fails, this handler also returns without deleting its temporary file ([`controller_routes.rs:1569`](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:1569>), [`controller_routes.rs:1573`](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:1573>), [`controller_routes.rs:1583`](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:1583>)). It also dispatches only `.qcow2`, `.qcow`, `.img`, and `.raw` to QEMU; `.vmdk` and `.vdi` accepted by the QEMU image helper ([`qemu.rs:2474`](</home/teodor/projects/network-simulator/src/qemu.rs:2474>)) fall through to the IOS validator and fail as uploads.

### P1 — Dynamips commands allow newline injection through a node name

The node configuration route accepts a `name` after only trimming and length-checking it ([`src/api.rs:3610`](</home/teodor/projects/network-simulator/src/api.rs:3610>), [`src/api.rs:3644`](</home/teodor/projects/network-simulator/src/api.rs:3644>)); the generic node update also permits `name` ([`src/api.rs:3463`](</home/teodor/projects/network-simulator/src/api.rs:3463>), [`src/api.rs:4510`](</home/teodor/projects/network-simulator/src/api.rs:4510>)). Dynamips formats the name in quotes, but `quote` escapes only backslashes and quotation marks ([`src/dynamips.rs:1760`](</home/teodor/projects/network-simulator/src/dynamips.rs:1760>)). `send` writes the formatted command followed by a newline to the line-oriented local hypervisor protocol ([`src/dynamips.rs:1178`](</home/teodor/projects/network-simulator/src/dynamips.rs:1178>)).

**Failure scenario:** set a Dynamips node name to a short string containing `\n` plus another protocol command, then start or operate the router. The injected newline terminates the first command and places the supplied command on the hypervisor connection. This is protocol-command injection; I did not establish host-shell execution.

### P2 — `/projects/load` reads an arbitrary local path into unbounded memory

[`load_project`](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:638>) takes a client-supplied path and calls `tokio::fs::read` with no size cap or file-type restriction ([lines 642–653](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:642>)).

**Failure scenario:** POST `{"path":"/dev/zero"}` or a very large local file. The service attempts to read the whole file before parsing it, causing unbounded memory growth or an indefinite read. The route is designed to read local project files; this implementation needs a bounded read before trusting that path.

### P2 — ZIP header lengths can panic the import request

[`read_project_archive`](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:1844>) slices the local-header filename at line 1856 before checking that `name_len` fits in the input. A 30-byte local header with a large filename length passes the loop guard and makes that slice out of bounds. The importer accepts up to 64 MiB at the route ([`controller_routes.rs:619`](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:619>)).

**Failure scenario:** submit a truncated ZIP local header with `name_len=65535`. The request future panics instead of returning a client error. The only archive parser test is a valid single-file round trip ([`controller_routes.rs:1884`](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:1884>)); there is no malformed-header/bounds test.

### P2 — Normal GNS3 3.1 project exports are rejected

The importer only accepts stored ZIP entries (`method == 0`) and rejects data descriptors ([`read_project_archive`](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:1844>), especially lines 1857–1860). The GNS3 3.1 export route defaults to Zstandard compression ([upstream `projects.py:721`](</tmp/gns3-server-3.1/gns3server/api/routes/controller/projects.py:721>), default at line 727), so a normal upstream export is rejected.

**Failure scenario:** import a default GNS3 3.1 `.gns3project` and receive a bad request. The single-entry archive limitation is disclosed in `HANDOFF.md`; the default-compression incompatibility still prevents ordinary upstream exports from loading.

### Resolved — Built-in node configuration contract

The current `PUT /v3/projects/{project_id}/nodes/{node_id}/config` handler dispatches built-in node types through `BuiltinRuntime::normalize_configuration_patch` and stores normalized flat fields. `BuiltinNodeEditorPage.save` submits flat patches that match this contract; the earlier finding that built-in saves were rejected no longer applies.

### P2 — Snapshot dates render near January 1970

Rust returns `created_at` as epoch seconds ([`controller_routes.rs:823`](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:823>)); upstream schema also defines it as integer seconds ([`snapshots.py:48`](</tmp/gns3-server-3.1/gns3server/schemas/controller/snapshots.py:48>)). The frontend type declares it as a string ([`api.ts:78`](</home/teodor/projects/network-simulator/frontend/src/api.ts:78>)) and passes it directly to `new Date(...)` ([`SnapshotsPage.tsx:62`](</home/teodor/projects/network-simulator/frontend/src/SnapshotsPage.tsx:62>)), which treats a number as milliseconds. The backend also returns an ISO `created` value, but this page ignores it.

**Failure scenario:** a snapshot created in 2026 displays a date in January 1970 rather than its actual creation time.

### P2 — The map interface picker hides configured adapters above 16

[`interfacesForNode`](</home/teodor/projects/network-simulator/frontend/src/map/helpers.ts:5>) clamps the count to 16 ([line 7](</home/teodor/projects/network-simulator/frontend/src/map/helpers.ts:7>)). The backend accepts QEMU adapter counts through 275 ([`src/api.rs:1156`](</home/teodor/projects/network-simulator/src/api.rs:1156>)) and Dynamips counts through 99 ([`src/api.rs:1158`](</home/teodor/projects/network-simulator/src/api.rs:1158>)).

**Failure scenario:** configure a QEMU or Dynamips node with more than 16 adapters, then use the map interface picker to link an endpoint on a higher adapter. That interface is never offered by the UI.

### P2 — Statistics and bulk console reset return success with stub behavior

[`controller_statistics`](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:299>) hard-codes `uptime_seconds` to 0, `nodes.by_type` and `nodes.by_status` to empty objects, and capture/Web Wireshark counts to 0 regardless of live state ([lines 332–338](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:332>)). [`reset_all_consoles`](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:954>) only checks that the project exists and returns 204 ([lines 954–960](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:954>)).

**Failure scenario:** with a running node or active capture, statistics still reports the hard-coded empty/zero values; calling bulk console reset succeeds but resets nothing. The API parity matrix marks statistics partial, but does not make these constant fields useful. Its bulk reset row says metadata is updated ([`docs/api-parity.md:127`](</home/teodor/projects/network-simulator/docs/api-parity.md:127>)); this bulk handler does not update metadata either.

### P2 — Project export/import silently drops saved project files

Project files are stored in `project_files`, but [`export_project`](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:588>) serializes only the topology and `save_imported_project` imports only nodes, links, and drawings ([`controller_routes.rs:664`](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:664>)).

**Failure scenario:** save `readme.md` through the project file route, export the project, and import it elsewhere. The readme and any other project files are absent. This is consistent with the documented one-entry archive limitation, but means the current export/import pair is not a project backup for files created by the newly added file API.

### P3 — Node-file rows survive node deletion

[`node_files`](</home/teodor/projects/network-simulator/migrations/0003_controller_api.sql:34>) has a project foreign key but no node foreign key or node-deletion cleanup ([lines 34–40](</home/teodor/projects/network-simulator/migrations/0003_controller_api.sql:34>)). Node delete removes its topology row ([`src/api.rs:4672`](</home/teodor/projects/network-simulator/src/api.rs:4672>)) but does not delete these file rows.

**Failure scenario:** delete a node with stored files; its blobs remain in SQLite, consuming space. If a node is later created with that same UUID, the file API can expose the old rows. Project deletion does cascade them.

## Suspected — needs runtime/browser confirmation

### P2 — QEMU extra options can expose service-readable host files to a guest

The QEMU option parser rejects a list of controller-owned flags but appends other `shlex` tokens unchanged to QEMU ([`qemu.rs:2071`](</home/teodor/projects/network-simulator/src/qemu.rs:2071>), [`qemu.rs:2074`](</home/teodor/projects/network-simulator/src/qemu.rs:2074>)). In particular, `-fw_cfg` is not blocked. A project topology can carry an option such as `-fw_cfg name=opt/example,file=/path/readable-by-service`; [QEMU's invocation manual](https://www.qemu.org/docs/master/system/qemu-manpage.html) says this loads a host file into fw_cfg and passes the entry to the guest. If an imported project is started with an attacker-controlled guest, this may disclose host-readable files. The arguments go directly to `Command` and are not shell-interpreted, so I found no host-shell injection in QEMU, Docker, or Dynamips process spawning by static reading.

### P2 — Uploaded SVG symbols may be same-origin active content (mitigated)

Symbol upload still accepts SVG bytes, but the raw route now serves them as attachments with a sandbox CSP and `nosniff`. This prevents the raw response from executing active SVG content as a same-origin document; browser execution was not tested.

## Documentation, route checks, and test gaps

- `docs/ui-pages-parity.md` says the Servers page implements adding/removing computes ([line 28](</home/teodor/projects/network-simulator/docs/ui-pages-parity.md:28>)); the current page is read-only and explicitly says those routes are absent ([`ComputesPage.tsx:39`](</home/teodor/projects/network-simulator/frontend/src/ComputesPage.tsx:39>)). `api.ts` has only a compute-list method ([line 452](</home/teodor/projects/network-simulator/frontend/src/api.ts:452>)), and Rust exposes only GET routes ([`controller_routes.rs:48`](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:48>)). HANDOFF/API parity correctly describe this as read-only/missing.
- `docs/api-parity.md` now marks link filters and packet capture routes as implemented by NET ([lines 97–101](</home/teodor/projects/network-simulator/docs/api-parity.md:97>)); this matches the route registrations and handlers.
- The archive parser now has malformed-header, deflate/Zstandard, project-file round-trip, and runtime-metadata stripping tests. The duplicate deletion test verifies source runtime files survive.
- The statistics HTTP test now asserts live node/link/capture values. Console reset returns 501 and the test verifies stored console metadata stays unchanged. Frontend tests now cover epoch-seconds rendering, adapter ranges, and flat built-in node configuration bodies.
- SQL in the reviewed handlers uses bound parameters; I found no SQL injection from file paths, names, or query values. `clean_relative_path` rejects empty, dot, dot-dot, and NUL path components ([`controller_routes.rs:1082`](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:1082>)); symbol IDs are restricted to a filename ([`controller_routes.rs:1682`](</home/teodor/projects/network-simulator/src/api/controller_routes.rs:1682>)). I found no ZIP-slip extraction path because archive members are not written to disk. The project readme client now uses raw text for GET/POST, matching the Rust handler; snapshot creation and link capture request bodies also match their current handlers.
- At review time, `HANDOFF.md` said the API tests did not run due to the then-current Rust compile failure ([line 71](</home/teodor/projects/network-simulator/HANDOFF.md:71>)). The current build/test status is recorded in the Fix status section below.

## UI contract gaps

A final audit of `frontend/src/api.ts` against routes and validation in `src/api.rs`, `src/api/controller_routes.rs`, `src/links/**`, and `src/builtin/**` found no remaining client path, method, query-key, or request-body mismatch. Built-in node configuration uses the flat body accepted by `BuiltinRuntime::normalize_configuration_patch`. The client uses raw bodies for project readmes and file uploads as the corresponding handlers expect. Packet capture now requests the supported Ethernet DLT and no longer offers the unsupported Web Wireshark action.

- Web Wireshark launch is not implemented: `POST /v3/projects/{project_id}/links/{link_id}/capture/start` returns 501 when `wireshark` is true. Packet capture file and stream routes are available.
- Compute management is read-only: Rust registers only `GET /v3/computes` and `GET /v3/computes/{compute_id}`; add/remove and remote connection management routes do not exist.
- Docker `console_type: none` is still rejected by template and node-configuration validation, so the UI keeps that choice disabled. Docker VNC can start on its allocated TCP console port, but Rust exposes no browser VNC WebSocket route, so the UI cannot open VNC.

## Fix status

Verified against the working tree and addressed in this task:

- Imported projects, project duplicates, and node duplicates now strip runtime-owned node fields, including adapter paths and IDs. Regression coverage checks that deleting copies leaves the source runtime files unchanged.
- Image upload streams in bounded chunks, removes partial temporary files on stream/size/finalization errors, and dispatches `.vmdk` and `.vdi` to QEMU.
- `/v3/projects/load` rejects non-regular files and enforces the 64 MiB read bound. The ZIP reader uses checked archive parsing, accepts deflate and Zstandard archives, bounds expanded entries, rejects malformed header lengths, and preserves `project_files` through export/import.
- Controller statistics report elapsed uptime and counts from stored project, node, link, capture, and Wireshark state. Single and bulk console reset now return 501 because no runtime adapter can perform the reset; they no longer report success after changing metadata only.
- Dynamips Idle-PC endpoints reject non-Dynamips nodes with 422 before attempting runtime parsing.
- Node-file rows are cleaned by migration trigger and explicit node deletion. Symbol responses use attachment disposition, a sandbox CSP, and `nosniff`.
- `docs/api-parity.md` already marks NET link filters and packet capture routes as implemented; this was verified against the route registrations and handlers. Its console-reset rows now say 501 is returned.
- Snapshot `created_at` is typed as epoch seconds and rendered after converting to milliseconds; the UI regression test checks the backend value.
- The map interface helper retains the configured QEMU/Dynamips ranges, and the picker searches scrollable choices above 16; tests cover 275 QEMU and 99 Dynamips interfaces.
- Docker auxiliary console targets use a distinct session ID and the registered `/aux_console/ws` WebSocket route. The UI leaves `console_type: none` disabled and reports that VNC has no browser route.
- App tests mock the heavy map dialogs, use explicit 15-second UI test timeouts, and wrap injected WebSocket events with React `act()`. jsdom stubs canvas, pointer-capture, and scroll-into-view APIs.
- Frontend validation passed: build and typecheck succeeded, and three sequential full runs each passed all 88 tests.

The three named route tests were initially reproduced. The load test expected 400 although this API maps `bad_request` to 422; the statistics test encountered the console-reset stub; the settings test received 409 for a non-Dynamips node. After the fixes, a final test pass could not be established: the test build fails on unresolved names in the pre-existing, out-of-scope edits in `src/qemu.rs` and `src/dynamips.rs`. `cargo check --locked` also fails on moved `project_id`/`node_id` values and a `String`/`&str` mismatch in `src/runtime.rs`.

Other review findings outside this task's assigned files remain: Dynamips node-name newline validation and the QEMU extra-option `-fw_cfg` concern. The snapshot date and adapter picker findings are fixed in the frontend, and built-in node editor saves use the current flat configuration route. Docker auxiliary console UI now uses the registered route. No Rust files were changed in this task.
