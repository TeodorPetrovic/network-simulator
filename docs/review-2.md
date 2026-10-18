# NET / BUILTIN review

Read-only review against the requested GNS3 3.1 tree. Rust references below point to the implementation as read during this review. Findings are ranked by severity; “verified by reading” means the failure path is directly visible in the code, not that it was exercised live.

## Verified by reading

### P1 — Capture download trusts a client-supplied filesystem path

[`src/api.rs`](../src/api.rs:3048) keeps caller-supplied `capture_file_path` and `capture_file_name` on link creation because it only supplies defaults. [`link_capture_response`](../src/api.rs:5080) later opens the stored path for `/capture/file` after checking only that it is a file. A caller can create a valid link with `capture_file_path: "/etc/passwd"`, then download that file through the capture route; the link need not be capturing. The capture-start route’s filename check does not protect this create-link path. This exposes files readable by the service account when the API is reachable by the caller.

### P1 — Newlines in capture filenames reach the privileged uBridge command stream

[`safe_capture_file_name`](../src/api.rs:5198) accepts newlines because it checks only the basename, length, and dot entries. [`escape_ubridge_arg`](../src/ubridge.rs:751) escapes quotes and backslashes but leaves newlines intact, and capture paths are interpolated into a line-oriented command at [`src/ubridge.rs`](../src/ubridge.rs:497). A filename containing `x\nhypervisor stop\n` therefore places a separate command line in the request written to uBridge. The upstream control client frames commands by newline ([`ubridge_hypervisor.py`](/tmp/gns3-server-3.1/gns3server/compute/ubridge/ubridge_hypervisor.py:184)). Because the uBridge process has the network capabilities needed by the controller, command execution here can disrupt the shared relay and host networking. This path was not exercised against the binary during this read-only review.

### P1 — NAT creation bypasses GNS3’s configured interface restriction

[`normalize_nat_ports`](../src/builtin/mod.rs:701) accepts `nat_interface` or the first supplied `ports_mapping` interface on direct node creation ([`create_builtin_node`](../src/api.rs:2010)). The link path then attaches a non-bridge interface as a uBridge raw Ethernet NIO ([`src/ubridge.rs`](../src/ubridge.rs:317)). A request can therefore create a NAT node on an arbitrary existing host NIC and connect a guest to it, enabling host-L2 packet injection and capture with the privileged uBridge binary. GNS3’s [`Nat` constructor](/tmp/gns3-server-3.1/gns3server/compute/builtin/nodes/nat.py:36) selects the configured default NAT interface and enforces `allowed_interfaces`; this port has no equivalent allowlist or existence check. Cloud intentionally offers host-interface NIOs too, so its raw-NIO capability is separately an exposure to consider when the controller is bound to an untrusted network.

### P2 — Link impairments are applied to both relay endpoints

[`attach_runtime_link_endpoint`](../src/api.rs:3296) passes the full filter map to each endpoint’s uBridge, including QEMU and Dynamips paths ([`src/api.rs`](../src/api.rs:3377), [`src/api.rs`](../src/api.rs:3425)). For a packet crossing both endpoint relays, this applies the impairment twice: requested delay accumulates, and independent packet-loss/corruption probabilities compound. GNS3’s relay path chooses one filter node and sends the filter map only to that endpoint ([`udp_link.py`](/tmp/gns3-server-3.1/gns3server/controller/udp_link.py:175)).

### P2 — `frequency_drop` is advertised and accepted, then rejected by uBridge

The API advertises `frequency_drop` and accepts values in [`validate_link_filters`](../src/api.rs:4807), but [`apply_link_filters`](../src/ubridge.rs:435) permits only `delay`, `packet_loss`, `corrupt`, and `bpf`. A normal every-Nth-packet filter consequently fails on attach or update; the special suspended-link branch is the only path that emits a `frequency_drop` command. On update, `reset_packet_filters` runs before the unsupported-kind check, so a failed request can also remove the previously working rules while the stored link still reports them.

### P2 — Failed active-link creation can leave a QEMU or Dynamips NIO attached

For QEMU, [`attach_runtime_link_endpoint`](../src/api.rs:3377) attaches the QMP NIO before creating its uBridge endpoint; Dynamips does the same at [`src/api.rs`](../src/api.rs:3419). If the uBridge attach fails, the current endpoint is not added to `create_link`’s `attached` rollback list ([`src/api.rs`](../src/api.rs:3093)), so the rollback skips detaching the runtime NIO. A failed link request can leave an extra QEMU NIC or Dynamips UDP NIO behind even though no link object was persisted.

### P2 — Link deletion races with node start and can orphan a relay

There is no shared lifecycle/link-operation lock. `delete_link` detaches runtime endpoints and only then deletes the stored row ([`src/api.rs`](../src/api.rs:5213)); node start independently reads stored links and reattaches them ([`src/api.rs`](../src/api.rs:3584)). If start reads the link before deletion, but attaches after deletion has detached it and before the row is removed, the delete can remove the row while start leaves a newly attached uBridge endpoint/NIO with no topology record to clean it up. The reverse create-versus-stop window also exists because link creation attaches before persistence ([`src/api.rs`](../src/api.rs:3093)), while stop enumerates only persisted links before stopping the runtime ([`src/api.rs`](../src/api.rs:3655)).

### P2 — Crash recovery treats a surviving uBridge as a fresh, empty daemon

After a controller crash, an orphan uBridge can still own the socket and named endpoint bridges. [`ensure_started`](../src/ubridge.rs:640) returns immediately when that socket accepts a connection, without recovering the child handle or rebuilding the in-memory `link_bridges` map. Startup reconciliation then tries to create deterministic bridge names that already exist ([`src/api.rs`](../src/api.rs:143), [`src/ubridge.rs`](../src/ubridge.rs:315)); those reattachments fail. Shutdown cannot stop the orphan because `daemon` is `None` and the in-memory endpoint map is empty ([`src/ubridge.rs`](../src/ubridge.rs:543)). Graceful shutdown is handled; abrupt termination is not.

### P2 — UDP ports are probed, not reserved

[`available_udp_port`](../src/ubridge.rs:789) binds port zero, reads the assigned port, then drops the socket on return. `reserve_pair` allocates several such ports this way ([`src/links/mod.rs`](../src/links/mod.rs:108)). Another process or concurrent request can claim a selected port before uBridge, VPCS, or Dynamips binds it; the same freed port can also be allocated again. The resulting attach/start fails or different link NIOs collide. GNS3 uses its compute port manager to track allocated UDP ports.

### P2 — Switch forwarding database has no size limit

[`forward_frame`](../src/builtin/mod.rs:896) learns every distinct unicast source MAC into `ForwardingNode.learned` and expires entries only when another frame arrives, after 300 seconds ([`src/builtin/mod.rs`](../src/builtin/mod.rs:907)). A connected guest can continuously send frames with spoofed source MACs to grow the map without bound and exhaust controller memory. The upstream Ethernet switch delegates learning to the kernel bridge rather than maintaining this unbounded Rust map.

## Suspected; needs runtime reproduction

### P2 — VPCS startup inspection error may orphan its forked PC process

If [`child.try_wait()`](../src/vpcs.rs:462) returns an I/O error during startup, `?` exits before wrapping the child in `ManagedVpcs` or calling `kill_process_group`. `kill_on_drop` only covers the tracked daemon, while this file documents that VPCS forks the PC into a separate session ([`src/vpcs.rs`](../src/vpcs.rs:61)). The forked process may outlive this failed start until startup orphan cleanup runs on a later controller launch. The OS error path and resulting process tree were not reproduced.

## Test gaps

- The topology API test stores `suspend` and filter metadata but does not apply filters to a live link ([`src/api.rs`](../src/api.rs:6766)). The handoff also says live non-empty filters and packet-capture start/stop/stream were not exercised ([`HANDOFF.md`](../HANDOFF.md:46)). There is no route test covering caller-supplied capture paths or newline filenames.
- Existing live checks cover traffic, VLAN access isolation, hub forwarding, Cloud UDP, and NAT through `virbr0`; they do not cover controller-crash recovery or concurrent create/delete/start/stop requests ([`HANDOFF.md`](../HANDOFF.md:24), [`HANDOFF.md`](../HANDOFF.md:81)).
- Switch QinQ/dot1q behavior has unit tests, but the documented live switch test exercises access-VLAN isolation, not QinQ trunk interoperability ([`src/builtin/mod.rs`](../src/builtin/mod.rs:1062), [`HANDOFF.md`](../HANDOFF.md:81)).

## Fix status

I verified each finding against the implementation before changing it. I found no factual disagreements with the review. The VPCS item was labeled suspected; the startup inspection error branch is present and now cleans up the process tree, though I did not reproduce an operating-system `try_wait()` error itself.

1. **Capture paths — fixed.** Link creation clears client-supplied capture metadata, link updates do not accept capture paths or names, capture start derives a filename under the project capture directory, and file/stream routes verify the stored path resolves to the expected canonical file inside that directory. HTTP and path tests cover untrusted create/update fields, path traversal, and symlinks.
2. **uBridge command injection — fixed.** The command writer rejects control characters, interface and BPF values are validated, capture paths are checked, and `escape_ubridge_arg` percent-encodes controls as defense in depth. A unit test rejects the exact `x\nhypervisor stop\n` input.
3. **NAT/Cloud host interfaces — fixed.** Attach and node create/config paths validate that mapped Linux interfaces exist. NAT accepts its configured default (`virbr0` by default) or an explicit `allow_host_interfaces` entry. Cloud rejects loopback and detected default-route interfaces unless explicitly listed. The node setting is documented in `HANDOFF.md`; `NETWORK_SIMULATOR_ALLOW_HOST_INTERFACES` is also available as an operator-wide comma-separated allowlist. Unit and HTTP tests cover missing, disallowed, and allowlisted interfaces.
4. **Filter endpoint selection — fixed.** A persisted endpoint selector sends link impairments to one relay only; a unit test checks the selected and unselected endpoint payloads.
5. **`frequency_drop` and filter replacement — fixed.** uBridge accepts `frequency_drop`; it validates the complete replacement before resetting current filters. A fake-socket unit test proves invalid updates leave the working command set intact and verifies the frequency-drop command.
6. **Failed QEMU/Dynamips attach rollback — fixed.** Relay attach failure detaches a newly attached runtime NIO; existing NIOs remain attached if a filter update fails on an existing relay. The rollback helper tests both paths; the failure path was not exercised against live QEMU or Dynamips.
7. **Lifecycle races — fixed in the controller.** A project-scoped async lock covers link create/update/delete, node lifecycle and reattach operations, project cleanup, and capture operations. Unit coverage checks same-project serialization and independence across projects. No live race stress test was run.
8. **uBridge crash recovery — fixed by teardown and rebuild.** Startup stops an unowned daemon answering on the existing socket, clears stale endpoint/TAP maps, and starts a clean daemon before reconciliation. Repeated endpoint attachment and built-in switch-port attachment are idempotent. The stop protocol has a fake-socket unit test; controller-crash recovery was not restarted live.
9. **UDP port allocation — fixed for controller-managed links.** The uBridge runtime keeps an owner-scoped port registry; create/startup paths reserve or claim persisted ports and link/project deletion releases them. Unit coverage checks concurrent unique reservations, idempotent claims, and owner isolation.
10. **Switch forwarding table — fixed.** The table is capped at 8192 entries, evicts the oldest learned entry, and ages entries on a timer. Unit tests cover the cap, eviction, and expiry.
11. **VPCS startup process cleanup — fixed.** A `try_wait()` error now kills and waits for the process group and its forked children before returning. Unit tests cover failed-start tree cleanup and managed-process drop cleanup; the actual `try_wait()` error was not induced.

### Verification

- `cargo test --locked`: **124 passed, 0 failed, 15 ignored**. `rustfmt` checks passed for `src/links/mod.rs`, `src/ubridge.rs`, `src/builtin/mod.rs`, `src/vpcs.rs`, and `src/api.rs` (with child modules skipped to preserve their owners' formatting). `git diff --check` passed.
- Live checks passed with the local capability-enabled uBridge 1.3.0, VPCS 0.8.5, Docker 29.6.2, and `alpine:latest`: Docker↔Docker and Docker↔VPCS uBridge bridges; VPCS↔VPCS UDP NIOs; Cloud UDP NIO; controller Alpine↔VPCS link; built-in hub traffic and cleanup; built-in switch VLAN isolation; VPCS template link creation; and Docker HTTP console bridging.
- Two live checks did not pass. The NAT/Docker ping test received no ARP response from `192.168.122.1`; `virbr0` existed but was `DOWN` even though `virsh net-list` reported the default network active. The CHR mixed QEMU/VPCS/Docker test failed twice on the Alpine ping to `10.0.2.1`, while the CHR ping to its peer and VPCS ping shown in the test output succeeded. These live failures are recorded as host/test-topology results and are not claimed as passing verification.
- Live controller crash recovery, live concurrent-operation races, live non-empty filters, and live packet capture remain unverified; their implementation paths have unit or HTTP test coverage as listed above.
