# Lab library

Ten ready-made labs live in `labs/`. Each lab is one module that declares its nodes and links, an optional
`configure` step (the commands you would type into the consoles) and a `verify` step that boots the lab and checks
that it actually works.

Labs are built the way a GNS3 user builds them, through the app itself: each lab imports the registry `.gns3a`
appliances (`openvswitch`, `onos`, `webterm`, `frr-docker`, `alpine-docker`, `mikrotik-chr`, `mikrotik-winbox`,
`cisco-3725`) with `POST /v3/marketplace/gns3/appliances/{file}/import`, installs their images (Docker pull, or an
image binding for QEMU/Dynamips), creates the template, and adds devices from it. Per-node settings use the node API
(`PUT /nodes/{id}`: adapter count, environment, start command) and each node's `/etc/network/interfaces` is written
through the node files API, which the container's init shim applies at boot (as GNS3's `init.sh` does).
Ryu and the gateway image have no registry appliance, so they are templates made from a Docker image.

```sh
bun scripts/labs.mjs list                       # what exists
bun scripts/labs.mjs verify 01                  # boot in a throw-away controller, run the checks, tear down
bun scripts/labs.mjs verify all
bun scripts/labs.mjs deploy 03 --start          # create the project in your running controller and start it
bun scripts/labs.mjs configure 06               # re-push router configs to an already running lab
```

`deploy` logs in as `admin` using `data/initial-admin-password.txt` (override with `--user`, `--password-file`, `--token`/`LABS_TOKEN`,
`--server`). Docker images are pulled or built on demand (`labs/docker/services` holds the gateway image).
`verify` runs against an isolated controller with a temporary database and never touches your projects.

| # | Lab | Needs |
| --- | --- | --- |
| 01 | Open vSwitch VLANs and trunking (router-on-a-stick gateway) | `gns3/openvswitch` |
| 02 | Open vSwitch + Ryu OpenFlow 1.3 controller | `gns3/openvswitch`, `osrg/ryu` |
| 03 | ONOS SDN controller managing two OVS switches | `onos`, `openvswitch-management` (≈1.5 GB RAM) |
| 04 | Webterm (Firefox over VNC), web servers and Internet via NAT | `gns3/webterm` |
| 05 | MikroTik CHR managed from the registry WinBox desktop | `mikrotik-chr` (+ local CHR image), `mikrotik-winbox` |
| 06 | MikroTik CHR OSPF triangle with failover | CHR image |
| 07 | Cisco IOS routers with OSPF (Dynamips c3725) | c3725 IOS image |
| 08 | Cisco switches: VLANs, trunks, router-on-a-stick (NM-16ESW) | c3725 IOS image |
| 09 | FRRouting eBGP across three ASes | `frrouting/frr` |
| 10 | Linux gateway: DHCP, DNS and NAT | built `ns-labs/services` |

Labs 05, 06, 07 and 08 boot RouterOS or IOS and take a few minutes. Labs 04 and 10 use the NAT node and expect the
host's libvirt `virbr0` network (their Internet checks are skipped when it is absent).

## Portable project archives

`bun scripts/labs.mjs package all` builds each lab in a throw-away controller and writes
`labs/projects/NN-name.gns3project` (git-ignored: they embed disk images). The archive can be imported on any
machine with **Projects → Import** (or `POST /v3/projects/{id}/import`); the images travel inside it, so nothing needs
to be installed first apart from Docker images, which GNS3 archives never contain (they are pulled by name; the
gateway lab needs `bun scripts/labs.mjs deploy` once, or `docker build labs/docker/services -t ns-labs/services`).

What is baked in so the imported project runs by itself:

| Node kind | How its configuration is carried |
| --- | --- |
| Docker | `start_command` and `/etc/network/interfaces` (stored as `project-files/docker/<node>/etc/network/interfaces`) |
| Dynamips | the IOS image plus the `startup_config` property |
| VPCS | the `startup_script` property |
| MikroTik CHR | a flattened, configured copy of the disk (one per router) embedded as a QEMU image; the node keeps a fixed base `mac_address` so RouterOS keeps its interface bindings |

`package` finishes by importing every archive into a fresh controller **with an empty image library** and running the
lab's own checks against the imported copy (`bun scripts/labs.mjs import-verify <lab>` repeats just that step).
Archive sizes: labs 01-04, 09, 10 are a few KiB; 05 is ~43 MiB, 06 ~129 MiB, 07/08 ~46 MiB.
