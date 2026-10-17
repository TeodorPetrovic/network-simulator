# Development handoff

Updated: 2026-10-17

## Project objective

Build a reliable Rust network-lab controller by reverse engineering the GNS3 3.1 server and Web UI. The evidence and scope decisions are in [docs/reverse-engineering.md](docs/reverse-engineering.md); the compact REA report is in [docs/evidence/gns3-web-ui-rea-summary.json](docs/evidence/gns3-web-ui-rea-summary.json).

## Current implementation

- The Axum controller persists projects and topology in SQLite. It exposes a selected GNS3-shaped `/v3` API, atomic topology snapshots, and replayable project events over WebSockets.
- Docker nodes use real local images and containers, lifecycle operations, and Engine bridge networks for Docker-only links. A live integration test has two Alpine containers exchange ICMP over a controller-created link.
- QEMU nodes use local disk images, private copy-on-write disks, QMP lifecycle control, and KVM with TCG fallback. QEMU-only Ethernet segments use a per-link multicast bus; mixed-runtime QEMU endpoints use the shared Linux bridge through TAP.
- VPCS nodes launch the upstream BSD-licensed `vpcs` executable as a managed process, expose its Telnet console through a WebSocket, and forward raw Ethernet frames over local UDP sockets. Shutdown uses the VPCS console's quit path and waits for its forked PC process to exit, avoiding orphan emulator children. VPCS has one adapter and no suspend operation yet.
- Classic IOS and IOSv are separate paths: the Dynamips adapter manages real c7200 hypervisor instances, uses real IOS ELF images, exposes router lifecycle and console operations, and binds its UDP NIO to the VPCS/Dynamips frame switch. IOSv/IOSvL2 disk appliances still use QEMU. The repository does not contain a Cisco IOS image, so a router boot and router-to-VPCS ping have not been verified.
- Running QEMU VMs expose a serial socket that the Rust controller bridges to a WebSocket. The TanStack Start UI attaches xterm.js in a docked, minimizable console area at the bottom of the screen; multiple consoles remain mounted in tabs while hidden. Each runtime-specific console route validates that node type and running state, and QEMU cleanup removes stale serial sockets.
- Docker nodes are created with stdin and a TTY enabled. The `/v3/compute/projects/{project_id}/docker/nodes/{node_id}/console/ws` route attaches to the running container TTY and streams raw input/output to xterm.js; disconnect sends Docker's detach sequence so an interactive container remains running. This uses `script` from util-linux to provide the PTY Docker CLI requires.
- Runtime status reports whether the service account can access `/dev/kvm` and returns the operating-system error when it cannot. The `teodor` account and current session now have `kvm` group access, and a QEMU launch with explicit `-accel kvm` succeeded.
- The browser UI uses TanStack Start in SPA mode with typed project routes and shadcn/ui components (Radix, Tailwind CSS v4). The Rust server serves the built static shell and assets alongside the API; Node is needed to build and develop, not to run the deployed controller.
- The project UI has a topology-first workspace: central draggable map, right-side device library and topology summary, template drag-and-drop with automatic device names, rename and lifecycle actions from the selection panel/context menu, explicit interface-port connection mode, and start/stop-all actions. The resource popover in the top-right reads host CPU, memory, load averages, and controller RSS/thread counts from `/v3/resources/monitor` and Linux `/proc`; values are unavailable on systems without those proc files.
- The image catalog searches the GNS3 registry for `.gns3a` appliance definitions and iShare2/LabHub. GNS3 definitions import as actual appliance JSON. Selected QEMU disk and single-file Dynamips IOS entries can download into the global image folders with the LabHub size and MD5/SHA1 checks; IOS images also pass Dynamips ELF validation. It does not run iShare2's EVE-NG installer. IOL and archived QEMU packages remain unsupported.
- Mixed-runtime links create one shared uBridge Linux bridge. The bridge is brought up at creation; QEMU uses TAP, Docker uses a veth moved into the container network namespace, and VPCS/Dynamips use uBridge UDP-to-TAP endpoints. Link creation rejects running endpoints so adapters cannot silently miss a link. Docker nodes receive `NET_ADMIN` and `NET_RAW` for interface configuration and packet tools. The operator granted `cap_net_admin,cap_net_raw=ep` to the user-local uBridge binary; `/v3/runtimes` now reports uBridge available. Both the direct Alpine↔VPCS datapath test and a real controller API test passed ICMP; the API test created, started, linked, and deleted real nodes and verified one reply with no loss.
- QEMU mixed-link arguments are sorted by configured adapter number so the guest's NIC order remains stable across starts. This was verified with a four-adapter CHR guest attached to separate Docker/uBridge segments.
- Base images are shared across all projects. Default QEMU and Dynamips directories are `$XDG_DATA_HOME/network-simulator/images/{qemu,dynamips}` or `~/.local/share/network-simulator/images/{qemu,dynamips}`; `NETWORK_SIMULATOR_IMAGE_HOME` moves both, and each runtime has its own path override. Startup creates the directories. QEMU accepts uploaded disk images validated by `qemu-img` (64 GiB max); IOS upload keeps its ELF validation. Docker images remain in the global Docker Engine store. VPCS and Dynamips binaries can be installed per-user in the app data `bin/` folder and are detected there automatically. Per-node QEMU overlays and hypervisor state remain in runtime state directories.
- Previous `#/projects/<id>` URLs redirect to `/projects/<id>`. The frontend uses the target server's `/v3` API; the supplied upstream UI snapshot calls `/v2`.
- Unsupported node types are rejected. There are no fake runtime implementations for unported hypervisors.

## Build and verification

From the repository root:

```sh
npm ci --prefix frontend
npm run build --prefix frontend
npm test --prefix frontend -- --run
npm run typecheck --prefix frontend
cargo fmt --all -- --check
cargo build --locked
cargo test --locked
```

Latest verification on 2026-10-17: `npm run build` and the frontend TypeScript check pass; all 36 frontend tests pass; `cargo check --locked`, `cargo test --locked` (29 passed, 7 live checks ignored by default), and `git diff --check` pass. The UI was not visually inspected in a browser because the collaborative browser is blocked by this host's AppArmor policy. The controller process already running on port 3080 predates `/v3/resources/monitor` and returns 404 for that route; restart the controller from the current checkout to activate the new resource monitor. The Vite development server is available on port 5173 in the current session.

Earlier live checks passed for the direct real Alpine↔VPCS uBridge test, a real controller API mixed-link test with an ICMP reply and automatic project cleanup, and focused upstream VPCS two-node packet exchange with console shutdown verification. The iShare2 downloader passed a local HTTP streaming test for catalog-provided size and MD5/SHA1; this did not download a vendor image. Earlier live checks passed for iShare2/GNS3 search and import of the actual `cisco-7200.gns3a` into a temporary directory; QEMU upload validated by real `qemu-img` and appeared as a global template; two-node VPCS ICMP and controller/console coverage; Docker Alpine-to-Alpine ICMP; and the real Dynamips hypervisor creating a c7200 and binding/removing its UDP Ethernet NIO. The Dynamips test does not boot IOS; that requires a licensed image supplied by the operator. Mixed QEMU/VPCS packets and Dynamips/IOS packets remain to be tested.

The live CHR routing and VLAN exercise is documented in [docs/network-exercises.md](docs/network-exercises.md). MikroTik CHR 7.23.8 long-term is installed in the global QEMU image library. A saved project (`MikroTik VLAN and routing lab`, ID `4a39b74f-3c0b-4830-a07b-ecafc527f65f`) contains one QEMU CHR and four interactive Alpine hosts. VLAN 10 and VLAN 20 switching, cross-VLAN isolation, inter-VLAN routing, CHR stop/start persistence, and both QEMU serial and Docker console WebSockets passed live checks. The project is left running for UI use. A separate real Alpine router topology routed between two /24 subnets with 2/2 ICMP replies and was deleted after testing.

Earlier end-to-end checks booted a temporary serial-echo guest with KVM and verified guest output and browser terminal input through the controller WebSocket. A real local Alpine container verified Docker WebSocket terminal input/output and remained running after the console detached. QEMU also accepted and ran the multicast netdev configuration in TCG/KVM smoke checks.

Start the combined service with `cargo run` after building the frontend. It listens on `127.0.0.1:3080` by default. For UI development, run `cargo run` and `npm run dev --prefix frontend`; the Vite server proxies API and WebSocket calls to the Rust service.

## Next development work

1. Expand GNS3 compatibility from observed source/API evidence and captured fixtures. Keep `/v2` compatibility decisions explicit rather than assuming the upstream UI and 3.1 server are aligned.
2. Add drawings editing, project import/export, and remaining controller endpoints against upstream behavior. Extend Docker console support with terminal resizing and per-image console settings.
3. Boot a licensed c7200 IOS image through the implemented Dynamips adapter, verify startup config and console, then verify ICMP between that router and a VPCS node.
4. Run a live QEMU/VPCS mixed-segment ping test, including stop/start and cleanup. For Dynamips/VPCS, first upload an operator-provided licensed IOS image, boot the c7200, and verify router-to-VPCS ping. The Docker/VPCS mixed link and capabilities are already verified.
5. Run full VM integration checks with user-supplied images and keep licensing-sensitive vendor images out of the repository.

## Known scope limits

Authentication, terminal resizing, VNC console modes, Dynamips models other than c7200, VMware, VirtualBox, IOU, IOL image downloads, QEMU archive downloads, `.gns3` project import/export, and full `/v3` coverage are not implemented yet. Mixed-runtime Docker/VPCS packet exchange is verified; QEMU/VPCS and licensed Dynamips mixed links remain to be validated. Docker console attach is for the container's main TTY; an image whose main process exits or does not provide an interactive shell will not gain a separate shell console. QEMU serial console attach is implemented; guest-image-dependent interactive boot remains to be verified. See the README roadmap before taking on a new adapter or API area.
