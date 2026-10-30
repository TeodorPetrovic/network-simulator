# TODO (compare against GNS3 3.1 server + web-ui)

## In flight (Codex gpt-6-luna agents)
- [ ] Canvas/node parity: switch shows 1 port (GNS3: 8), right-click Configure on some node types, sidebar drag/drop, context menus, drawings, labels
- [ ] Backend API parity vs gns3server routes -> docs/api-parity.md
- [ ] Frontend pages parity: projects, templates (all types), appliances, computes, users/ACL, settings, toolbar, console windows

## To verify after merge
- [ ] CHR (MikroTik) and Cisco 2600/2691/3660 templates present; `image_missing` shown
- [ ] Four-quadrant drag, snap off by default (e2e/canvas-move.test.mjs)
- [ ] Zoom, pan, rubber-band, keyboard shortcuts, double-click configure in a real browser

## Build speed
- [ ] Blaze (indexable-inc/blaze) installed in ~/.cargo/bin but blocked: AppArmor
      kernel.apparmor_restrict_unprivileged_userns=1 -> needs sudo sysctl or an AppArmor profile for ~/.cargo/bin/blaze
- [ ] Interim: cargo -j1 with shared CARGO_TARGET_DIR

## Hygiene
- [ ] After each agent batch: df -h /, rm worktree target/ dirs, stale ~/.cache/ns-target-*
