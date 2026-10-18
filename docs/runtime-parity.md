# Runtime parity audit

This is the post-implementation audit for the RUNTIME workstream. It compares the writable compute-node fields and runtime behaviors in the checked-out GNS3 3.1 schemas/launch paths with this Rust port. Template-only configurator fields are called out separately. A field marked **stored** is preserved in topology/template JSON but does not imply equivalent launch behavior.

References read: `/tmp/gns3-server-3.1/gns3server/schemas/compute/{qemu_nodes,docker_nodes,dynamips_nodes,vpcs_nodes}.py`, `/tmp/gns3-server-3.1/gns3server/schemas/controller/templates/{qemu,docker,dynamips,vpcs}_templates.py`, and the corresponding `compute/{qemu,docker,dynamips,vpcs}` launch implementations. UI field names were checked in `/tmp/gns3-research/gns3-web-ui/src/app/components/project-map/node-editors/configurator/` and `preferences/`.

## QEMU

### Compute-node schema fields

| Upstream fields | Port status | Evidence in the port |
| --- | --- | --- |
| `name`, `node_id` | present | Node identity is persisted and passed into QEMU runtime state. |
| `usage` | stored | Preserved as node metadata; it does not affect the QEMU command. |
| `linked_clone` | present | Each configured HDA–HDD disk uses a per-node qcow2 overlay when enabled; direct image paths are used when disabled. Removal deletes only overlays and generated state. |
| `qemu_path` | partial | The runtime uses its configured `NETWORK_SIMULATOR_QEMU` binary; the per-node/template path value does not select a separate executable. |
| `platform` | present with constraint | All upstream architecture names validate in schema, but launch requires the selected platform to match the configured `qemu-system-*` binary. |
| `console`, `aux` | partial | The port allocates runtime console/aux endpoints and reports them; caller-specified TCP ports are not honored. |
| `console_type`, `aux_type` | partial | Telnet, VNC, SPICE, SPICE agent, none, and SSH are parsed. QEMU launch arguments support telnet/SSH serial, VNC, and SPICE; SSH is exposed through the same controller WebSocket path as telnet rather than an SSH service. |
| `hda_disk_image`, `hdb_disk_image`, `hdc_disk_image`, `hdd_disk_image` | present with HDA requirement | HDA plus optional HDB–HDD images are resolved from the configured image directory and passed as QEMU drives. The current node-create path still requires an HDA image, so diskless ISO/kernel-only guests are not supported. |
| `hda_disk_interface`, `hdb_disk_interface`, `hdc_disk_interface`, `hdd_disk_interface` | present | `ide`, `sata`, `nvme`, `scsi`, `sd`, `mtd`, `floppy`, `pflash`, `virtio`, and `none` map to QEMU drive/device arguments. |
| `hda_disk_image_backing_file`, `hdb_disk_image_backing_file`, `hdc_disk_image_backing_file`, `hdd_disk_image_backing_file` | present | Explicit backing-file values override the selected source image and resolve from the node directory or configured QEMU image directory before linked-clone overlays are created. |
| `hda_disk_image_md5sum`, `hdb_disk_image_md5sum`, `hdc_disk_image_md5sum`, `hdd_disk_image_md5sum` | present as cached metadata | MD5 checksums are calculated for active disk files and cached beside the node runtime files. Supplied checksum values are not compared as validation. |
| `cdrom_image` | present | Optional CD-ROM image is passed with `-cdrom`; simultaneous HDC and CD-ROM is rejected as upstream does. |
| `cdrom_image_md5sum` | present as cached metadata | The active CD-ROM file's MD5 is calculated and cached beside the node runtime files; supplied values are not compared. |
| `bios_image` | present | Optional custom BIOS is passed as `-bios`; using it together with UEFI is rejected. |
| `bios_image_md5sum` | present as cached metadata | The active BIOS file's MD5 is calculated and cached beside the node runtime files; supplied values are not compared. |
| `initrd`, `kernel_image`, `kernel_command_line` | present | Map to `-initrd`, `-kernel`, and `-append`. |
| `initrd_md5sum`, `kernel_image_md5sum` | present as cached metadata | Active support files have their MD5 calculated and cached beside the node runtime files; supplied values are not compared. |
| `boot_priority` | present | All schema orders (`c`, `d`, `n`, `cn`, `cd`, `dn`, `dc`, `nc`, `nd`) are validated and passed to `-boot order=`. |
| `ram`, `cpus`, `maxcpus` | present | Passed through `-m` and `-smp cpus=...,maxcpus=...`; `maxcpus` is raised to at least `cpus`. |
| `adapters` | partial; NET boundary | Values from 0 through 275 are accepted. NIC attachment/count materialization is in the NET-owned launch loop; when there are no links, that loop currently selects `-nic none`. |
| `adapter_type` | present; NET boundary | All QEMU schema NIC models are accepted and used by the NET-owned NIC launch loop. |
| `mac_address`, `custom_adapters` | pending NET integration | Values are preserved/validated by controller/template code; the QEMU NET hook is marked `TODO(RUNTIME-NET QEMU custom adapter hook)`. Launch currently synthesizes MACs and does not apply per-adapter overrides. |
| `replicate_network_connection_state` | pending NET integration | Value is stored, but carrier state must be applied by NET’s NIC/NIO work; named TODO is beside QEMU’s NIC loop. |
| `create_config_disk` | present with a configured image dependency | The runtime copies `NETWORK_SIMULATOR_QEMU_CONFIG_DISK`, or `config.img` from the configured QEMU image directory, into the node directory and attaches it as a raw HDD. The HDD interface falls back to HDA when unset or `none`. The image must exist and be raw; `config.zip` import/export through mtools remains unsupported. |
| `tpm` | present with host dependency | When enabled, a `swtpm` process and QEMU TPM device arguments are launched. Requires `swtpm`. |
| `uefi` | present with host dependency | OVMF code/vars are located and a per-VM writable vars copy is created. Requires OVMF files discoverable by the runtime. |
| `on_close` | present | `power_off`, `shutdown_signal` (QMP powerdown), and `save_vm_state` (save/load named QMP snapshot) have distinct stop behavior. |
| `cpu_throttling` | present with host dependency | A `cpulimit` child is launched when nonzero, using `NETWORK_SIMULATOR_CPULIMIT` or `cpulimit` on PATH; the current host lacks `cpulimit`, so live throttling was not exercised. |
| `process_priority` | present with OS permission caveat | QEMU child priority is set before launch using the upstream priority mapping; OS permission failures are non-fatal as in upstream. |
| `options` | present with a safe allowlist | Only `-cpu`, `-rtc`, and a small set of non-file-reading toggles are accepted. Unknown flags and controller-owned QEMU settings are rejected, which blocks `-fw_cfg`, direct `-drive`, config files, host chardevs, passthrough filesystems, alternate monitors/serial devices, and privilege/process switches. Imported node data is checked again at launch. |

### QEMU template-only fields and helpers

| Upstream fields/behavior | Port status | Evidence in the port |
| --- | --- | --- |
| Template `default_name_format`, `category`, `symbol` | partial | Runtime template defaults include display/category/symbol data; editable custom-template persistence is handled by the API/template layer. |
| Template `first_port_name`, `port_name_format`, `port_segment_size` | pending NET/UI | They are not used by QEMU launch. NIC naming/segmentation belongs to the NET/UI integration. |
| Template `console_auto_start` | stored only | It is not acted on by runtime launch; console auto-opening belongs to the client/controller flow. |
| Template `mac_address`, `custom_adapters` | pending NET integration | Persisted by template/API paths, but the runtime NIC loop does not apply them yet. |
| Image create/resize/delete helpers | present | The controller routes use the runtime's configured `qemu-img` and node state directory. File names are validated, resize/delete require a stopped VM, and deletion removes the cached checksum. |
| `disk_images_to_reset` | present | Requested HDA–HDD linked-clone overlays and checksum sidecars are removed before replacement overlays are created. |
| Launch console smoke check | live checked | Installed QEMU and `qemu-img` are available; the ignored real QEMU/QMP start-stop smoke test is included. It uses the runtime's automatic KVM/TCG choice. Interactive guest boot depends on a user image. |

## Docker

### Compute-node schema fields

| Upstream fields | Port status | Evidence in the port |
| --- | --- | --- |
| `name`, `image`, `node_id` | present | Image/name are sent to Docker create; node identity is persisted. |
| `console`, `aux` | partial | HTTP, HTTPS, and VNC console ports are allocated and returned; telnet uses the controller's main-TTY WebSocket route. Auxiliary consoles use the dedicated Docker PTY WebSocket route. Caller-selected ports are not honored. |
| `console_type`, `console_resolution`, `console_http_port`, `console_http_path`, `aux_type` | present with host-tool/runtime dependencies | `none` and telnet require no host console tool. HTTP/HTTPS start a host TCP bridge to `docker exec ... busybox nc` on `console_http_port` (default 80), and rewrite container localhost/port references. The configured path and resolution remain node metadata for the console client. VNC uses Xvfb at the requested resolution with x11vnc; it returns a clear unsupported error when either tool is missing. Aux uses a `script` PTY around `docker exec -it` and streams it through `/aux_console/ws`. The live HTTP/AUX smoke check passed with Docker and Alpine. Xvfb/x11vnc are absent here, so only the clear unsupported VNC path was tested. The node configuration route accepts `console_type: none`. |
| `usage` | stored | Preserved as node metadata, with no container launch effect. |
| `start_command` | present | Parsed into argv and supplied to `docker create`. |
| `adapters` | partial; NET boundary | Adapter count is stored and validated; actual interface creation is in NET. |
| `mac_address`, `custom_adapters` | pending NET integration | Custom adapter definitions and MAC syntax are validated/stored, but the named NET hook is needed to configure interfaces/MACs. |
| `environment` | present | Validated `KEY=VALUE` values become Docker `--env` options. |
| `extra_hosts` | present | Validated host mappings become `--add-host` options. |
| `extra_volumes` | present with narrower forms | Per-node persistent host paths are mounted into the container. Upstream’s broader volume configuration forms are not represented. |
| `extra_configs` | missing | Read-only injected configuration files are not materialized or bind-mounted. |
| `startup_config_content` | missing | Not written into a container startup config or node directory. |
| `memory`, `cpus` | present | Nonzero memory and CPU limits are passed to Docker create. |

### Docker template-only fields and console behavior

| Upstream fields/behavior | Port status | Evidence in the port |
| --- | --- | --- |
| Template `default_name_format`, `category`, `symbol` | partial | Runtime and custom templates carry defaults; no Docker-specific launch effect. |
| Template `console_auto_start` | stored only | The controller does not auto-open a console when a node starts. |
| Template `console_resolution`, HTTP settings, `aux_type` | present | HTTP(S) port/path are used by the web-console endpoint and client metadata. Resolution configures Xvfb for VNC. Aux type enables a separate PTY console endpoint. |
| TTY resize | partial | The WebSocket accepts a JSON text control frame `{ "type": "resize", "rows": N, "columns": N }` and calls Docker Engine’s `/containers/{id}/resize` API over the Unix socket. The current frontend does not send that control frame yet. |
| Console attach/detach | present | Raw TTY input/output is attached; disconnect detaches without stopping the container. |

## Dynamips

### Compute-node schema fields

| Upstream fields | Port status | Evidence in the port |
| --- | --- | --- |
| `node_id`, `name`, `dynamips_id` | present with allocator and protocol validation | Controller selects a free ID; router names are validated before hypervisor commands, and every quoted string escapes control characters so line breaks cannot split the Dynamips protocol stream. |
| `platform` | present | `c1700`, `c2600`, `c2691`, `c3600`, `c3725`, `c3745`, and `c7200` are supported. |
| `ram`, `nvram` | present with narrower bounds | Passed to hypervisor. Port accepts RAM 128–2048 MB and NVRAM 64–2048 KB; upstream schema only requires positive values. |
| `image` | present | IOS image is resolved from the Dynamips image directory and passed to `vm set_ios`. |
| `image_md5sum` | missing | IOS checksum is not compared at node launch. |
| `usage` | stored | Preserved as metadata, with no router launch effect. |
| `chassis` | present | Platform/chassis combinations for c1700/c2600/c3600 are validated and sent to the hypervisor. |
| `startup_config_content`, `private_config_content` | present | Both config files are written and passed to `vm set_config`. |
| `mmap`, `sparsemem`, `clock_divisor` | present | Mapped-memory, sparse-memory, and configured clock-divisor commands are included before start. |
| `idlepc`, `idlemax`, `idlesleep` | present | Idle-PC is set; idle max/sleep are applied after VM start. |
| `exec_area` | present | Passed to `vm set_exec_area`. |
| `disk0`, `disk1` | present | Disk sizes are passed to the hypervisor. |
| `auto_delete_disks` | present | When enabled, only matching Dynamips disk/NVRAM/slot/flash/ROM artifacts for this router ID are removed. Generated startup/private config files are kept while the node exists and removed when that node is deleted; remaining virtual disk artifacts are preserved unless `auto_delete_disks` is enabled. |
| `console`, `console_type`, `aux`, `aux_type` | partial | Console/aux ports and telnet/none settings are sent to Dynamips. SSH values are accepted but do not launch an SSH service; API console access remains the controller WebSocket. |
| `mac_addr`, `system_id` | present | Validated and passed to platform hardware commands. |
| `slot0`–`slot6`, `wic0`–`wic2` | present with NET limitation | Platform-specific module/WIC settings are validated and installed in hypervisor commands. NET currently binds only one interface/port; a named TODO marks that boundary. |
| `npe`, `midplane` | present for c7200 | Values are applied with c7200-specific commands. |
| `sensors`, `power_supplies` | missing | Read-only hardware state is not exposed in runtime fields. |
| `iomem` | present where supported | Sent for non-c7200 platforms when configured. |
| `image_md5sum`, `usage` | missing / stored | Checksum validation and launch effect for usage are absent. |

### Dynamips template-only fields and behavior

| Upstream fields/behavior | Port status | Evidence in the port |
| --- | --- | --- |
| Template `default_name_format`, `category`, `symbol` | partial | Defaults are emitted by runtime templates; custom settings are preserved. |
| Template `console_auto_start` | stored only | No automatic console opening. |
| Ghost IOS | present with mmap and image dependencies | On first start, the runtime creates a temporary Dynamips Ghost router with the same IOS/platform/RAM, waits for cache generation, deletes the temporary router, and attaches an existing cache. Upstream requires mmap when Ghost IOS is enabled; the validation test now exercises both rejection without mmap and acceptance with it. c7200/NPE-G2 skips cache generation. This requires a usable IOS image. |
| Idle-PC proposals and auto selection | present | Proposals temporarily clear online Idle-PC and restore it. Auto-selection samples all managed Dynamips process threads from `/proc` for three seconds per candidate and accepts the first candidate below 50% of one host core; if process sampling is unavailable it falls back to Dynamips' whole-second CPU counter. A stopped router is started for 20 seconds and stopped again afterward. The auto-selection route saves the chosen address on the node, and each subsequent router start reapplies that saved address before `vm start`; if there is no selected address, the appliance's default remains in effect. The UI has no fixed Idle-PC example. |
| Suspend/reload/console | present and live-tested | The ignored live lab booted c3620/c3725/c7200 IOS, reached CLI prompts, exercised reload and suspend/resume, and passed post-reload pings through the raw TCP console WebSocket. |
| Network module NIO attachments | present for tested port | Dynamips endpoints use the NET UDP NIO path; live coverage exercised adapter 0/port 0 for point-to-point and switch links. Other module slot/port combinations remain to be tested. |
| Live verification | ignored live integration passed | c3725↔c7200 passed 5/5 pings; c3620↔VPCS and c3600+VPCS+Docker through the built-in switch passed. Auto-selection saved Idle-PC `0x60c086a8` through the API and the live restart reapplied it. The latest run measured 40.8% before / 36.4% after restart (5-second settle, 8-second sample), and 49.0% at the later process check. An earlier full run measured 32.8% / 39.8%, and two immediate 2.5-second post-restart samples read 52.0% and 54.8%, exceeding the strict 50% check before topology checks. With the installed 0.2.14 binary, c3725/c7200 crash in glibc `vscanf` after ROMMON starts IOS; the same images booted using an upstream 0.2.25 override. Graceful controller shutdown left no uBridge child. See `docs/ios-images.md` for image hashes and full results. |

## VPCS

### Compute-node schema fields

| Upstream fields | Port status | Evidence in the port |
| --- | --- | --- |
| `name`, `node_id` | present | Used for per-node process state and startup script substitution. |
| `usage` | stored | Preserved as metadata, with no process launch effect. |
| `console` | partial | A free local TCP port is allocated; caller-specified port is not honored. |
| `console_type` | partial | The runtime provides Telnet console behavior. SSH/none options from the schema are not implemented by the VPCS WebSocket console. |
| `startup_script` | present | Written to `startup.vpc`, `%h` expands to node name, and an empty script overwrites/removes stale content. |

### VPCS template-only fields and behavior

| Upstream fields/behavior | Port status | Evidence in the port |
| --- | --- | --- |
| Template `base_script_file` | present for DB-backed project files | On start/reload, an empty inline script falls back to the relative `project_files` entry. Nonempty inline `startup_script` wins. The source file is resolved as UTF-8 with replacement for invalid bytes and `%h` is expanded when writing the per-instance `startup.vpc`. |
| Template `console_auto_start` | stored only | No automatic console opening. |
| Adapter limit | present | Controller enforces one adapter, matching upstream VPCS’s single Ethernet adapter. |
| Suspend | pending NET integration | VPCS process remains running; suspending its NIO/carrier is marked `TODO(RUNTIME-NET VPCS suspend hook)`. |
| Multiple instances and startup scripts | unit-tested | Each node has its own project/node state directory and startup file. Tests cover independent scripts and host substitution. The host currently has no VPCS executable for a live runtime check. |

## Shared scope boundary

NET owns interface/NIC creation, NIO attachment, carrier state, and link functions. This workstream does not alter those paths. Remaining integration hooks are explicitly marked `TODO(RUNTIME-NET ...)` in QEMU, Docker, Dynamips, and VPCS runtime code. The API/console resize frame is documented above so the frontend can send the expected control message without changing the console byte stream.

Runtime-owned paths from topology JSON are revalidated before use. QEMU QMP/serial paths must match the configured controller socket directory; PID/log/overlay files must match the generated per-project/node paths, and disk/support images must resolve inside the configured image library or node directory. Dynamips working/log/ghost paths and hypervisor/console ports must match a controller-written manifest in the node runtime directory. Docker container IDs must have a valid controller runtime ownership marker under the configured Docker state directory. These checks make imported topology metadata unable to select arbitrary cleanup paths or local hypervisor ports.
