# SDN Controller ("Flowdeck") — design and API contract

Goal: a controller UI for Open vSwitch devices (the `gns3/openvswitch` Docker nodes of the network simulator)
that exposes everything OpenFlow offers (flows, groups, meters, ports, stats, tracing) with a real CRUD UI
instead of typing `ovs-ofctl` lines. Standalone app in `sdn-controller/` (not part of the main crate).

## Architecture
- `sdn-controller/` — Rust crate `flowdeck` (axum 0.8, tokio, sqlx+SQLite, serde). Listens on `FLOWDECK_BIND` (default `127.0.0.1:3095`), serves `web/dist` at `/`, API under `/api/v1`.
- `sdn-controller/web/` — React 19 + Vite + Tailwind v4 + shadcn/ui + TanStack Router (same stack and look as `../frontend`, copy its tokens from `frontend/src/styles.css`, `components.json`, `src/components/ui/*`). bun only (never npm).
- **Driver**: there is no `ovs-ofctl` on the host. Switches are managed by running `docker exec <container> ovs-ofctl -O OpenFlow13 …` / `ovs-vsctl …`. A `SwitchDriver` trait (`exec(args) -> Output`) has a `DockerDriver`; a `MockDriver` (canned outputs) is used by unit tests. Never build shell strings: pass argv vectors to `tokio::process::Command`.
- **Desired state in SQLite** (`flowdeck.db`): switches, flows, groups, meters, links. UI edits the DB; `POST /switches/{id}/sync` pushes desired state to the switch (groups first, then meters, then flows, via `ovs-ofctl --bundle`), `GET /switches/{id}/drift` compares desired vs `dump-*` actual (missing / extra / differing). Every write endpoint also supports `?apply=true` to push immediately.
- Always use port NAMES in flows (`in_port=eth2`, `output:eth3`), not ofport numbers. Groups need bridge protocols `OpenFlow13` (`ovs-vsctl set bridge <br> protocols=OpenFlow13`), done automatically on connect. Create groups/meters before flows that reference them.

## Data model (JSON, snake_case)
```
Switch   { id, name, container, bridge (default "main-net"), status: "connected"|"unreachable"|"unknown",
           ovs_version?, protocols[], last_seen? }
Port     { name, ofport, state: "up"|"down", link_state, type, tag?, trunks[], mac?, rx_packets, tx_packets, rx_bytes, tx_bytes, rx_dropped, tx_dropped }
Flow     { id (stable hash of table+priority+match), table (0), priority (0..65535), cookie?, idle_timeout?, hard_timeout?,
           match: { in_port?, dl_src?, dl_dst?, dl_type?, dl_vlan?, nw_src?, nw_dst?, nw_proto?, tp_src?, tp_dst?, ... any ofctl field: string },
           actions: Action[], raw?: string, n_packets?, n_bytes?, duration? }
Action   { type: "output"|"group"|"meter"|"mod_vlan_vid"|"strip_vlan"|"push_vlan"|"set_field"|"mod_dl_src"|"mod_dl_dst"|"mod_nw_src"|"mod_nw_dst"
                 |"mod_tp_src"|"mod_tp_dst"|"set_queue"|"goto_table"|"resubmit"|"normal"|"flood"|"drop"|"controller"|"local"|"bundle"|"learn"|"raw",
           value?: string, ... }      // e.g. {type:"output",value:"eth0"}, {type:"mod_vlan_vid",value:"10"}, {type:"raw",value:"ct(commit)"}
Group    { group_id, type: "all"|"select"|"indirect"|"ff", buckets: Bucket[] }   // Bucket { weight?, watch_port?, watch_group?, actions: Action[] }
Meter    { meter_id, flags: ("kbps"|"pktps"|"burst"|"stats")[], bands: { type:"drop"|"dscp_remark", rate, burst_size?, prec_level? }[] }
Link     { id, a_switch, a_port, b_switch, b_port }
```
Serialisation to/from ofctl text lives in one well-tested module (`ofctl.rs`): `Flow -> "priority=…,match,actions=…"` and parser for `dump-flows`/`dump-groups`/`dump-meters` output. Unknown actions/fields must survive as `raw` rather than being dropped.

## REST API (`/api/v1`)
```
GET    /health
GET    /discover                       list running docker containers whose image matches openvswitch (name, image, ports)
GET    /switches                       POST /switches {name, container, bridge?}
GET    /switches/{id}                  PUT /switches/{id}       DELETE /switches/{id}
POST   /switches/{id}/connect          verify reachability, set protocols=OpenFlow13, refresh status
GET    /switches/{id}/ports            live ports + counters
PUT    /switches/{id}/ports/{name}     {tag?, trunks?}  (ovs-vsctl set port)
GET    /switches/{id}/flows            desired flows merged with live counters; ?source=live|desired
POST   /switches/{id}/flows            PUT /switches/{id}/flows/{flow_id}    DELETE /switches/{id}/flows/{flow_id}
POST   /switches/{id}/flows/import     {text}  one ofctl flow per line  → parsed flows;  GET …/flows/export → text
GET/POST/PUT/DELETE /switches/{id}/groups[/{group_id}]
GET/POST/PUT/DELETE /switches/{id}/meters[/{meter_id}]
POST   /switches/{id}/sync             push desired → switch      GET /switches/{id}/drift
POST   /switches/{id}/trace            {in_port, packet:{dl_src,dl_dst,dl_vlan,dl_type,nw_src,nw_dst,nw_proto,tp_src,tp_dst}} → ovs-appctl ofproto/trace result (text + parsed steps)
GET/POST/DELETE /links                 topology links; GET /topology → {switches, links}
WS     /ws                             {type:"switch_status"|"flow_stats"|"port_stats", switch_id, data} pushed every 2 s while a client is connected
```
Errors: `{ "error": { "code": "...", "message": "..." } }` with proper HTTP status. Auth is out of scope for phase 1.

## UI (web)
Sidebar app shell (compact 44px header like the main app). Pages: **Switches** (cards/table, status dot, discover + add dialog), **Switch detail** with tabs Overview · Ports · Flows · Groups · Meters · Trace · Drift, **Topology** (SVG graph, drag nodes, click link/port → detail, live counters), **Settings**.
Flows tab: sortable/filterable table (priority, table, match chips, action chips, packets, bytes with live rate), row actions edit/duplicate/delete, **flow editor dialog** with a structured match builder (field picker + value), an ordered action builder (drag to reorder, per-type inputs, group/meter pickers populated from existing ones), a live "ofctl preview" line and a raw-text mode, validation before save. Groups tab: group cards showing buckets, type explainer, bucket editor with watch_port pickers. Everything has loading/empty/error states, keyboard accessible, dark/light.

## Phases
1. Backend core: DB, ofctl serialisation/parsers (unit tested with MockDriver), switches/ports/flows/groups/meters CRUD, sync/drift, tests; integration test against a real `gns3/openvswitch` container (skipped when docker is missing).
2. Frontend against the contract (mock server in `web/mock/server.ts` until the backend lands): shell, Switches, Switch detail with Flows/Groups editors.
3. Meters, Trace, Topology + `/ws` live counters, link import from the main simulator.
