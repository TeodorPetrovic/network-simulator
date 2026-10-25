# Live network scenarios

`e2e/scenarios/` contains REST and device-console integration tests. Each test starts an isolated simulator with a temporary database and a free loopback port, creates a project, boots real runtimes, creates links, and checks console or packet output. No browser is used.

## Run

```sh
scripts/scenarios.sh
scripts/scenarios.sh e2e/scenarios/vpcs-basic.test.mjs
```

The runner builds `target/debug/network-simulator` with `nice -n 10 cargo build -j2` when it is missing. Tests run sequentially because the local uBridge runtime uses a shared socket. The simulator binary can be overridden with `E2E_BINARY`.

VPCS coverage needs `vpcs` and `ubridge`; routed cases also need `dynamips` and the c3620 IOS image. CHR coverage needs KVM/QEMU and `~/.local/share/network-simulator/images/qemu/mikrotik-chr-7.23.8.qcow2`. Docker coverage needs a working Docker daemon and `alpine:latest`. Tests skip only when those external prerequisites are absent; device or traffic failures remain test failures.

Scenario files:

- `vpcs-basic.test.mjs`: point-to-point ICMP, subnet isolation, switch fanout, and peer-stop behavior.
- `vpcs-routing.test.mjs`: c3620 single-router and static two-router routes, with ping and trace output.
- `mikrotik.test.mjs`: CHR and VPCS ICMP over a linked interface.
- `docker.test.mjs`: Alpine interfaces, Docker-to-Docker and Docker-to-VPCS ICMP.
- `mixed.test.mjs`: VPCS, Docker, CHR, and c3620 attached to one built-in Ethernet switch.
