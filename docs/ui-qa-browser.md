# Browser UI QA

Date: 2026-10-21  
Build: current working tree, including the carried-over uncommitted frontend changes  
Browser: headless Chromium 1248 with `playwright-core` installed under `~/.cache/qa-tools`  
Controller: `127.0.0.1:3220`, authentication enabled; database and controller, image, appliance, QEMU, Dynamips, VPCS, uBridge, capture, and Docker state directories isolated under `~/.cache/bq-d`.

The browser exercised the built frontend served by the Rust controller. The first administrator login required a password change. A student account was created with a Project.Audit role and a project ACL; signing in as that student showed only the authorized project. The final logout check cleared the session, and navigating to Settings redirected to `/login?returnTo=%2Fsettings`.

## Browser results

| Area | Verified in Chromium |
|---|---|
| Login and account | Initial sign-in, forced password change, subsequent sign-in, sign-out, protected-route redirect, user settings page. The earlier screenshots also cover wrong-password feedback and rate limiting. |
| Project list | Create and open a project; export to `.gns3project`; duplicate; import the archive as a new project; edit and save README; delete the original, duplicate, and imported workflow fixtures. |
| Device library and map | Added Ethernet switch, hub, Cloud, NAT, Docker, VPCS, QEMU, and Dynamips nodes. Dragged a saved VPCS template onto the canvas and moved an existing node by pointer drag. Linked VPCS 1 to VPCS 2 through the interface picker. Created a rectangle drawing. Created, restored, and deleted a project snapshot. |
| Node actions | Start all and Stop all; Docker and QEMU start, suspend, resume, reload, and stop; VPCS start, reload, and stop; Dynamips start, reload, suspend, resume, and stop. VPCS suspend is not offered because this runtime does not support it. |
| Console dock | Docker `echo browser_docker` returned output; VPCS `show` returned interface information. The QEMU serial console connected and accepted typed input, but the guest produced no visible serial output during the check. The dock was resized at several viewport sizes. Docker also sent WebSocket resize frames with row and column counts. |
| Preferences and templates | Opened the template wizard, created Docker, QEMU, VPCS, and Dynamips templates, edited Docker settings, duplicated and deleted a VPCS template, and opened the registry/import step. Saved runtime templates were added to the map. |
| Appliance manager | Created a custom QEMU appliance from the isolated CHR image, opened the global image list, and opened the image/appliance catalog. No registry import or catalog download was performed. |
| Administration pages | Opened Computes, System status, Installed software, Settings, all access-management sections, and User settings. Created a user, group, limited role, and project ACL in the UI. |

Screenshots for the current run are in [`docs/evidence/browser-qa/`](evidence/browser-qa/), including [the student project list](evidence/browser-qa/run-student-projects.png), [the map after linking](evidence/browser-qa/run-interface-link-created.png), [the fixed node context menu](evidence/browser-qa/run-node-context-fixed.png), [the Docker console](evidence/browser-qa/run-console-alpine-1.png), [the VPCS console](evidence/browser-qa/run-vpcs-console.png), [the QEMU console](evidence/browser-qa/run-console-mikrotik-chr-1.png), [the Dynamips runtime](evidence/browser-qa/run-dynamips-started.png), and [snapshot restore](evidence/browser-qa/run-snapshot-restored.png).

## Frontend fixes

- Node right-click menus now retain the node selection. The SVG canvas previously cleared selection when the right-click target was a node’s `<rect>`, so it displayed the generic canvas menu. The canvas now clears selection only for empty-canvas rectangles. The corrected node menu shows start, stop, reload, suspend where supported, configure, and other node actions.
- Template forms now provide defaults for fields the controller copies directly to new nodes. Docker, QEMU, and VPCS defaults were part of the uncommitted changes found at handoff. Dynamips templates now save the runtime values needed by the controller, including boolean and string options, slot defaults, and null for an empty chassis.
- Dynamips template nodes use names restricted to letters, numbers, dot, underscore, and hyphen. The generated display name is compacted before the template-add request.
- A missing project README is treated as empty content by the UI. The controller returns 404 until the README is written; editing and saving it then succeeded.

`SimulatorWorkspace.tsx` remains 1,355 lines and `App.tsx` remains 779 lines; neither file grew. No Rust files were edited for this QA task.

## Browser errors and failed requests

There were no uncaught JavaScript exceptions on the successful paths. The captured unsuccessful responses were:

- `GET /v3/projects/{id}/files/readme.md` returned 404 for a newly created project with no README. The UI displayed an empty editor and the write succeeded.
- An unauthenticated request for `/settings` returned 401 and redirected to the login route as expected.
- The first QEMU template-add attempt returned 404 because the disk image had not yet been copied into the isolated image directory. After copying the image into `~/.cache/bq-d/images/qemu`, the QEMU node was added and started.
- Early Dynamips template-add attempts exposed the issues below. After the frontend defaults and image path were corrected, the node was added and its lifecycle actions succeeded.

## Backend issues observed (not edited here)

1. **Sparse Dynamips templates fail during node creation.** Reproduce by creating a minimal `dynamips` record through `POST /v3/templates` with `name`, `template_type`, `platform`, `image`, `memory`, and `nvram`, then call `POST /v3/projects/{project_id}/templates/{template_id}` with a valid router name. The map-add route builds a node with null values for absent template fields and returns 422 while deserializing runtime options (observed first as “invalid type: null, expected a boolean”, then as a missing string after the booleans were supplied). UI-created templates now save defaults to avoid this path; API-created or older sparse templates can still reproduce it. See [`src/api.rs`](../src/api.rs:2638) and its Dynamips option deserialization at [`src/api.rs`](../src/api.rs:3464).
2. **Dynamips image basenames are resolved against the controller working directory.** With an IOS image present under the configured Dynamips image directory, create a template whose `image` is only the basename and add it to a project. The request returns 404 because `DynamipsRuntime::resolve_image` canonicalizes the supplied string without first joining it to the configured image directory. An absolute path inside the isolated image directory worked. See [`src/dynamips.rs`](../src/dynamips.rs:1297).

## Not verified

- The QEMU console accepted input, but no guest response appeared on its serial stream during this browser run; guest-side serial interaction remains unverified.
- No GNS3 registry appliance import, catalog image download, QEMU upload, or Cloud/TAP host-interface connection was attempted.
- Dynamips was exercised with the available local 0.2.25 binary. Other Dynamips platforms and appliance configurations were not tested.

## Final checks

- `npm run build --prefix frontend` passed.
- `npm test --prefix frontend -- --run` passed: 94 tests in 11 files.
- `npm run typecheck --prefix frontend` passed.
