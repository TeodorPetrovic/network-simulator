# Live network scenarios

`e2e/scenarios/` contains REST and device-console integration tests. Each test starts an isolated simulator with a temporary database and a free loopback port, creates a project, boots real runtimes, creates links, and checks console or packet output. No browser is used.

## Run

```sh
scripts/scenarios.sh
scripts/scenarios.sh e2e/scenarios/vpcs-basic.test.mjs
```

The runner builds `target/debug/network-simulator` with `nice -n 10 cargo build -j2` when it is missing. Tests run sequentially because the local uBridge runtime uses a shared socket. The simulator binary can be overridden with `E2E_BINARY`.

VPCS coverage needs `vpcs` and `ubridge`; routed cases also need `dynamips` and the c3620 IOS image. CHR coverage needs KVM/QEMU and `~/.local/share/network-simulator/images/qemu/mikrotik-chr-7.23.8.qcow2`. Docker coverage needs a working Docker daemon and `alpine:latest`. Tests skip only when those external prerequisites are absent; device or traffic failures remain test failures.

Current runtime limits: the Dynamips launch path exposes one link port per router, so the two-interface c3620 routing cases fail at link creation until the NET-owned multi-interface attachment work is implemented. CHR's QEMU serial console did not produce a login prompt within the 120-second boot window on the local image; the scenario reports this as a failure so it stays visible.

Scenario files:

- `vpcs-basic.test.mjs`: point-to-point ICMP, subnet isolation, switch fanout, and peer-stop behavior.
- `vpcs-routing.test.mjs`: c3620 single-router and static two-router routes, with ping and trace output.
- `mikrotik.test.mjs`: CHR and VPCS ICMP over a linked interface.
- `mikrotik-hotlink.test.mjs`: hot attach, detach, and reattach across both CHR adapters while checking the serial session and uptime remain live.
- `docker.test.mjs`: Alpine interfaces, Docker-to-Docker and Docker-to-VPCS ICMP.
- `mixed.test.mjs`: VPCS, Docker, CHR, and c3620 attached to one built-in Ethernet switch.
