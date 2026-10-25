# Project map UI parity checklist

Status is measured against the current MAP implementation. **Present** means an interactive implementation exists; **partial** means an interactive subset exists or the UI is waiting on a controller endpoint; **missing** means there is no user-facing implementation yet. GNS3 reference behavior is read from `/tmp/gns3-research/gns3-web-ui/src/app/components/project-map/**`, topology-summary, snapshots, web-console-full-window, and `src/app/cartography`; upstream code is not copied.

## Project map menu and workspace

- [x] Present — project title/status, back navigation, connection state, resource monitor.
- [x] Present — device library with template filter, grouped cards, drag/drop and add action.
- [x] Present — topology panel with device state, interfaces, link count, drawings count, and event cursor.
- [x] Present — map menu for project lock, grid/snap, drawings, snapshots, log, screenshot, and help.
- [x] Partial — project lock toggle calls the controller lock routes and blocks map interaction; the current controller records lock metadata without enforcing it in runtime operations.
- [x] Present — grid visibility and snap-to-grid controls.
- [x] Present — zoom in/out, fit-to-topology, zoom to selection, explicit reset, and keyboard zoom.
- [x] Present — canvas pan, selection box, modifier multi-select, and group dragging.
- [x] Present — keyboard help and select/delete/zoom/undo shortcuts.
- [x] Present — screenshot dialog with SVG export and browser canvas PNG export.
- [x] Partial — snapshots list/create/restore/delete dialog calls controller routes; restore changes saved topology but does not restore runtime processes.
- [x] Present — log console subscribes to the live project notification stream.
- [x] Present — topology summary text/state filters and filtered node list.

## Node interactions and menus

- [x] Present — select, drag, rename, configure, duplicate, delete, start, stop, suspend, reload, and primary console actions.
- [x] Partial — auxiliary console entry is visible and reports unavailable because no auxiliary console transport exists in this controller.
- [x] Partial — isolate/unisolate actions call `/v3/projects/{project_id}/nodes/{node_id}/isolate|unisolate`; unavailable responses are shown inline.
- [x] Present — node lock/unlock updates the node and blocks map dragging while locked.
- [x] Partial — symbol picker calls `/v3/symbols` and stores the selected symbol; the canvas still uses type icons because the controller does not serve resolvable symbol files. Node label placement is draggable and persisted.
- [ ] Missing — rendering the chosen symbol asset on the topology canvas.
- [x] Present — selection box, modifier multi-select, and group dragging.
- [x] Present — bulk start, stop, suspend, reload, console, and delete actions.
- [x] Present — unavailable API errors surface in the map status area or dialog.

## Link creation and interactions

- [x] Present — draw-link toolbar mode, node interface picker, direct port selection, and link creation.
- [x] Present — link selection, endpoint summary, delete, and context menu.
- [x] Partial — link color and width editor; endpoint label editing is not included.
- [ ] Missing — editable labels for individual link endpoints.
- [x] Partial — suspend/resume calls link update and reports controller availability.
- [x] Partial — filter editor and available-filter discovery use `/v3` link routes; runtime filtering behavior still needs an end-to-end controller check.
- [x] Partial — capture start/stop, Web Wireshark launch, stream open, and stream URL copy use `/v3` routes; runtime capture behavior still needs an end-to-end controller check.
- [x] Capture Viewer — classic pcap stream/file inspection, display filters, packet details/hex, floating topology viewer, and capture badge are available in browser; pcapng and advanced protocol dissection are not yet supported.
- [x] Present — link menu shows clear error states when API operations fail.

## Drawings

- [x] Present — existing sanitized SVG drawings are displayed; embedded raster images are accepted while external references are removed.
- [x] Present — create and edit rectangle, ellipse, line, text, and embedded image drawings.
- [x] Present — select, move, edit-size, and delete drawing objects.
- [x] Present — drawing color, stroke, fill, text, and image controls.
- [x] Partial — drawing z-order controls persist `z_index`; visual stacking is applied by the map renderer.

## Validation coverage

- [x] Present — Vitest coverage for interface/drawing/selection helpers, drawing sanitizer, API routes, snapshot/link/symbol unavailable states, screenshot dialog, point-to-point link creation, node context menu, and bulk multi-select actions.

## MAP implementation notes

The MAP UI uses controller `/v3` routes for snapshots, symbols, project lock, node isolation, link filters/capture, drawings, and topology updates. The API workstream has added these routes, but its Rust build is currently blocked by unresolved helpers in the NET-owned link changes, so end-to-end route verification is pending. Unsupported or failed calls remain visible as clear unavailable states; no backend behavior is faked in the frontend.
