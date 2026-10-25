# GNS3 3.1 controller API parity matrix

Compared against `~/.cache/gns3-server-3.1/gns3server/api/routes/controller/` route decorators and `controller/__init__.py` router prefixes. Paths below include the server `/v3` prefix. The matrix records route coverage after the auth implementation; auth behavior is summarized below the table.

Statuses mean:

- **Implemented**: current route and behavior match the selected upstream endpoint at the level verified in the existing code.
- **Partial**: a route overlaps by path but has narrower or different behavior.
- **Missing**: endpoint is absent or not yet implemented.
- **Intentionally skipped/deferred**: explicitly outside this workstream or assigned to NET/later phase.

| Upstream module | Method | Path | Status |
| --- | --- | --- | --- |
| `acl.py` | GET | `/v3/access/acl` | Implemented |
| `acl.py` | POST | `/v3/access/acl` | Implemented |
| `acl.py` | GET | `/v3/access/acl/endpoints` | Partial — current endpoint catalog covers projects and access principals |
| `acl.py` | DELETE | `/v3/access/acl/{ace_id}` | Implemented |
| `acl.py` | GET | `/v3/access/acl/{ace_id}` | Implemented |
| `acl.py` | PUT | `/v3/access/acl/{ace_id}` | Partial — path, allow/deny, and propagation fields update |
| `api_keys.py` | GET | `/v3/access/api-keys` | Implemented — current user's keys |
| `api_keys.py` | POST | `/v3/access/api-keys` | Implemented — key secret shown once |
| `api_keys.py` | DELETE | `/v3/access/api-keys/{api_key_id}` | Implemented — current user's keys |
| `api_keys.py` | POST | `/v3/access/api-keys/{api_key_id}/restore` | Implemented — current user's keys |
| `api_keys.py` | POST | `/v3/access/api-keys/{api_key_id}/revoke` | Implemented — current user's keys |
| `appliances.py` | GET | `/v3/appliances` | Missing — appliance list/install routes remain |
| `appliances.py` | GET | `/v3/appliances/{appliance_id}` | Missing — appliance list/install routes remain |
| `appliances.py` | POST | `/v3/appliances/{appliance_id}/install` | Missing — appliance list/install routes remain |
| `appliances.py` | POST | `/v3/appliances/{appliance_id}/version` | Missing — appliance list/install routes remain |
| `chat.py` | POST | `/v3/copilot/projects/{project_id}/chat/inject` | Intentionally skipped — auth, later, or optional phase |
| `chat.py` | GET | `/v3/copilot/projects/{project_id}/chat/sessions` | Intentionally skipped — auth, later, or optional phase |
| `chat.py` | DELETE | `/v3/copilot/projects/{project_id}/chat/sessions/{session_id}` | Intentionally skipped — auth, later, or optional phase |
| `chat.py` | PATCH | `/v3/copilot/projects/{project_id}/chat/sessions/{session_id}` | Intentionally skipped — auth, later, or optional phase |
| `chat.py` | POST | `/v3/copilot/projects/{project_id}/chat/sessions/{session_id}/abort` | Intentionally skipped — auth, later, or optional phase |
| `chat.py` | GET | `/v3/copilot/projects/{project_id}/chat/sessions/{session_id}/history` | Intentionally skipped — auth, later, or optional phase |
| `chat.py` | DELETE | `/v3/copilot/projects/{project_id}/chat/sessions/{session_id}/pin` | Intentionally skipped — auth, later, or optional phase |
| `chat.py` | PUT | `/v3/copilot/projects/{project_id}/chat/sessions/{session_id}/pin` | Intentionally skipped — auth, later, or optional phase |
| `chat.py` | POST | `/v3/copilot/projects/{project_id}/chat/stream` | Intentionally skipped — auth, later, or optional phase |
| `computes.py` | GET | `/v3/computes` | Partial — local compute only |
| `computes.py` | POST | `/v3/computes` | Missing — remote compute administration is unsupported |
| `computes.py` | DELETE | `/v3/computes/{compute_id}` | Missing — remote compute administration is unsupported |
| `computes.py` | GET | `/v3/computes/{compute_id}` | Partial — local compute only |
| `computes.py` | PUT | `/v3/computes/{compute_id}` | Missing — remote compute administration is unsupported |
| `computes.py` | POST | `/v3/computes/{compute_id}/connect` | Missing — remote compute administration is unsupported |
| `computes.py` | POST | `/v3/computes/{compute_id}/disconnect` | Missing — remote compute administration is unsupported |
| `computes.py` | GET | `/v3/computes/{compute_id}/docker/images` | Missing — remote compute administration is unsupported |
| `computes.py` | POST | `/v3/computes/{compute_id}/docker/images/pull` | Missing — remote compute administration is unsupported |
| `computes.py` | POST | `/v3/computes/{compute_id}/dynamips/auto_idlepc` | Missing — remote compute administration is unsupported |
| `computes.py` | GET | `/v3/computes/{compute_id}/virtualbox/vms` | Missing — remote compute administration is unsupported |
| `computes.py` | GET | `/v3/computes/{compute_id}/vmware/vms` | Missing — remote compute administration is unsupported |
| `computes.py` | GET | `/v3/computes/{compute_id}/{emulator}/{endpoint_path:path}` | Missing — remote compute administration is unsupported |
| `computes.py` | POST | `/v3/computes/{compute_id}/{emulator}/{endpoint_path:path}` | Missing — remote compute administration is unsupported |
| `computes.py` | PUT | `/v3/computes/{compute_id}/{emulator}/{endpoint_path:path}` | Missing — remote compute administration is unsupported |
| `controller.py` | GET | `/v3/iou_license` | Missing |
| `controller.py` | PUT | `/v3/iou_license` | Missing |
| `controller.py` | GET | `/v3/notifications` | Partial — local counters/project event payloads; controller notification types differ |
| `controller.py` | WEBSOCKET | `/v3/notifications/ws` | Partial — local counters/project event payloads; controller notification types differ |
| `controller.py` | POST | `/v3/reload` | Missing |
| `controller.py` | POST | `/v3/shutdown` | Missing |
| `controller.py` | GET | `/v3/statistics` | Partial — local counters/project event payloads; controller notification types differ |
| `controller.py` | GET | `/v3/version` | Implemented |
| `controller.py` | POST | `/v3/version` | Implemented |
| `copilot.py` | POST | `/v3/copilot/reload/skills` | Intentionally skipped — auth, later, or optional phase |
| `drawings.py` | GET | `/v3/projects/{project_id}/drawings` | Implemented |
| `drawings.py` | POST | `/v3/projects/{project_id}/drawings` | Implemented |
| `drawings.py` | DELETE | `/v3/projects/{project_id}/drawings/{drawing_id}` | Implemented |
| `drawings.py` | GET | `/v3/projects/{project_id}/drawings/{drawing_id}` | Implemented |
| `drawings.py` | PUT | `/v3/projects/{project_id}/drawings/{drawing_id}` | Implemented |
| `gns3vm.py` | GET | `/v3/gns3vm` | Intentionally skipped — auth, later, or optional phase |
| `gns3vm.py` | PUT | `/v3/gns3vm` | Intentionally skipped — auth, later, or optional phase |
| `gns3vm.py` | GET | `/v3/gns3vm/engines` | Intentionally skipped — auth, later, or optional phase |
| `gns3vm.py` | GET | `/v3/gns3vm/engines/{engine}/vms` | Intentionally skipped — auth, later, or optional phase |
| `groups.py` | GET | `/v3/access/groups` | Implemented |
| `groups.py` | POST | `/v3/access/groups` | Implemented |
| `groups.py` | DELETE | `/v3/access/groups/{user_group_id}` | Partial — built-in groups are protected |
| `groups.py` | GET | `/v3/access/groups/{user_group_id}` | Implemented |
| `groups.py` | PUT | `/v3/access/groups/{user_group_id}` | Partial — built-in groups are protected |
| `groups.py` | GET | `/v3/access/groups/{user_group_id}/members` | Implemented |
| `groups.py` | DELETE | `/v3/access/groups/{user_group_id}/members/{user_id}` | Implemented |
| `groups.py` | PUT | `/v3/access/groups/{user_group_id}/members/{user_id}` | Implemented |
| `images.py` | GET | `/v3/images` | Partial — local QEMU/IOS files; IOU and catalog/sync workflows are absent |
| `images.py` | POST | `/v3/images/compatibility` | Missing — image catalog, sync, prune, IOU, or alternate upload route |
| `images.py` | GET | `/v3/images/compatibility/catalog` | Missing — image catalog, sync, prune, IOU, or alternate upload route |
| `images.py` | POST | `/v3/images/install` | Missing — image catalog, sync, prune, IOU, or alternate upload route |
| `images.py` | DELETE | `/v3/images/prune` | Missing — image catalog, sync, prune, IOU, or alternate upload route |
| `images.py` | POST | `/v3/images/qemu/{image_path:path}` | Missing — image catalog, sync, prune, IOU, or alternate upload route |
| `images.py` | POST | `/v3/images/sync` | Missing — image catalog, sync, prune, IOU, or alternate upload route |
| `images.py` | GET | `/v3/images/sync/jobs/{job_id}` | Missing — image catalog, sync, prune, IOU, or alternate upload route |
| `images.py` | POST | `/v3/images/upload/{image_path:path}` | Partial — local QEMU/IOS files; IOU and catalog/sync workflows are absent |
| `images.py` | DELETE | `/v3/images/{image_path:path}` | Partial — local QEMU/IOS files; IOU and catalog/sync workflows are absent |
| `images.py` | GET | `/v3/images/{image_path:path}` | Partial — local QEMU/IOS files; IOU and catalog/sync workflows are absent |
| `links.py` | GET | `/v3/projects/{project_id}/links` | Implemented |
| `links.py` | POST | `/v3/projects/{project_id}/links` | Implemented |
| `links.py` | POST | `/v3/projects/{project_id}/links/batch` | Missing |
| `links.py` | DELETE | `/v3/projects/{project_id}/links/{link_id}` | Implemented |
| `links.py` | GET | `/v3/projects/{project_id}/links/{link_id}` | Implemented |
| `links.py` | PUT | `/v3/projects/{project_id}/links/{link_id}` | Implemented |
| `links.py` | GET | `/v3/projects/{project_id}/links/{link_id}/available_filters` | Implemented — NET |
| `links.py` | GET | `/v3/projects/{project_id}/links/{link_id}/capture/file` | Implemented — NET |
| `links.py` | POST | `/v3/projects/{project_id}/links/{link_id}/capture/start` | Implemented — NET |
| `links.py` | POST | `/v3/projects/{project_id}/links/{link_id}/capture/stop` | Implemented — NET |
| `links.py` | GET | `/v3/projects/{project_id}/links/{link_id}/capture/stream` | Implemented — NET |
| `links.py` | WEBSOCKET | `/v3/projects/{project_id}/links/{link_id}/capture/web-wireshark` | Intentionally deferred — NET workstream |
| `links.py` | POST | `/v3/projects/{project_id}/links/{link_id}/capture/wireshark/restart` | Intentionally deferred — NET workstream |
| `links.py` | GET | `/v3/projects/{project_id}/links/{link_id}/iface` | Intentionally deferred — NET workstream |
| `links.py` | GET | `/v3/projects/{project_id}/links/{link_id}/markers` | Intentionally deferred — NET workstream |
| `links.py` | POST | `/v3/projects/{project_id}/links/{link_id}/markers` | Intentionally deferred — NET workstream |
| `links.py` | DELETE | `/v3/projects/{project_id}/links/{link_id}/markers/{marker_name}` | Intentionally deferred — NET workstream |
| `links.py` | PUT | `/v3/projects/{project_id}/links/{link_id}/markers/{marker_name}` | Intentionally deferred — NET workstream |
| `links.py` | POST | `/v3/projects/{project_id}/links/{link_id}/reset` | Missing |
| `llm_model_configs.py` | GET | `/v3/access/groups/{group_id}/llm-model-configs` | Intentionally skipped — auth, later, or optional phase |
| `llm_model_configs.py` | POST | `/v3/access/groups/{group_id}/llm-model-configs` | Intentionally skipped — auth, later, or optional phase |
| `llm_model_configs.py` | GET | `/v3/access/groups/{group_id}/llm-model-configs/default` | Intentionally skipped — auth, later, or optional phase |
| `llm_model_configs.py` | PUT | `/v3/access/groups/{group_id}/llm-model-configs/default/{config_id}` | Intentionally skipped — auth, later, or optional phase |
| `llm_model_configs.py` | DELETE | `/v3/access/groups/{group_id}/llm-model-configs/{config_id}` | Intentionally skipped — auth, later, or optional phase |
| `llm_model_configs.py` | PUT | `/v3/access/groups/{group_id}/llm-model-configs/{config_id}` | Intentionally skipped — auth, later, or optional phase |
| `llm_model_configs.py` | GET | `/v3/access/users/{user_id}/llm-model-configs` | Intentionally skipped — auth, later, or optional phase |
| `llm_model_configs.py` | POST | `/v3/access/users/{user_id}/llm-model-configs` | Intentionally skipped — auth, later, or optional phase |
| `llm_model_configs.py` | GET | `/v3/access/users/{user_id}/llm-model-configs/default` | Intentionally skipped — auth, later, or optional phase |
| `llm_model_configs.py` | PUT | `/v3/access/users/{user_id}/llm-model-configs/default/{config_id}` | Intentionally skipped — auth, later, or optional phase |
| `llm_model_configs.py` | GET | `/v3/access/users/{user_id}/llm-model-configs/own` | Intentionally skipped — auth, later, or optional phase |
| `llm_model_configs.py` | DELETE | `/v3/access/users/{user_id}/llm-model-configs/{config_id}` | Intentionally skipped — auth, later, or optional phase |
| `llm_model_configs.py` | PUT | `/v3/access/users/{user_id}/llm-model-configs/{config_id}` | Intentionally skipped — auth, later, or optional phase |
| `netmiko.py` | GET | `/v3/netmiko/device_types` | Intentionally skipped — auth, later, or optional phase |
| `nodes.py` | GET | `/v3/projects/{project_id}/nodes` | Implemented |
| `nodes.py` | POST | `/v3/projects/{project_id}/nodes` | Implemented |
| `nodes.py` | POST | `/v3/projects/{project_id}/nodes/batch` | Missing |
| `nodes.py` | POST | `/v3/projects/{project_id}/nodes/console/reset` | Not implemented — returns 501 until runtime console reset is supported |
| `nodes.py` | POST | `/v3/projects/{project_id}/nodes/reload` | Partial |
| `nodes.py` | POST | `/v3/projects/{project_id}/nodes/start` | Partial |
| `nodes.py` | POST | `/v3/projects/{project_id}/nodes/stop` | Partial |
| `nodes.py` | POST | `/v3/projects/{project_id}/nodes/suspend` | Partial — supported runtimes suspend; VPCS is skipped because GNS3 does not support its process suspension |
| `nodes.py` | DELETE | `/v3/projects/{project_id}/nodes/{node_id}` | Implemented |
| `nodes.py` | GET | `/v3/projects/{project_id}/nodes/{node_id}` | Implemented |
| `nodes.py` | PUT | `/v3/projects/{project_id}/nodes/{node_id}` | Implemented |
| `nodes.py` | POST | `/v3/projects/{project_id}/nodes/{node_id}/console/reset` | Not implemented — returns 501 until runtime console reset is supported |
| `nodes.py` | WEBSOCKET | `/v3/projects/{project_id}/nodes/{node_id}/console/vnc` | Missing — controller console proxy endpoints remain |
| `nodes.py` | WEBSOCKET | `/v3/projects/{project_id}/nodes/{node_id}/console/ws` | Missing — controller console proxy endpoints remain |
| `nodes.py` | POST | `/v3/projects/{project_id}/nodes/{node_id}/duplicate` | Partial — copies topology metadata without provisioning a separate runtime instance |
| `nodes.py` | GET | `/v3/projects/{project_id}/nodes/{node_id}/dynamips/auto_idlepc` | Partial — hypervisor protocol implemented for running nodes; stopped-node start/stop workflow is absent |
| `nodes.py` | GET | `/v3/projects/{project_id}/nodes/{node_id}/dynamips/idlepc_proposals` | Partial — hypervisor protocol implemented for running nodes; stopped-node start/stop workflow is absent |
| `nodes.py` | GET | `/v3/projects/{project_id}/nodes/{node_id}/files` | Partial — database-backed node files rather than compute node directories |
| `nodes.py` | DELETE | `/v3/projects/{project_id}/nodes/{node_id}/files/{file_path:path}` | Partial — database-backed node files rather than compute node directories |
| `nodes.py` | GET | `/v3/projects/{project_id}/nodes/{node_id}/files/{file_path:path}` | Partial — database-backed node files rather than compute node directories |
| `nodes.py` | POST | `/v3/projects/{project_id}/nodes/{node_id}/files/{file_path:path}` | Partial — database-backed node files rather than compute node directories |
| `nodes.py` | POST | `/v3/projects/{project_id}/nodes/{node_id}/isolate` | Partial — updates stored node metadata; does not reset compute console/link runtime state |
| `nodes.py` | GET | `/v3/projects/{project_id}/nodes/{node_id}/links` | Implemented |
| `nodes.py` | DELETE | `/v3/projects/{project_id}/nodes/{node_id}/qemu/disk_image/{disk_name}` | Partial — local qemu-img operations use a local disk folder |
| `nodes.py` | POST | `/v3/projects/{project_id}/nodes/{node_id}/qemu/disk_image/{disk_name}` | Partial — local qemu-img operations use a local disk folder |
| `nodes.py` | PUT | `/v3/projects/{project_id}/nodes/{node_id}/qemu/disk_image/{disk_name}` | Partial — local qemu-img operations use a local disk folder |
| `nodes.py` | POST | `/v3/projects/{project_id}/nodes/{node_id}/reload` | Implemented |
| `nodes.py` | POST | `/v3/projects/{project_id}/nodes/{node_id}/start` | Implemented |
| `nodes.py` | POST | `/v3/projects/{project_id}/nodes/{node_id}/stop` | Implemented |
| `nodes.py` | POST | `/v3/projects/{project_id}/nodes/{node_id}/suspend` | Implemented — VPCS returns 405, matching GNS3 |
| `nodes.py` | POST | `/v3/projects/{project_id}/nodes/{node_id}/unisolate` | Partial |
| `pools.py` | GET | `/v3/pools` | Implemented |
| `pools.py` | POST | `/v3/pools` | Implemented |
| `pools.py` | DELETE | `/v3/pools/{resource_pool_id}` | Partial — deletion is blocked while ACLs reference the pool |
| `pools.py` | GET | `/v3/pools/{resource_pool_id}` | Implemented |
| `pools.py` | PUT | `/v3/pools/{resource_pool_id}` | Implemented |
| `pools.py` | GET | `/v3/pools/{resource_pool_id}/resources` | Implemented |
| `pools.py` | DELETE | `/v3/pools/{resource_pool_id}/resources/{resource_id}` | Implemented |
| `pools.py` | PUT | `/v3/pools/{resource_pool_id}/resources/{resource_id}` | Implemented — project resources only |
| `privileges.py` | GET | `/v3/access/privileges` | Implemented |
| `projects.py` | GET | `/v3/projects` | Implemented |
| `projects.py` | POST | `/v3/projects` | Implemented |
| `projects.py` | POST | `/v3/projects/load` | Partial — DB-backed implementation; archive, filesystem, or event behavior is narrower than GNS3 |
| `projects.py` | DELETE | `/v3/projects/{project_id}` | Implemented |
| `projects.py` | GET | `/v3/projects/{project_id}` | Implemented |
| `projects.py` | PUT | `/v3/projects/{project_id}` | Implemented |
| `projects.py` | POST | `/v3/projects/{project_id}/close` | Implemented |
| `projects.py` | POST | `/v3/projects/{project_id}/duplicate` | Partial — DB-backed implementation; archive, filesystem, or event behavior is narrower than GNS3 |
| `projects.py` | GET | `/v3/projects/{project_id}/export` | Partial — writes a GNS3 `.gns3project` ZIP; project files are included and `include_images=true` packages local QEMU/IOS images; export is size-limited |
| `projects.py` | GET | `/v3/projects/{project_id}/files/{file_path:path}` | Partial — DB-backed implementation; archive, filesystem, or event behavior is narrower than GNS3 |
| `projects.py` | POST | `/v3/projects/{project_id}/files/{file_path:path}` | Partial — DB-backed implementation; archive, filesystem, or event behavior is narrower than GNS3 |
| `projects.py` | GET | `/v3/projects/{project_id}/gns3file` | Implemented |
| `projects.py` | POST | `/v3/projects/{project_id}/import` | Partial — reads GNS3 `.gns3project` ZIP/JSON, installs included QEMU/IOS images, and materializes supported node runtimes; filesystem project paths and some upstream node types remain unsupported |
| `projects.py` | POST | `/v3/projects/{project_id}/lock` | Partial — records locked state; does not enforce every GNS3 lock interaction |
| `projects.py` | GET | `/v3/projects/{project_id}/locked` | Implemented |
| `projects.py` | GET | `/v3/projects/{project_id}/marker-definitions` | Missing — marker API not in the requested priority list |
| `projects.py` | POST | `/v3/projects/{project_id}/marker-definitions` | Missing — marker API not in the requested priority list |
| `projects.py` | DELETE | `/v3/projects/{project_id}/marker-definitions/{def_name}` | Missing — marker API not in the requested priority list |
| `projects.py` | PUT | `/v3/projects/{project_id}/marker-definitions/{def_name}` | Missing — marker API not in the requested priority list |
| `projects.py` | POST | `/v3/projects/{project_id}/marker-definitions/{def_name}/pause` | Missing — marker API not in the requested priority list |
| `projects.py` | POST | `/v3/projects/{project_id}/marker-definitions/{def_name}/resume` | Missing — marker API not in the requested priority list |
| `projects.py` | GET | `/v3/projects/{project_id}/markers` | Missing — marker API not in the requested priority list |
| `projects.py` | GET | `/v3/projects/{project_id}/markers/tags/{tag}/replay/frame/detail` | Missing — marker API not in the requested priority list |
| `projects.py` | GET | `/v3/projects/{project_id}/markers/tags/{tag}/replay/frames` | Missing — marker API not in the requested priority list |
| `projects.py` | GET | `/v3/projects/{project_id}/markers/tags/{tag}/replay/range` | Missing — marker API not in the requested priority list |
| `projects.py` | GET | `/v3/projects/{project_id}/notifications` | Partial — DB-backed implementation; archive, filesystem, or event behavior is narrower than GNS3 |
| `projects.py` | WEBSOCKET | `/v3/projects/{project_id}/notifications/markers/ws` | Missing — marker API not in the requested priority list |
| `projects.py` | WEBSOCKET | `/v3/projects/{project_id}/notifications/ws` | Implemented |
| `projects.py` | POST | `/v3/projects/{project_id}/open` | Implemented |
| `projects.py` | GET | `/v3/projects/{project_id}/stats` | Partial |
| `projects.py` | POST | `/v3/projects/{project_id}/templates/{template_id}` | Implemented |
| `projects.py` | POST | `/v3/projects/{project_id}/unlock` | Partial — records locked state; does not enforce every GNS3 lock interaction |
| `roles.py` | GET | `/v3/access/roles` | Implemented |
| `roles.py` | POST | `/v3/access/roles` | Implemented |
| `roles.py` | DELETE | `/v3/access/roles/{role_id}` | Partial — built-in roles are protected |
| `roles.py` | GET | `/v3/access/roles/{role_id}` | Implemented |
| `roles.py` | PUT | `/v3/access/roles/{role_id}` | Partial — built-in roles are protected |
| `roles.py` | GET | `/v3/access/roles/{role_id}/privileges` | Implemented |
| `roles.py` | DELETE | `/v3/access/roles/{role_id}/privileges/{privilege_id}` | Implemented |
| `roles.py` | PUT | `/v3/access/roles/{role_id}/privileges/{privilege_id}` | Implemented |
| `settings.py` | GET | `/v3/settings` | Partial — generic persisted settings; lacks GNS3 config validation and secret masking |
| `settings.py` | PUT | `/v3/settings` | Partial — generic persisted settings; lacks GNS3 config validation and secret masking |
| `snapshots.py` | GET | `/v3/projects/{project_id}/snapshots` | Partial — topology snapshots; restore does not rehydrate runtime processes |
| `snapshots.py` | POST | `/v3/projects/{project_id}/snapshots` | Partial — topology snapshots; restore does not rehydrate runtime processes |
| `snapshots.py` | DELETE | `/v3/projects/{project_id}/snapshots/{snapshot_id}` | Partial — topology snapshots; restore does not rehydrate runtime processes |
| `snapshots.py` | POST | `/v3/projects/{project_id}/snapshots/{snapshot_id}/restore` | Partial — topology snapshots; restore does not rehydrate runtime processes |
| `symbols.py` | GET | `/v3/symbols` | Partial — custom DB symbols; built-in resource symbols/theme mapping are incomplete |
| `symbols.py` | GET | `/v3/symbols/default_symbols` | Partial — custom DB symbols; built-in resource symbols/theme mapping are incomplete |
| `symbols.py` | DELETE | `/v3/symbols/{symbol_id:path}` | Partial — custom DB symbols; built-in resource symbols/theme mapping are incomplete |
| `symbols.py` | GET | `/v3/symbols/{symbol_id:path}/dimensions` | Partial — custom DB symbols; built-in resource symbols/theme mapping are incomplete |
| `symbols.py` | GET | `/v3/symbols/{symbol_id:path}/raw` | Partial — custom DB symbols; built-in resource symbols/theme mapping are incomplete |
| `symbols.py` | POST | `/v3/symbols/{symbol_id:path}/raw` | Partial — custom DB symbols; built-in resource symbols/theme mapping are incomplete |
| `templates.py` | GET | `/v3/templates` | Implemented |
| `templates.py` | POST | `/v3/templates` | Implemented |
| `templates.py` | DELETE | `/v3/templates/{template_id}` | Partial |
| `templates.py` | GET | `/v3/templates/{template_id}` | Implemented |
| `templates.py` | PUT | `/v3/templates/{template_id}` | Partial |
| `templates.py` | GET | `/v3/templates/{template_id}/base-config/{filename}` | Missing — template base-config file endpoints remain |
| `templates.py` | PUT | `/v3/templates/{template_id}/base-config/{filename}` | Missing — template base-config file endpoints remain |
| `templates.py` | GET | `/v3/templates/{template_id}/base-configs` | Missing — template base-config file endpoints remain |
| `templates.py` | POST | `/v3/templates/{template_id}/duplicate` | Implemented |
| `users.py` | GET | `/v3/access/users` | Implemented |
| `users.py` | POST | `/v3/access/users` | Implemented |
| `users.py` | POST | `/v3/access/users/authenticate` | Implemented — JSON credentials endpoint |
| `users.py` | POST | `/v3/access/users/login` | Implemented — OAuth2 password form |
| `users.py` | POST | `/v3/access/users/logout` | Partial — revokes all JWTs by token version |
| `users.py` | GET | `/v3/access/users/me` | Implemented |
| `users.py` | PUT | `/v3/access/users/me` | Implemented — profile and password update |
| `users.py` | POST | `/v3/access/users/refresh` | Implemented — signed refresh JWT |
| `users.py` | DELETE | `/v3/access/users/{user_id}` | Partial — superadmin account is protected |
| `users.py` | GET | `/v3/access/users/{user_id}` | Implemented |
| `users.py` | PUT | `/v3/access/users/{user_id}` | Implemented |
| `users.py` | GET | `/v3/access/users/{user_id}/groups` | Implemented |

## Selected classroom parity work

This pass addresses portable `.gns3project` hand-ins, VPCS console modes and
GNS3's unsupported VPCS suspend response, and Docker injected configuration
files. The other classroom-facing missing routes selected for follow-up are
node/link batch operations (useful for large pasted or imported topologies)
and local appliance listing/install routes (useful for preparing a course's
image catalog). Marker replay, remote compute administration, and other
optional integrations remain outside this pass.

## Authentication behavior

- Auth routes and database models live in `src/auth/`; migration `0006_auth.sql` adds users, groups, roles, privileges, ACL entries, project resource pools, API keys, and auth configuration. OAuth2 password form login is at `/v3/access/users/login`; JSON login, refresh, logout, `/me`, profile updates, and user CRUD are also available.
- Passwords use Argon2id. Access and refresh tokens are HS256 JWTs; the signing secret is randomly generated once and persisted in `auth_config`. Login and refresh responses include `expires_in` and `refresh_expires_in` in seconds. API keys (`gns3_<api_key_id>_<secret>`) are accepted as bearer tokens; key secrets are returned once, stored as Argon2 hashes, and can be revoked, restored, or deleted by their owner.
- A new database creates an `admin` superadmin with a random initial password. That password is printed once at bootstrap; it is not `admin/admin`, which is a public default credential. Change it after the first login. `NETWORK_SIMULATOR_AUTH_ENABLED` explicitly overrides the persisted setting. Without an override, the first startup defaults auth OFF for loopback binds and ON for non-loopback binds; the setting can be changed in Settings by a superadmin and persists in SQLite. Startup logs the effective auth state.
- When enabled, API routes require a bearer token; WebSocket routes accept the same token in the `token` query parameter. Privileges are resolved per method and endpoint family, then checked against user and group ACEs. ACL paths omit `/v3`; nearer paths replace inherited rules, `propagate` controls allow inheritance, direct user ACEs are checked before group ACEs, and project access can be granted through resource-pool membership.
- The ACL endpoint catalog currently includes projects and access principals; node/link/template/image endpoint enumeration and broader route-specific policy detail remain partial. Resource pools currently contain project resources only. The OAuth2 refresh response and API-key shape follow GNS3 but token algorithms and persistence details differ.

**Upstream controller routes enumerated:** 223 method/path pairs.

The local router also has product-specific endpoints (`/v3/runtimes`, `/v3/resources/monitor`, marketplace routes, runtime image uploads and runtime console WebSockets) that do not correspond to controller routes in this upstream tree; they are outside this matrix.
