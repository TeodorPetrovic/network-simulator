# Live routing and VLAN exercises

Updated: 2026-10-17

## Available lab

The local controller has a saved, running project named **MikroTik VLAN and routing lab** (`4a39b74f-3c0b-4830-a07b-ecafc527f65f`). Open it from the project list. It contains one MikroTik CHR VM and four interactive Alpine hosts. The Docker and QEMU console WebSockets were both exercised.

The CHR disk is MikroTik RouterOS **7.23.8 long-term**, downloaded from the [official CHR downloads page](https://mikrotik.com/download/chr) and converted to the shared image:

```text
~/.local/share/network-simulator/images/qemu/mikrotik-chr-7.23.8.qcow2
```

The official archive was 45,139,084 bytes and passed ZIP integrity checking. Its SHA-256 is `ac01ec7c092f4cab8136212e8ebfd613e0a37844c406317fa228d1ad446ce62c`. The extracted raw disk is 128 MiB. The lab VM uses 1 CPU, 1024 MiB RAM, and four VirtIO adapters. CHR's free license runs indefinitely and limits upload throughput to 1 Mbps per interface; RouterOS features used in these routing and switching exercises remain available under that tier. See [MikroTik's CHR licensing documentation](https://manual.mikrotik.com/docs/getting-started/routeros-licensing/chr/chr-licensing/).

## MikroTik VLAN switching and inter-VLAN routing

The four QEMU adapters are attached in adapter-number order, so RouterOS sees them as `ether1` through `ether4`:

| CHR port | Lab host | Switch role | IPv4 address |
| --- | --- | --- | --- |
| `ether1` | VLAN 10 host A | Access, PVID 10 | `10.10.10.10/24` |
| `ether2` | VLAN 10 host B | Access, PVID 10 | `10.10.10.11/24` |
| `ether3` | VLAN 20 host A | Access, PVID 20 | `10.10.20.10/24` |
| `ether4` | VLAN 20 host B | Access, PVID 20 | `10.10.20.11/24` |

`bridge1` has VLAN filtering enabled. VLAN 10 and VLAN 20 are both members of the bridge CPU port, and RouterOS VLAN interfaces use `10.10.10.1/24` and `10.10.20.1/24` as their gateways. The configuration is saved in the project's QEMU overlay and survived a real stop/start cycle.

From the Alpine console panels in the UI, run these checks:

```sh
# On VLAN 10 host A: same-VLAN switching
ping -c 2 10.10.10.11

# On VLAN 20 host A: same-VLAN switching
ping -c 2 10.10.20.11

# On VLAN 10 host A: inter-VLAN routing through CHR
ping -c 2 10.10.20.10
```

To verify isolation, temporarily give VLAN 10 host A `192.0.2.10/24` and VLAN 20 host A `192.0.2.20/24` using `ip addr flush dev eth1` followed by `ip addr add <address> dev eth1`. A ping between those addresses must fail because ARP stays within each VLAN. Restore the original addresses and default gateways with `ip addr flush dev eth1`, `ip addr add <address>/24 dev eth1`, and `ip route replace default via <gateway>`; the saved node start commands restore the initial values after a container restart.

Verified live results: both same-VLAN pings received 2/2 replies; the inter-VLAN ping received 2/2 replies with TTL 63; the same-subnet cross-VLAN ping received 0 replies; after the CHR and containers were stopped and restarted, same-VLAN and inter-VLAN pings passed again. RouterOS `/interface bridge vlan print` showed VLAN 10 untagged on `ether1,ether2` and VLAN 20 untagged on `ether3,ether4`, with `bridge1` tagged for both.

## Basic routed-subnet exercise

A separate temporary controller project used three Alpine Docker nodes and two Ethernet links:

| Node | Interface | Address / route |
| --- | --- | --- |
| Router | `eth1` | `10.10.1.1/24` |
| Router | `eth2` | `10.10.2.1/24` |
| Left host | `eth1` | `10.10.1.10/24`, default via `10.10.1.1` |
| Right host | `eth1` | `10.10.2.10/24`, default via `10.10.2.1` |

The router enabled IPv4 forwarding. A left-to-right ping received 2/2 replies with TTL 63, and `ip route get 10.10.2.10` selected `10.10.1.1` as the next hop. This temporary project was deleted after the check.

## Runtime notes

- Allow CHR about 20 seconds to finish booting before running the first ping after a start.
- The CHR image is global, so it also appears in the QEMU template list for new projects.
- The persistent MikroTik lab is currently running to make it ready for UI use. Stop the nodes when finished to release the VM's 1 GiB of RAM.
- The QEMU API now sorts links by adapter number before creating VirtIO devices; this keeps guest port names aligned with the topology after stop/start.
