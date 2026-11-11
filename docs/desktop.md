# Desktop app (Tauri 3 alpha)

`desktop/` is a native shell around the web UI. It contains **no simulator runtime**: QEMU, Dynamips,
Docker, VPCS and uBridge keep running on the controller server, exactly as for the browser. The app
only renders the UI and talks to the server's `/v3` API and WebSockets.

```
desktop app (Tauri webview, UI from frontend/dist/client)
        │  HTTPS/HTTP  +  WebSocket   (Bearer token, single-use ws tickets)
        ▼
controller server (Axum, SQLite, runtimes)
```

## How it works

- First launch opens `/connect`: the user types the server (`lab:3080`, `https://lab.example.com`).
  The app probes `<server>/healthz`, stores the origin in `localStorage` and continues to `/login`.
  Recent servers are remembered; **Change server** is on the login page.
- `frontend/src/api/server.ts` is the single place that knows the target server. In a browser it is a
  no-op (everything stays same-origin). In the desktop app (`globalThis.isTauri`) every API path, WebSocket
  URL, capture/Wireshark URL and the "console host" fallback is resolved against the chosen server.
- Auth already used bearer tokens and short-lived WebSocket tickets (no cookies), so cross-origin works
  without weakening it. The token is kept in `sessionStorage`; users sign in again on every launch.
- The controller answers CORS only for an allow-list: `tauri://localhost`, `http(s)://tauri.localhost`, plus
  anything in `NETWORK_SIMULATOR_CORS_ORIGINS` (comma-separated exact origins; `*` is ignored). Credentials
  are not allowed. **The server must be this build or newer** to accept the desktop app.

## Build prerequisites

Tauri 3.0.0-alpha uses the system WebView (WebKitGTK 4.1 on Linux, WebView2 on Windows, WKWebView on macOS).

Debian/Ubuntu:

```sh
sudo apt install build-essential pkg-config libssl-dev libglib2.0-dev libgtk-3-dev \
  libwebkit2gtk-4.1-dev libsoup-3.0-dev librsvg2-dev libayatana-appindicator3-dev patchelf
```

Rust 1.95+ and bun are needed. The crate is its own package (`[workspace]` in its `Cargo.toml`), so the
controller still builds on a headless server without these libraries.

## Commands

```sh
cd desktop
bun install
bun run dev      # starts the Vite dev server and the app (see CORS note below)
bun run build    # builds the frontend, then bundles the app (.deb/.AppImage/.msi/.dmg per OS)
bun run icons    # regenerate icons from icon-source.png
```

Dev mode loads the UI from `http://127.0.0.1:5173`, so the controller needs
`NETWORK_SIMULATOR_CORS_ORIGINS=http://127.0.0.1:5173` set when you run `bun run dev`.

## Status and known gaps

- Verified: the frontend builds, type-checks and its tests pass (server routing, probe, token header,
  wss for https servers); the controller's CORS behaviour is tested and checked with curl preflights;
  `cargo check` of the desktop crate passes against WebKitGTK 4.1 headers.
- **Not yet run:** the native window was never launched (no display on the development host), and
  `tauri build` bundles were not produced. First real run: `bun run dev`.
- `window.open` for extra windows (separate capture window, HTTP consoles) relies on the webview's default
  handling; an opener/multiwindow integration is the next step if it misbehaves.
- Plain-HTTP servers are allowed (lab networks). On Windows the webview origin is `http://tauri.localhost`;
  an `https://` server is recommended outside trusted networks.
- Tokens are not persisted across launches by design; adding keychain storage is a possible follow-up.

## Companion tools (Wireshark, Motty)

Open **Preferences → Desktop tools** (`/desktop`) in the desktop app.

- **Wireshark** – on Windows the app downloads the newest stable installer from wireshark.org, checks its published SHA-256, then runs it (Windows asks for administrator approval). On Linux install it with your package manager. Live captures are streamed from the controller and piped into `wireshark -k -i -`, so Npcap/libpcap is not needed.
- **Motty** – downloaded from its GitHub release (SHA256SUMS checked) into the app data folder. Telnet consoles open directly through `telnet://host:port/`; VNC opens Motty and copies the address.
- **Viewer choice** – consoles, VNC and captures can each use the built-in viewer (xterm, noVNC, built-in capture) or the external tool. External tools connect from your computer, so the device's console port must be reachable on the server; the built-in viewers go through the controller and always work.
- **Pop-out windows** – consoles, captures and Web Wireshark open as native windows (`open_window`), sharing the saved login.

## Installers

`.github/workflows/desktop.yml` builds a Windows NSIS installer and Linux `.deb` + AppImage on every desktop change, and publishes a draft release (with `SHA256SUMS`) for tags named `desktop-v*`.
