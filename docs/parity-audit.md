
## 1. Project and node idle lifecycle

**Partial.** Project `auto_close`, `auto_open`, and `auto_start` fields are accepted and persisted; closing a project stops active nodes, and an optional controller idle-stop worker stops nodes after inactivity. However, opening a project does not honor `auto_start`, and startup does not honor `auto_open`. Node idle settings are presently limited to the existing Dynamips `idlepc`/`idlemax`/`idlesleep` behavior; they are not a generic project/node lifecycle mechanism. GNS3 3.1's controller auto-opens projects marked `auto_open` and its `Project.open()` starts nodes for `auto_start` projects.

## 2. Project export and import

**Implemented (with storage-model adaptation).** The controller exports stopped projects as `.gns3project` ZIPs containing `project.gns3`, project files, and optional packaged images; import accepts ZIP archives and raw `.gns3` JSON. The `/projects/load` route also reads local `.gns3` or `.gns3project` paths, regenerates project and node IDs, and provisions imported nodes. This matches GNS3's portable archive and topology-load formats; filesystem placement is represented in the server's database rather than a per-project directory tree.
