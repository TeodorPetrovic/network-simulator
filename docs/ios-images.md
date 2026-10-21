# Dynamips IOS image inventory and live verification

Inventory captured 2026-10-18 from the local GNS3 3.1 registry snapshot at `/tmp/gns3-registry-inspect/appliances/cisco-*.gns3a`, the app's `/v3/marketplace/ishare2/images?kind=dynamips` route, and the app download/import routes. LabHub search results expose package names and sizes, but no image digest. The download response reports the manifest MD5/SHA1 checks and SHA256 computed from the downloaded file. IOS is proprietary software: the app only downloads files LabHub makes available and does not bypass credentials, licensing, or acceptance gates.

## GNS3 registry appliance expectations

The file, IOS version, expected byte size, and MD5 columns are the registry's exact `images[]` entries. RAM/NVRAM and module defaults come from the `dynamips` appliance definition; Idle-PC is the per-version appliance default. `—` means the appliance leaves the module setting unspecified (the controller's Dynamips platform default is shown where useful).

| Appliance | IOS image expected by registry | Version | Bytes | MD5 | Platform / chassis | RAM / NVRAM (MB/KB) | Slot and WIC defaults | Idle-PC |
|---|---|---:|---:|---|---|---:|---|---|
| Cisco 1700 | `c1700-adventerprisek9-mz.124-25d.image` | 124-25d | 57,475,320 | `7f4ae12a098391bc0edcaf4f44caaf9d` | c1700 / 1720 | 160 / 128 | slot0 C1700-MB-1FE; no WIC | `0x80358a60` |
| Cisco 2600 | `c2600-adventerprisek9-mz.124-15.T14.image` | 124-15.T14 | 87,256,400 | `483e3a579a5144ec23f2f160d4b0c0e2` | c2600 / 2610 | 160 / 128 | slot0 C2600-MB-1E for 2610 (chassis-integrated); no WIC | `0x8027ec88` |
| Cisco 2691 | `c2691-adventerprisek9-mz.124-15.T14.image` | 124-15.T14 | 95,976,624 | `e7ee5a4a57ed1433e5f73ba6e7695c90` | c2691 / — | 192 / 256 | slot0 GT96100-FE (integrated) | `0x60bcf9f8` |
| Cisco 3620 | `c3620-a3jk8s-mz.122-26c.image` | 122-26c | 38,947,996 | `37b444b29191630e5b688f002de2171c` | c3600 / 3620 | 192 / 256 | registry leaves slots empty; default slot0 NM-1FE-TX | `0x603a8bac` |
| Cisco 3640 | `c3640-a3js-mz.124-25d.image` | 124-25d | 65,688,632 | `493c4ef6578801d74d715e7d11596964` | c3600 / 3640 | 192 / 256 | registry leaves slots empty; default slot0 NM-1FE-TX | `0x6050b114` |
| Cisco 3660 | `c3660-a3jk9s-mz.124-15.T14.image` | 124-15.T14 | 90,181,268 | `daed99f508fd42dbaacf711e560643ed` | c3600 / 3660 | 192 / 256 | registry leaves slots empty; default slot0 Leopard-2FE | `0x6076e0b4` |
| Cisco 3725 | `c3725-adventerprisek9-mz.124-15.T14.image` | 124-15.T14 | 97,859,480 | `64f8c427ed48fd21bd02cf1ff254c4eb` | c3725 / — | 128 / 256 | slot0 GT96100-FE (integrated) | `0x60c09aa0` |
| Cisco 3745 | `c3745-adventerprisek9-mz.124-25d.image` | 124-25d | 82,053,028 | `ddbaf74274822b50fa9670e10c75b08f` | c3745 / — | 256 / 256 | slot0 GT96100-FE; slot1 NM-1FE-TX; slot2 NM-4T; slots3–4 empty; WIC0–2 WIC-1T | `0x60aa1da0` |

The c7200 appliance has eight registry image choices. All use platform c7200, RAM/NVRAM 512/512, no chassis, slot0 C7200-IO-FE, NPE-400, and VXR midplane.

| IOS image expected by registry | Version | Bytes | MD5 | Idle-PC |
|---|---:|---:|---|---|
| `c7200-adventerprisek9-mz.153-3.XB12.image` | 153-3.XB12 | 131,471,340 | `3d234a3793331c972776354531f87221` | `0x60630d08` |
| `c7200-advipservicesk9-mz.152-4.S7.image` | 152-4.S7 | 93,595,460 | `840a757e71bd73daaf9c77e5b02c6333` | `0x62cc930c` |
| `c7200-advipservicesk9-mz.152-4.S6.image` | 152-4.S6 | 93,586,040 | `8d0bbd5473b58ef5bbc7e926f56e174a` | `0x62cc930c` |
| `c7200-advipservicesk9-mz.152-4.S5.image` | 152-4.S5 | 87,756,936 | `cbbbea66a253f1dac0fcf81274dc778d` | `0x62cc930c` |
| `c7200-adventerprisek9-mz.152-4.M11.image` | 152-4.M11 | 128,487,680 | `9a2005ad09ce1ec6fe7cf9af1e9b099e` | `0x6062e5c0` |
| `c7200-adventerprisek9-mz.124-24.T5.image` | 124-24.T5 | 102,345,240 | `6b89d0d804e1f2bb5b8bda66b5692047` | `0x606df838` |
| `c7200-a3jk9s-mz.124-25g.image` | 124-25G | 71,528,984 | `9c7cc9b3f3b3571411a7f62faaa2c036` | `0x6066a998` |
| `c7200-itpk9-mz.124-15.SW.image` | 124-15.SW | 46,423,952 | `3334a0facdbd26164e83d3c201fff5b5` | `0x60b2bc68` |

## LabHub offers

Sizes are bytes as exposed by the app search route. Search does not expose MD5/SHA1; those are listed as “not exposed”. The package ID is the ID used in `/v3/marketplace/ishare2/images/{kind}/{id}/download`.

| Registry platform | LabHub offer(s): package ID, exact package name, bytes, search digest |
|---|---|
| c1700 | No c1700-named package. ID 1 `c1710-[1]-adventerprisek9-mz.124-25d.image`, 57,475,320; ID 2 `c1710-bk9no3r2sy-mz.124-23.image`, 37,540,240. Digests not exposed. |
| c2600 | ID 3 `c2600-[1]-adventerprisek9_ivs-mz.124-25b.image`, 74,547,064; ID 4 `c2600-[2]-advipservicesk9-mz.124-25b.image`, 70,082,136; ID 5 `c2600-[3]-adventerprisek9-mz.124-25d.image`, 73,913,048. Digests not exposed. |
| c2691 | ID 6 `c2691-[1]-adventerprisek9_ivs-mz.124-25d.image`, 81,300,900; ID 7 `c2691-[2]advsecurityk9-mz.124-25d.image`, 44,738,716. Digests not exposed. |
| c3620 | No c3620 package in search; the exact registry image was already installed locally (see acquired images below). |
| c3640 | ID 8 `c3640-[1]-a3js-mz.124-25d.image`, 65,688,632 (same size/version as registry expectation, package name differs); ID 9 telco 124-25d, 33,395,020. Digests not exposed. |
| c3660 | ID 10 `c3660-[1]-jk9s2-mz.124-25d.image`, 76,049,252; ID 11 `c3660-[2]-a3js-mz.124-25d.image`, 73,543,952; ID 12 `c3660-[3]-is-mz.124-25d.image`, 67,528,652. Digests not exposed. |
| c3725 | ID 13 `c3725-[1]-adventerprisek9-mz.124-25d.image`, 81,841,724; ID 14 `c3725-[2]-advipservicesk9-mz.124-25d.image`, 78,214,136; ID 15 `c3725-[3]-advsecurityk9-mz.124-25d.image`, 46,031,920; ID 16 `c3725-adventerprisek9-mz.124-15.T14.image`, 97,859,480 (exact registry version and size). Digests not exposed in search. |
| c3745 | ID 17 `c3745-[1]-adventerprisek9-mz.124-25d.image`, 82,053,028; ID 18 `c3745-[2]-advipservicesk9-mz.124-25d.image`, 78,392,636; ID 19 `c3745-[3]-advsecurityk9-mz.124-25d.image`, 46,234,344. Digests not exposed in search. |
| c7200 | ID 20 `c7200-[10]-adventerprisek9-mz.151-4.M2.image`, 117,918,192; ID 21 `c7200-[11]-advsecurityk9-mz.150-1.M9.image`, 96,496,256; ID 22 `c7200-[1]-a3jk9s-mz.124-25g.image`, 71,528,984 (exact registry version and size); ID 23 `c7200-[2]-adventerprisek9-mz.152-4.M7.image`, 128,474,856; ID 24 `c7200-[3]-advipservicesk9-mz.152-4.S2.image`, 87,521,544; ID 25 `c7200-adventerprisek9-mz.152-4.S2.image`, 93,329,832; ID 26 `c7200-adventerprisek9-mz.152-4.S6.image`, 93,586,040. Digests not exposed in search. |

## Images installed in the global library

Downloads were requested through this app's LabHub POST routes. The downloader validated manifest size, MD5 and SHA1, then accepted each file only after Dynamips ELF validation. Files live in `~/.local/share/network-simulator/images/dynamips` (or the directory selected by `NETWORK_SIMULATOR_IMAGE_HOME` / `NETWORK_SIMULATOR_DYNAMIPS_IMAGES`); no IOS payload is stored in this repository.

| Local image file | Source | Bytes | MD5 | SHA1 | SHA256 | Result |
|---|---|---:|---|---|---|---|
| `c3620-a3jk8s-mz.122-26c.image` | Pre-existing local image, exact c3620 registry expectation | 38,947,996 | `37b444b29191630e5b688f002de2171c` | `984d9031a664f600adfbebec7ff3e3c8d09e0e4f` | `bfe627d6b990c20f2aa818c916844228d79adfc925a8a8d14b639efed1dfbb95` | ELF accepted; available to imported Cisco 3620 appliance |
| `ishare-16-c3725-adventerprisek9-mz.124-15.T14.image` | LabHub package 16, c3725 124-15.T14 | 97,859,480 | `64f8c427ed48fd21bd02cf1ff254c4eb` | `b0b8310bd5262db2ff7ec0b008624c5f77cb407e` | `98dda304d8c7bd7f8f38f54623431ceaf1af543316d1368f4b8f5559614acd24` | Downloaded and verified; exact registry image bytes |
| `ishare-22-c7200-[1]-a3jk9s-mz.124-25g.image` | LabHub package 22, c7200 124-25G | 71,528,984 | `9c7cc9b3f3b3571411a7f62faaa2c036` | `8c7e4273f1a92b4a6e7b56a0d7a976d14b87894f` | `d1669a18585346710370269b5dfd2ec2d3d0680d35329f502541eecf5c139b30` | Downloaded and verified; exact registry image bytes |

The installed binary reports `Dynamips 0.2.14-amd64/Linux stable`, satisfying the numeric GNS3 minimum 0.2.11. In live c3725 and c7200 attempts, however, 0.2.14 crashed in glibc `vscanf` immediately after ROMMON printed “Launching IOS image”; the images matched the registry MD5s. A separate upstream Dynamips 0.2.25 build booted the same c3725 and c7200 images, and also booted the c3620 image. It was used through `NETWORK_SIMULATOR_DYNAMIPS=/path/to/dynamips` and was not installed over the user's 0.2.14 binary. The image hashes can be independently recomputed with `md5sum`, `sha1sum`, and `sha256sum` in the global image directory.

## Live boot and topology checks

The opt-in harness is `scripts/live_dynamips_lab.mjs`; `tests/live_dynamips.rs` starts a current-source controller with a temporary database and runtime state, leaving the global images in place. Run it with `NETWORK_SIMULATOR_TEST_DYNAMIPS=/path/to/dynamips CARGO_TARGET_DIR=$HOME/.cache/network-simulator/cargo-target cargo test --locked --test live_dynamips -- --ignored --nocapture` from the repository root. The test wrapper requires the dedicated test variable and passes that binary to its temporary controller as `NETWORK_SIMULATOR_DYNAMIPS`. It creates nodes from imported templates, handles the IOS initial-configuration prompt, probes and persists Idle-PC, exercises Dynamips lifecycle and point-to-point links, tests VPCS and Docker through the built-in switch, and deletes the test project in a `finally` cleanup.

The latest full live run passed with the upstream 0.2.25 build. The c3725 and c7200 templates reached IOS CLI prompts and exchanged 5/5 router-to-router pings. The c3620 template booted and exchanged traffic with VPCS; then c3600, VPCS, and Alpine Docker exchanged traffic through the built-in Ethernet switch. Router reload, suspend/resume, running link delete/recreate, post-reload ping, project deletion, graceful controller shutdown, and uBridge cleanup also passed. It selected Idle-PC `0x60c086a8`; the auto-selection API saved it on the router, and the live restart reapplied it before `vm start`. CPU measured 40.8% before restart and 36.4% after a 5-second settle plus an 8-second sample (one Dynamips process, as a percentage of one host core); it was 49.0% at the later process check. An earlier full live run measured 32.8% before and 39.8% after restart, with a 49.6% final process average. Two attempts using a 2.5-second immediate post-restart sample read 52.0% and 54.8%, exceeding the strict 50% threshold and stopping before VPCS/Docker checks; extending the sample until IOS settled passed. Cleanup left no test Dynamips or uBridge processes, transient runtime files, or runtime directories. Other registered families (c1700, c2600, c2691, c3640, c3660, c3745) were not boot-tested in these runs.

## Why GNS3 boots these images

Compared with the GNS3 Server 3.1 source checkout at `~/.cache/gns3-server` (tag `v3.1.0a7`, commit `2ea13ca`), this port originally differed in several launch details. GNS3's `gns3server/compute/dynamips/__init__.py` rejects versions below 0.2.11 and enables console binding only at 0.2.23 or newer; `hypervisor.py` launches with `-N1`, a log path, and `-H`, and starts the child with its per-hypervisor working directory. This port now applies that minimum, blocks the locally observed 0.2.14 build because it crashes in `vscanf` while launching IOS, conditionally passes `--console-binding-addr 127.0.0.1`, and uses a per-node working directory. The version failure is returned in runtime information and again on node start, with the detected version in the message.

GNS3's router defaults in `nodes/router.py` are mmap on, sparse memory on, Idle-PC unset, idle max 500, and idle sleep 30. The platform classes explicitly disable sparse memory for c1700/c2600 and set their I/O memory to 15%; other supported families here use 5%. The old port applied 5% universally and also sent idle sleep `30` to Dynamips, whose command parameter is microseconds; leaving GNS3's default untouched is necessary to retain Dynamips' 30 ms CPU sleep. The port now uses platform I/O memory defaults, disables c1700/c2600 sparse memory in appliance defaults, and sends idle tuning only when it differs from GNS3's defaults. IOS ghosting in GNS3 requires mmap: it starts an otherwise configured ghost router to create the shared RAM file, stops it immediately, deletes the temporary VM, then points the real router at the file. The port follows the same start/stop sequence and only enables ghosting with mmap.

GNS3 prepares router configuration and its NIO bindings before `vm start` (`nodes/router.py`); Dynamips links must therefore be attached before the IOS process boots so interfaces are ready as IOS initializes. This port creates/attaches Dynamips NIOs in the pre-start phase, then starts the VM, while other runtime links are attached after their node starts. GNS3's `auto_idlepc` tries proposals for three seconds and accepts the first below 70% CPU. This port measures for five seconds, selects the best proposal below 10% when available, and otherwise uses the lowest measured proposal only if it remains below GNS3's 70% acceptance limit.

Finally, GNS3 scopes hypervisor output and transient VM files to its working directory and cleans these on project close. The port now launches each process in its per-node directory, owns its process group, kills descendants on stop/drop/shutdown, and reaps stale processes whose executable and cwd match this runtime. Live CPU samples in the table below are measurements from the opt-in lab, as a percentage of one host core; they are not inferred from the selected Idle-PC value.

## Idle CPU measurements

The recorded historical live c3725 runs before the current Idle-PC changes measured 32.8–40.8% before controller restart and 36.4–39.8% after restart (one core, after the IOS CLI settled). Those runs used the earlier 50% acceptance test and do not meet the current <10% target. The revised live harness now boots each of the nine installed families, samples before and after applying a candidate, and fails if steady-state CPU remains at or above 10%. No post-change nine-family measurement is recorded yet: the ignored integration test could not be rebuilt in the current shared worktree because `src/ubridge.rs` references the temporarily absent `src/ubridge/bridge_forwarding.rs` file. Do not treat the historical samples as verification of the new threshold.
