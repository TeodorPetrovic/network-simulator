
## 1. Project and node idle lifecycle

**Partial.** Project `auto_close`, `auto_open`, and `auto_start` fields are accepted and persisted; closing a project stops active nodes, and an optional controller idle-stop worker stops nodes after inactivity. However, opening a project does not honor `auto_start`, and startup does not honor `auto_open`. Node idle settings are presently limited to the existing Dynamips `idlepc`/`idlemax`/`idlesleep` behavior; they are not a generic project/node lifecycle mechanism. GNS3 3.1's controller auto-opens projects marked `auto_open` and its `Project.open()` starts nodes for `auto_start` projects.
