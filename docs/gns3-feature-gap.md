# GNS3 GUI feature gap audit

Compared this port's `frontend/src` and `src/api` with the GNS3 GUI v3.1.0a7 feature areas requested for this audit. Release records were fetched from [gns3-gui v3.1.0a7](https://api.github.com/repos/GNS3/gns3-gui/releases/tags/v3.1.0a7) and [gns3-server v3.1.0a7](https://api.github.com/repos/GNS3/gns3-server/releases/tags/v3.1.0a7). The GUI release specifically includes #3847, “Remove deleted project from recent projects immediately.” The statuses below describe code paths present in this repository, not equivalence to every desktop workflow.

| GNS3 feature | Status here | Note |
|---|---|---|
| Main window menus / project actions | partial | `WorkspaceToolbar` has a project-map dropdown for preferences, grid, drawings, snapshots, log, screenshot, and information. It is not the desktop main-window menu system. |
| Preferences dialogs | partial | Preferences route exposes server settings and preference pages, but desktop-specific local-client and operating-system integration settings are not represented. |
| Node context menu | partial | Start/stop/suspend/reload, console, configure, symbol, lock/isolate, duplicate, delete are present. Menus are web-context menus and some node-specific desktop actions are absent. |
| Link context menu | partial | Link properties, filters, suspend/resume, capture entry, and delete are present. |
| Topology summary | partial | Sidebar provides searchable/status-filtered node summary; no comparable full desktop statistics/overview workspace was found. |
| Console | partial | Console dock supports runtime consoles and auxiliary console; it is browser terminal UI, not native terminal integration. |
| Packet capture | partial | Link context opens capture settings; backend has capture routes. Capture is link/interface scoped and does not reproduce all desktop Wireshark integration. |
| Snapshots | done | Create/list/restore/delete flows are wired in `MapDialogs` and controller snapshot routes. |
| Appliance wizard / manager | partial | Registry lookup, appliance versions, image assignment/upload, and custom appliance creation exist; installation/runtime-specific desktop steps are not all represented. |
| Compute management | partial | Compute page lists registered computes and runtime status. Its UI explicitly states remote compute registration/removal and server start/stop routes are unavailable. |
| Drawing tools | partial | Rectangle, ellipse, line, text, and image create/edit flows plus order controls exist. This is a smaller web canvas workflow than the desktop drawing toolset. |
| Annotations | partial | Text and image drawings provide annotations; richer desktop annotation metadata and editing affordances were not found. |
| Recent projects after delete (#3847) | partial | Both project list implementations remove the deleted item from their in-memory list after successful DELETE. There is no distinct persisted “recent projects” list in `frontend/src`; add regression coverage for immediate list removal. |

## Things we do worse or wrongly

- **Duplicate project list implementations:** `AppProjectsIndex.tsx` is used by `App.tsx`, while the typed `/projects` route has a separate `ProjectsPage.tsx`. They have different action sets and state handling, so a fix can land in one list and not the other.
- **Compute management is read-only:** `ComputesPage.tsx` lists computes and local runtime status, and its own copy says registration/removal and server start/stop need routes that are absent.
- **Some desktop integrations are not web-portable:** `ConsoleDock.tsx` is browser-based, and `MapDialogs.tsx` exposes capture setup rather than desktop Wireshark launching. This is an observed surface difference, not a claim that the desktop launch succeeds on every host.
- **API surface is intentionally incomplete:** the Axum router in `src/api/mod.rs` returns 404 for unmatched `/v3` routes. This means unsupported GNS3 endpoints fail as not found rather than being transparently proxied to a stock controller.
- **Authorization token handling:** `frontend/src/api/client.ts` removes the legacy localStorage token at module load and stores the active token in sessionStorage, then sends it as a Bearer header. That limits persistence to a tab session, unlike a desktop client credential store; browser XSS can still access sessionStorage, so this is not an XSS boundary.

## Three selected low-risk gaps

1. **Immediate project-list update on delete (#3847):** frontend-only; both list paths already update state after successful deletion. Add regression tests to pin this behavior in each UI.
2. **Keep existing project rows visible if refresh fails:** frontend-only; `ProjectsPage.refresh()` currently replaces the list with an empty array on request failure. Preserve the last successful list and show the error.
3. **Expose capture action clearly in the link menu:** frontend-only; retain the existing capture dialog route while using a single, explicit “Packet capture…” entry (the current menu has both a generic properties/filter entry and a capture entry that open the same dialog).

## Top 10 gaps by user impact

1. Remote compute registration/removal and connection editing are absent.
2. No complete desktop main-window menu and command surface.
3. No native Wireshark launch/hand-off workflow.
4. Preferences do not cover desktop-local/OS integrations.
5. Node context menus omit some desktop runtime-specific operations.
6. Topology summary is narrower than the desktop overview and statistics workflow.
7. Appliance installation is split across web registry, image library, and templates rather than the full desktop wizard flow.
8. Console behavior is browser-based and lacks native terminal integration.
9. Annotation/drawing workflows are a reduced subset of desktop tools.
10. The separate project list implementations can diverge, including around delete/recent-list behavior.
