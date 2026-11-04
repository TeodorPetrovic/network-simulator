# Agentic E2E layer (e2e.com)

Extra layer next to the Playwright suite in `e2e/`. Install: `bun install`.

Start a throwaway server from the repo root (loopback => auth off):
`NETWORK_SIMULATOR_BIND=127.0.0.1:3180 NETWORK_SIMULATOR_DB=/tmp/agent-e2e.sqlite ./target/debug/network-simulator`

- Scripted test (no model needed): `E2E_BASE_URL=http://127.0.0.1:3180 bunx e2e run --config e2e.config.ts`
- Bug bash (needs a model): set `agents.default.model` in `e2e.config.ts` to an AI SDK model, then
  `bunx e2e explore "<goal>" --config e2e.config.ts` (or `bun run explore`).
