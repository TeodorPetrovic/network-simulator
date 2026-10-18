# UI QA report

Date: 2026-10-18  
Tested build: committed `HEAD` `5105535`, built in `/tmp/qa/checkout`  
Controller: `127.0.0.1:3181`, isolated database and runtime/image/appliance directories under `/tmp/qa`

## Browser limitation

I could not run the UI in a browser. T3 `preview_status` reported no available browser, and `preview_open` returned:

> Preview automation open failed: This host blocks the sandbox T3's browser runs in (AppArmor on Ubuntu 23.10+). Run `sudo /home/teodor/.t3/runtime/versions/0.1.1-nightly.20261016.3203/t3 browser setup` on the host once to allow it, then try again.

There was no installed Chromium executable or local Playwright/Puppeteer package. The frontend build succeeded, and the server returned SPA HTML with status 200 for each checked route, but I could not execute the frontend JavaScript. Visual layout, click/keyboard behavior, browser console errors, and browser network logs are therefore **untested**. I used API-level checks of the page calls and WebSocket upgrade handshakes instead.

The frontend was absent from the temporary committed worktree because `frontend/dist` is ignored, so I ran `npm run build --prefix frontend` there. The `node_modules` symlink pointed to the existing checkout; generated files stayed under `/tmp/qa/checkout`.

## Findings

### P1 — Templates saved in Preferences cannot add runtime nodes to a project

**Expected:** A saved template listed by the controller should create a node when used from the map palette. GNS3 documents `POST /projects/{project_id}/templates/{template_id}` as creating a node from a stored template; it returns 404 when the project or template does not exist ([GNS3 template-node API](https://api.gns3.net/en/stable/api/v2/controller/template/projectsprojectidtemplatestemplateid.html)).

**Observed:** Creating VPCS, Docker, and QEMU templates through `POST /v3/templates` returned 201, and `GET /v3/templates` listed each returned ID. Posting the map's normal add-node request to `/v3/projects/{project_id}/templates/{template_id}` returned 404 `Template … does not exist` for all three. The raw node endpoint accepted the same runtime types (201), so this is template lookup on the map-add path.

**Reproduction:** Create a VPCS, Docker, or QEMU template in Preferences; open a project; add that template from the map palette. The palette's add request returns 404 and no node is created. This blocks the normal map workflow for user-created runtime templates.

### P2 — Start all and Stop all fail when always-on built-ins are present

**Expected:** GNS3 describes Start all as starting/resuming stopped or paused nodes and Stop all as stopping running nodes ([GNS3 project topology actions](https://docs.gns3.com/docs-3.1-en/web-ui/project-topology)). Always-on switch/hub and Cloud/NAT nodes should not prevent the batch action from reaching stoppable runtime nodes.

**Observed:** With an Ethernet switch, Cloud, NAT, and a stopped VPCS in one project, `POST /v3/projects/{project_id}/nodes/start` returned 405 `Ethernet switches and hubs are always running`; the VPCS stayed stopped. After starting VPCS individually, `POST /v3/projects/{project_id}/nodes/stop` returned 405 `Cloud and NAT nodes cannot be stopped, reloaded, or suspended`; VPCS stayed started. The map offers these batch actions based on topology status, so they can be enabled in this state while failing without performing the requested work.

### P2 — QEMU Reload returns an error and leaves the node stopped

**Expected:** Reload should reset a running QEMU node. GNS3's QEMU reload endpoint reports a successful reload with 204 ([GNS3 QEMU reload API](https://api.gns3.net/en/latest/api/v2/compute/qemu/projectsprojectidqemunodesnodeidreload.html)).

**Observed:** With a valid temporary 16 MiB QCOW2 fixture, QEMU start returned 200 / `started`. Reload returned 503 (`Could not read the QEMU QMP greeting: Connection reset by peer (os error 104)` in the isolated-socket retest), and a subsequent node read reported `stopped`. The failure reproduced after start both with and without a preceding suspend. QEMU stop and suspend requests returned 200.

### P3 — Computes page reports the default port under a non-default bind

**Expected:** The Local compute row should report the endpoint used by the running controller.

**Observed:** This controller was bound to `127.0.0.1:3181`, while `GET /v3/computes` returned Local at `127.0.0.1:3080`. The Computes page displays the returned host and port, so it would show a stale endpoint when `NETWORK_SIMULATOR_BIND` changes from the default. This is limited to non-default bind configurations.

## Page and feature coverage

| Page or feature | API-level result |
|---|---|
| Projects list | List/create/open/rename/lock/unlock/stats/readme write/read/duplicate/export/import/delete calls succeeded. Export was a valid ZIP and imported as a separate project. A new project's README GET returned 404 until the file was written; write/readback succeeded. |
| Project map | Switch, hub, Cloud, and NAT nodes were added through the template route. Docker, VPCS, and QEMU nodes were created through the raw node endpoint for runtime checks; the normal Preferences-template add route is the P1 failure above. Node move/rename/lock/isolate/unisolate, topology/interface data, VPCS-to-switch link create/edit/filter-list/delete, drawing create/move/delete, and snapshot create/restore/delete calls succeeded. Two selected-node delete calls returned 204 and left no nodes. Actual selection, drag gestures, interface picker UI, context menus, and map rendering were not browser-tested. |
| Node lifecycle and consoles | Docker start/suspend/reload/stop returned 200; its console WebSocket upgraded with 101. VPCS start/reload/stop returned 200; its console WebSocket upgraded with 101. VPCS suspend returned 409, but the current UI hides Suspend for VPCS. QEMU start/suspend/stop returned 200; QEMU reload is the P2 failure above. WebSocket checks verified the handshake only, not interactive console rendering or input. |
| Preferences/templates | Create/update/duplicate/delete template calls succeeded. Templates created here could not be used by the map-add endpoint (P1). |
| Appliances manager | Manager/read/imported-list calls returned 200. QEMU image upload returned 200; creating a temporary custom appliance from the local Alpine template returned 200 and appeared on manager reload. The GNS3 registry initial query returned results, and a valid iShare QEMU search (`q=alpine&kind=qemu`) returned results. I did not download a catalog image or import/bind a registry appliance. |
| Settings | `GET /v3/settings` returned `{}`; empty update returned 200. The UI's no-editable-settings state is consistent with this response. No nonempty setting was available to round-trip. |
| System status | Resource monitor and runtime status calls returned 200 with host/controller metrics and runtime versions. |
| Installed software | Runtime status returned 200: Docker 29.6.2, QEMU 10.2.1 with KVM, VPCS 0.8.5, and uBridge 1.3.0 were available; Dynamips was unavailable. Installation controls are explicitly disabled in the page. |
| Computes | Compute and runtime calls returned 200. Local endpoint port mismatch is noted above. |

Other limits: I did not test Dynamips because no Dynamips executable/image was installed. I added Cloud and NAT nodes but did not connect them to host interfaces. No test required creating TAP/veth devices. An extra invalid catalog probe using `kind=vpcs` returned 502 validation error; the UI's valid QEMU catalog query succeeded, so this is not a product finding. The new-project README 404 is recorded as observed behavior, not classified as a defect without browser confirmation.

## Isolation and cleanup

Before testing, Docker showed four running simulator containers, and the host already had QEMU PID 55777 using `data/qemu`, VPCS PIDs 542057/542067, uBridge PID 534912, and multiple `ns*`/`nst*` interfaces. These were left untouched. After testing, the Docker container ID set and `ip -o link` output matched the recorded baseline; the exact test Docker container name was absent. `ps` showed no `/tmp/qa` controller/uBridge/QEMU process, port 3181 was no longer listening, and the only QEMU/VPCS processes were the pre-existing PIDs. The broad first pass used the default QMP socket directory once; its per-test sockets were removed on stop. The repeated QEMU check used `NETWORK_SIMULATOR_QEMU_SOCKETS=/tmp/qa/state/qmp` and left no QMP socket behind.

The request script and API response log are in `/tmp/qa/scripts/api-qa.py` and `/tmp/qa/api-run.json`. No source files were edited by this QA run, and no commit was made.
