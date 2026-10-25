
## 1. Project and node idle lifecycle

**Partial.** Project `auto_close`, `auto_open`, and `auto_start` fields are accepted and persisted; closing a project stops active nodes, and an optional controller idle-stop worker stops nodes after inactivity. However, opening a project does not honor `auto_start`, and startup does not honor `auto_open`. Node idle settings are presently limited to the existing Dynamips `idlepc`/`idlemax`/`idlesleep` behavior; they are not a generic project/node lifecycle mechanism. GNS3 3.1's controller auto-opens projects marked `auto_open` and its `Project.open()` starts nodes for `auto_start` projects.

## 2. Project export and import

**Implemented (with storage-model adaptation).** The controller exports stopped projects as `.gns3project` ZIPs containing `project.gns3`, project files, and optional packaged images; import accepts ZIP archives and raw `.gns3` JSON. The `/projects/load` route also reads local `.gns3` or `.gns3project` paths, regenerates project and node IDs, and provisions imported nodes. This matches GNS3's portable archive and topology-load formats; filesystem placement is represented in the server's database rather than a per-project directory tree.

## 3. VPCS suspend/resume and console types

**Partial.** GNS3's VPCS compute node does not implement a dedicated suspend method, but the controller exposes node lifecycle state. This port rejected VPCS suspension, and it accepted `ssh`/`none` console types even though its console adapter always performs a Telnet handshake. Process suspension now freezes and resumes the tracked VPCS process tree. Console negotiation still uses Telnet for all stored console types, so non-Telnet VPCS console modes remain a parity gap.

## 4. Docker node configuration

**Implemented.** Docker node creation and updates preserve and validate `extra_hosts`, `extra_volumes`, `environment`, `start_command`, and `extra_configs`; runtime construction passes these through to Docker. Existing runtime validation and construction tests cover the fields.
