
## 1. Project and node idle lifecycle

**Partial.** Project `auto_close`, `auto_open`, and `auto_start` fields are accepted and persisted; closing a project stops active nodes, an optional controller idle-stop worker stops nodes after inactivity, and opening now starts nodes when `auto_start` is set. Startup still does not auto-open flagged projects, and the inactivity worker is configured globally rather than using `auto_close`. Node idle settings remain limited to Dynamips `idlepc`/`idlemax`/`idlesleep` behavior. GNS3 3.1 auto-opens projects marked `auto_open` and uses `auto_close` to close a project after its last client disconnects.

## 2. Project export and import

**Implemented (with storage-model adaptation).** The controller exports stopped projects as `.gns3project` ZIPs containing `project.gns3`, project files, and optional packaged images; import accepts ZIP archives and raw `.gns3` JSON. The `/projects/load` route also reads local `.gns3` or `.gns3project` paths, regenerates project and node IDs, and provisions imported nodes. This matches GNS3's portable archive and topology-load formats; filesystem placement is represented in the server's database rather than a per-project directory tree.

## 3. VPCS suspend/resume and console types

**Partial.** GNS3's VPCS compute node does not implement a dedicated suspend method, but the controller exposes node lifecycle state. This port rejected VPCS suspension, and it accepted `ssh`/`none` console types even though its console adapter always performs a Telnet handshake. Process suspension now freezes and resumes the tracked VPCS process tree. Console negotiation still uses Telnet for all stored console types, so non-Telnet VPCS console modes remain a parity gap.

## 4. Docker node configuration

**Implemented.** Docker node creation and updates preserve and validate `extra_hosts`, `extra_volumes`, `environment`, `start_command`, and `extra_configs`; runtime construction passes these through to Docker. Existing runtime validation and construction tests cover the fields.
