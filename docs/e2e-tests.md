# Browser end-to-end tests

`e2e/` holds a Playwright (playwright-core) suite run with `node:test`. Each test starts its own
isolated simulator (temp database, loopback bind so auth is off) and its own browser, so tests
do not share state.

## Run

```
cargo build                      # target/debug/network-simulator
cd frontend && npm run build     # frontend/dist/client (served by the simulator)
scripts/e2e.sh                   # whole suite, Chromium
scripts/e2e.sh e2e/devices.test.mjs
E2E_BROWSER=firefox scripts/e2e.sh
```

Playwright location defaults to `~/.cache/qa-tools/node_modules/playwright-core`
(override with `PLAYWRIGHT_CORE`); the simulator binary can be overridden with `E2E_BINARY`.
Install browsers with `node <playwright-core>/cli.js install chromium firefox`.

## Files

- `workspace-ui.test.mjs` – every menu, toolbar button, zoom, rail/flyout, drag and drop, docks, floating console, status bar, shortcuts.
- `devices.test.mjs` – VPCS, switch/hub, NAT/cloud, Dynamips (needs the dynamips binary and IOS images, skipped otherwise), links, group drag, locking, missing-runtime errors.
- `pages.test.mjs`, `pages2.test.mjs` – navigation, projects, computes, Preferences/templates, appliances, access, settings, snapshots, dialogs, layout at 1500x900 and 1100x700.
- `smoke.test.mjs` – minimal start-a-device check.

Every test asserts that the page produced no uncaught errors.
Note: run browsers one after another (shared ubridge socket).
