# Security review 3: authentication work

**Scope:** read-only review of `migrations/0006_auth.sql`, `src/auth/`, the auth wiring in `src/api.rs` and `src/main.rs`, and the requested frontend files. Compared route and RBAC behavior with the requested local GNS3 3.1 source tree. Reviewed the current shared worktree on 2026-10-18; other workstreams may still be editing it. No tests or runtime probes were run.

**Evidence labels:** “Verified by reading” means the behavior follows directly from the current source. Deployment-dependent impact is called out separately. “Suspected” identifies an outcome that depends on unspecified database ordering or browser/proxy behavior.

## Findings, ranked by severity

### High

1. **A loopback bind behind a reverse proxy defaults to authentication off, and a later bind change does not enable it.** **Verified by reading; deployment-dependent exposure.** [src/main.rs:37-53](/home/teodor/projects/network-simulator/src/main.rs:37) derives the initial value only from `bind_address.ip().is_loopback()`. On a new database `127.0.0.1` and `::1` default OFF; `0.0.0.0`, `::`, and other non-loopback addresses default ON. The code does not inspect a reverse-proxy or forwarded-client configuration. A controller bound to `127.0.0.1:3080` and exposed by a public reverse proxy therefore starts with the API unauthenticated unless the operator explicitly sets `NETWORK_SIMULATOR_AUTH_ENABLED=true`. Also, if the database was first initialized with a loopback bind, changing the bind to `0.0.0.0` later does not change the persisted OFF setting without an override or Settings update. Accepted false override values turn auth off on any bind address. A valid override is applied on every start; with no valid override, the persisted setting is retained after initial creation. **Exploit scenario:** an operator initially runs locally, later publishes the same database behind a proxy or on a public interface, and assumes the new network bind enables auth; a remote client calls project, node, image, or settings APIs without a token. The proxy is invisible to this decision.

2. **Refresh tokens are reusable and can be renewed indefinitely after theft.** **Verified by reading.** `/refresh` verifies signature, `token_use`, expiry, account activity, and token version, then returns a new access and refresh token without recording or invalidating the presented refresh token [src/auth/mod.rs:1014-1037](/home/teodor/projects/network-simulator/src/auth/mod.rs:1014). Each new refresh token expires 30 days from issuance [src/auth/mod.rs:1040-1071](/home/teodor/projects/network-simulator/src/auth/mod.rs:1040). **Exploit scenario:** an attacker who obtains one refresh token can replay it to mint a new refresh token before each 30-day expiry and maintain access until a password change/logout increments `token_version` or the account is disabled. The browser UI currently does not persist or use the refresh token, but API clients can.

### Medium

3. **Several controller routes use a different, sometimes weaker privilege than GNS3.** **Verified by reading.** `required_privilege()` chooses mostly from the HTTP method and broad path substring checks [src/auth/mod.rs:478-644](/home/teodor/projects/network-simulator/src/auth/mod.rs:478), so all of the following are authenticated but not gated by the upstream privilege:

   | Local route / local required privilege | GNS3 3.1 required privilege | Concrete consequence |
   | --- | --- | --- |
   | `POST /v3/projects/{project_id}/templates/{template_id}` → `Project.Allocate` | `Node.Allocate` ([projects.py:1017-1023](/tmp/gns3-server-3.1/gns3server/api/routes/controller/projects.py:1017)) | Project allocators can create nodes from templates without node-allocation permission. |
   | `POST /v3/projects/{project_id}/files/{*file_path}` → `Project.Allocate` | `Project.Modify` ([projects.py:983-994](/tmp/gns3-server-3.1/gns3server/api/routes/controller/projects.py:983)) | A project allocator can write project files without modify permission. |
   | `POST /v3/projects/{project_id}/lock` and `/unlock` → `Project.Allocate` | `Project.Modify` ([projects.py:901-928](/tmp/gns3-server-3.1/gns3server/api/routes/controller/projects.py:901)) | A project allocator can lock or unlock an existing project. |
   | `POST /v3/projects/{project_id}/nodes/console/reset` and `/nodes/{node_id}/console/reset` → `Node.Allocate` | `Node.Console` ([nodes.py:987-1012](/tmp/gns3-server-3.1/gns3server/api/routes/controller/nodes.py:987)) | Node allocators can reset consoles without console permission. |
   | `POST /v3/projects/{project_id}/nodes/{node_id}/files/{*file_path}` and `DELETE` → `Node.Allocate` | `Node.Modify` ([nodes.py:727-770](/tmp/gns3-server-3.1/gns3server/api/routes/controller/nodes.py:727)) | Node allocators can write/delete node files without node-modify permission. |
   | `POST .../nodes/{node_id}/isolate` and `/unisolate` → `Node.Allocate` | `Link.Modify` ([nodes.py:530-546](/tmp/gns3-server-3.1/gns3server/api/routes/controller/nodes.py:530)) | Node allocators can change link isolation state without link-modify permission. |
   | `GET .../nodes/{node_id}/links` → `Node.Audit` | `Link.Audit` ([nodes.py:561](/tmp/gns3-server-3.1/gns3server/api/routes/controller/nodes.py:561)) | Node auditors can read the attached link list without link-audit permission. |
   | `DELETE /v3/pools/{pool_id}/resources/{resource_id}` → `Pool.Allocate` | `Pool.Modify` ([pools.py:227-240](/tmp/gns3-server-3.1/gns3server/api/routes/controller/pools.py:227)) | Pool allocators can remove projects from pools; pool modifiers alone cannot. |

   The generic fallback is also unsafe for product-specific routes: unclassified POST paths require `Project.Modify`, while unclassified PUT/DELETE paths require only `Server.Audit` [src/auth/mod.rs:639-643](/home/teodor/projects/network-simulator/src/auth/mod.rs:639). For example, the PUT/DELETE appliance image-binding route [src/api.rs:276-279](/home/teodor/projects/network-simulator/src/api.rs:276) can be changed by a principal holding only a read-oriented `Server.Audit` ACE on that path. Docker image pull and marketplace import also fall through to `Project.Modify` rather than a route-specific appliance/image privilege. These paths remain token-protected; the issue is privilege authorization after authentication.

4. **Pool-derived project access does not consistently honor ACE path semantics.** **Verified by reading.** `has_pool_access()` treats any allowed ACE at `/pools/{id}` *or any descendant* as a project grant and ignores the ACE’s `propagate` flag [src/auth/mod.rs:688-702](/home/teodor/projects/network-simulator/src/auth/mod.rs:688). It also considers only allowed ACEs, so a direct user deny on a pool path is skipped when a group allow is evaluated later. **Exploit scenario:** a group has a project-audit allow on `/pools/{id}`; a member’s direct user deny at the same pool path does not stop the later group-pool check, so the member can still read a project in that pool. Separately, a non-propagating allow at `/pools/{id}/resources` can grant project access even though it is not the pool-root grant presented by the UI. The project route matcher itself uses segment boundaries (`/projects/abc` does not match `/projects/abcd`) and `project_id_from_path()` extracts a segment, so the common project-prefix confusion is not present in that matcher [src/auth/mod.rs:704-753](/home/teodor/projects/network-simulator/src/auth/mod.rs:704).

5. **Login has no throttling and its response time can reveal account names.** **Verified by reading; timing impact is an inference.** `token_for_credentials()` returns immediately for an unknown username, while a known active username with a wrong password runs Argon2 verification; inactive accounts short-circuit before verification [src/auth/mod.rs:965-974](/home/teodor/projects/network-simulator/src/auth/mod.rs:965). No rate-limit or lockout layer is present in the router. Password hashing is `Argon2::default()` [src/auth/mod.rs:876-904](/home/teodor/projects/network-simulator/src/auth/mod.rs:876); the locked `argon2` 0.5.3 defaults are Argon2id, 19,456 KiB memory, 2 iterations, 1 lane, 32-byte output. **Exploit scenario:** a remote client can enumerate candidate usernames by latency and make repeated password guesses against `admin`; concurrent hash attempts also provide an unauthenticated CPU-denial-of-service path. The response text for unknown and wrong-password users is otherwise uniform.

6. **WebSocket bearer tokens are put in query strings that can be logged.** **Verified by reading; downstream exposure depends on logging configuration.** The browser app appends its local-storage token to project-notification and console WebSocket URLs [frontend/src/api.ts:680-751](/home/teodor/projects/network-simulator/frontend/src/api.ts:680), and the server accepts that query token for every path ending in `/ws` [src/auth/mod.rs:436-450](/home/teodor/projects/network-simulator/src/auth/mod.rs:436). The router installs `TraceLayer::new_for_http()` [src/api.rs:405-413](/home/teodor/projects/network-simulator/src/api.rs:405); tower-http’s default request span formats the full URI, including query, at DEBUG level. Reverse-proxy access logs commonly record the request target as well. **Exploit scenario:** an operator enables debug request spans or keeps query strings in proxy logs; a log reader copies a still-valid token and calls the API. I did not find code proving a browser `Referer` leak for these WebSocket handshakes; the log exposure is sufficient to warrant concern.

7. **A user manager can deactivate the last active superadmin.** **Verified by reading.** The authenticated user-update route accepts `is_active=false` for any target; the self-disable guard applies only to `/me`, and the update handler does not protect superadmins [src/auth/mod.rs:1148-1155, 1208-1221](/home/teodor/projects/network-simulator/src/auth/mod.rs:1148). Deleting a superadmin is blocked [src/auth/mod.rs:1235-1254](/home/teodor/projects/network-simulator/src/auth/mod.rs:1235), but disabling is not. Bootstrap checks whether *any* `is_superadmin=1` row exists, not whether one is active [src/auth/mod.rs:92-96](/home/teodor/projects/network-simulator/src/auth/mod.rs:92). **Exploit scenario:** a principal with `User.Modify` on `/access/users` disables the sole superadmin. The superadmin can no longer authenticate and startup will not create a replacement. This is an administrative lockout/availability issue; it does not make the caller a superadmin.

8. **The auth-setting lookup fails open if the singleton row is missing.** **Verified by reading; requires a damaged/deleted config row.** Database read errors return HTTP 500 [src/auth/mod.rs:407-421](/home/teodor/projects/network-simulator/src/auth/mod.rs:407), which is fail-closed. But a successful query returning no row is converted to `false` [src/auth/mod.rs:407-414](/home/teodor/projects/network-simulator/src/auth/mod.rs:407), after which the gate skips both authentication and privilege checks [src/auth/mod.rs:423-435](/home/teodor/projects/network-simulator/src/auth/mod.rs:423). **Exploit scenario:** if the `auth_config` singleton is deleted while the process is running, requests to every `/v3` API become unauthenticated until the row is restored or the process is restarted. Normal startup initializes the row, so this is a fail-open corruption path, not the normal startup path.

9. **The initial administrator password is written to the warning log.** **Verified by reading; access depends on deployment log controls.** The random initial password is included as a structured `initial_password` field in a WARN event [src/auth/mod.rs:96-118](/home/teodor/projects/network-simulator/src/auth/mod.rs:96). It is generated with 244 random UUID-v4 bits and hashed before insert, but it can be retained by journald, container logs, or centralized logging. **Exploit scenario:** someone with log access during first start retrieves the initial admin credential before it is changed. The “shown once” behavior is based on the admin row existing at startup, not on log redaction or deletion.

### Low / informational

10. **Auth-enabled static pages, health, and the `/v3` 404 sentinel are public.** **Verified by reading.** The auth middleware immediately bypasses every path outside `/v3/` [src/auth/mod.rs:395-405](/home/teodor/projects/network-simulator/src/auth/mod.rs:395). This includes `GET /healthz`, `/`, `/assets/**`, the catch-all SPA route for `/login` and other frontend URLs, and the exact `/v3` route, which only returns 404 [src/api.rs:243-245, 387-395](/home/teodor/projects/network-simulator/src/api.rs:243). The HTML/JS bundle is public; application data requests still require API tokens. The unauthenticated `/v3/access/settings/status` route additionally reveals only whether auth is enabled. This is useful for login/bootstrap, but `/healthz` reveals controller/database availability to any caller; `/v3` reveals only a 404.

11. **The browser stores the access token in `localStorage`.** **Verified by reading; abuse requires same-origin script execution.** `getAuthToken()` and `saveAuthToken()` read/write `network-simulator.access-token` in local storage [frontend/src/api.ts:44-60](/home/teodor/projects/network-simulator/frontend/src/api.ts:44), and `fetchWithAuth()` adds it to requests. A same-origin XSS can therefore exfiltrate the bearer token and use it until expiry or token-version invalidation. Login stores only the access token and the UI does not use the returned refresh token [frontend/src/api.ts:430-440](/home/teodor/projects/network-simulator/frontend/src/api.ts:430); 401 responses clear the stored token and redirect to `/login` [frontend/src/api.ts:62-69, 406-417](/home/teodor/projects/network-simulator/frontend/src/api.ts:62). This is a storage tradeoff, not a route-auth bypass.

12. **There is no token-key rotation workflow, and `iat` is not validated.** **Verified by reading; low direct risk with the present local token issuer.** A random 64-byte key is generated once and persisted in `auth_config` [src/auth/mod.rs:82-90](/home/teodor/projects/network-simulator/src/auth/mod.rs:82). `decode_token()` accepts only `HS256`, verifies with `ring::hmac::verify`, and enforces `exp` [src/auth/mod.rs:788-805](/home/teodor/projects/network-simulator/src/auth/mod.rs:788); the token consumer checks `token_use`, active status, and token version [src/auth/mod.rs:822-857](/home/teodor/projects/network-simulator/src/auth/mod.rs:822). `none` and algorithm-confusion inputs are rejected. There is no issuer/audience check or validation that the required integer `iat` is plausible/current, and no supported key rollover; manually changing the stored signing key invalidates all JWT access and refresh tokens. The omitted `token_use` claim defaults to `access`, which is only relevant to a valid signature produced with this same secret.

## Route-by-route authentication and privilege table

The route inventory below follows registrations in [src/api.rs:243-415](/home/teodor/projects/network-simulator/src/api.rs:243), [src/api/controller_routes.rs:26-155](/home/teodor/projects/network-simulator/src/api/controller_routes.rs:26), and [src/auth/mod.rs:315-383](/home/teodor/projects/network-simulator/src/auth/mod.rs:315). All registered routes are merged under `auth::protect`; none of the legacy controller or new controller route modules is accidentally omitted. The second column is the policy for an authenticated request when auth is enabled. The four public `/v3` exceptions and exact `/v3` 404 sentinel are listed explicitly; when auth is disabled, the middleware skips auth and privilege checks for every `/v3/` route.

For compact rows, each comma-separated suffix is appended to the nearest full prefix in that row (for example, `/v3/projects/{project_id}/open`, `/close` represents the registered `.../open` and `.../close` routes).

### Core and older controller routes

| Registered route pattern(s) | Method(s) | Auth-enabled gate |
| --- | --- | --- |
| `/healthz` | GET | **No token / no privilege check** |
| `/`, `/{*path}`, `/assets/**` | GET/HEAD via static services | **No token / no privilege check**; `/{*path}` serves the SPA outside the more-specific `/v3` routes |
| `/v3/version` | GET; POST | `Server.Audit`; `Project.Modify` respectively (POST is generic fallback) |
| `/v3/runtimes`, `/v3/resources/monitor` | GET | `Server.Audit` (generic fallback) |
| `/v3/docker/images/pull` | POST | `Project.Modify` (generic fallback) |
| `/v3/marketplace/ishare2/images`, `/v3/marketplace/gns3/appliances`, `/v3/marketplace/gns3/appliances/{file_name}`, `/v3/marketplace/gns3/imported`, `/v3/marketplace/gns3/manager` | GET | `Server.Audit` (generic fallback) |
| `/v3/marketplace/ishare2/images/{image_type}/{image_id}/download`, `/v3/marketplace/gns3/appliances/{file_name}/import`, `/v3/marketplace/gns3/custom`, `/v3/marketplace/gns3/import/{file_name}` | POST | `Project.Modify` (generic fallback) |
| `/v3/marketplace/gns3/appliances/{file_name}/bindings` | PUT, DELETE | `Server.Audit` (generic fallback; mutation/read privilege mismatch) |
| `/v3/projects` | GET, POST | `Project.Audit`, `Project.Allocate` |
| `/v3/projects/{project_id}` | GET, PUT, DELETE | `Project.Audit`, `Project.Modify`, `Project.Allocate` |
| `/v3/projects/{project_id}/open`, `/close` | POST | `Project.Allocate` |
| `/v3/projects/{project_id}/nodes` | GET, POST | `Node.Audit`, `Node.Allocate` |
| `/v3/projects/{project_id}/templates/{template_id}` | POST | `Project.Allocate` (upstream expects `Node.Allocate`) |
| `/v3/projects/{project_id}/nodes/{node_id}` | GET, PUT, DELETE | `Node.Audit`, `Node.Modify`, `Node.Allocate` |
| `/v3/projects/{project_id}/nodes/{node_id}/config` | PUT | `Node.Modify` |
| `/v3/projects/{project_id}/nodes/{node_id}/start`, `/stop`, `/suspend`, `/reload` | POST | `Node.PowerMgmt` |
| `/v3/projects/{project_id}/links` | GET, POST | `Link.Audit`, `Link.Allocate` |
| `/v3/projects/{project_id}/links/{link_id}` | GET, PUT, DELETE | `Link.Audit`, `Link.Modify`, `Link.Allocate` |
| `/v3/projects/{project_id}/links/{link_id}/available_filters` | GET | `Link.Audit` |
| `/v3/projects/{project_id}/links/{link_id}/capture/start`, `/capture/stop` | POST | `Link.Capture` |
| `/v3/projects/{project_id}/links/{link_id}/capture/stream`, `/capture/file` | GET | `Link.Capture` |
| `/v3/projects/{project_id}/drawings` | GET, POST | `Drawing.Audit`, `Drawing.Allocate` |
| `/v3/projects/{project_id}/drawings/{drawing_id}` | GET, PUT, DELETE | `Drawing.Audit`, `Drawing.Modify`, `Drawing.Allocate` |
| `/v3/projects/{project_id}/topology` | GET | `Project.Audit` |
| `/v3/projects/{project_id}/notifications/ws` | WebSocket GET | `Project.Audit`; query-token auth is accepted |
| `/v3/compute/projects/{project_id}/qemu/nodes/{node_id}/console/ws`, `/docker/nodes/{node_id}/console/ws`, `/docker/nodes/{node_id}/aux_console/ws`, `/vpcs/nodes/{node_id}/console/ws`, `/dynamips/nodes/{node_id}/console/ws` | WebSocket GET | `Node.Console`; query-token auth is accepted |
| `/v3/dynamips/images/{file_name}`, `/v3/qemu/images/{file_name}` | POST | `Image.Allocate` |
| `/v3` | Any | **No token / no privilege check**; exact route returns only 404 because the gate requires a `/v3/` prefix. |
| `/v3/{*path}` | Any | Token required; then fallback privilege by method, followed by 404. |

### Additional controller routes added before auth

| Registered route pattern(s) | Method(s) | Auth-enabled gate |
| --- | --- | --- |
| `/v3/templates` | GET, POST | `Template.Audit`, `Template.Allocate` |
| `/v3/templates/{template_id}` | GET, PUT, DELETE | `Template.Audit`, `Template.Modify`, `Template.Allocate` |
| `/v3/templates/{template_id}/duplicate` | POST | `Template.Allocate` |
| `/v3/statistics`, `/v3/notifications`, `/v3/notifications/ws` | GET / WebSocket GET | `Server.Audit` (generic fallback); controller notification socket accepts query token |
| `/v3/settings` | GET, PUT | `Server.Audit`, `Server.Modify` |
| `/v3/computes`, `/v3/computes/{compute_id}` | GET | `Compute.Audit` |
| `/v3/projects/load` | POST | `Project.Allocate` |
| `/v3/projects/{project_id}/stats`, `/locked`, `/gns3file`, `/export` | GET | `Project.Audit` |
| `/v3/projects/{project_id}/duplicate`, `/import` | POST | `Project.Allocate` |
| `/v3/projects/{project_id}/lock`, `/unlock` | POST | `Project.Allocate` (upstream expects `Project.Modify`) |
| `/v3/projects/{project_id}/files/{*file_path}` | GET, POST | `Project.Audit`, `Project.Allocate` (upstream POST expects `Project.Modify`) |
| `/v3/projects/{project_id}/notifications` | GET | `Project.Audit` |
| `/v3/projects/{project_id}/snapshots` | GET, POST | `Snapshot.Audit`, `Snapshot.Allocate` |
| `/v3/projects/{project_id}/snapshots/{snapshot_id}` | DELETE | `Snapshot.Allocate` |
| `/v3/projects/{project_id}/snapshots/{snapshot_id}/restore` | POST | `Snapshot.Restore` |
| `/v3/projects/{project_id}/nodes/start`, `/stop`, `/suspend`, `/reload` | POST | `Node.PowerMgmt` |
| `/v3/projects/{project_id}/nodes/console/reset`, `/nodes/{node_id}/console/reset` | POST | `Node.Allocate` (upstream expects `Node.Console`) |
| `/v3/projects/{project_id}/nodes/{node_id}/duplicate` | POST | `Node.Allocate` |
| `/v3/projects/{project_id}/nodes/{node_id}/isolate`, `/unisolate` | POST | `Node.Allocate` (upstream expects `Link.Modify`) |
| `/v3/projects/{project_id}/nodes/{node_id}/links` | GET | `Node.Audit` (upstream expects `Link.Audit`) |
| `/v3/projects/{project_id}/nodes/{node_id}/files` | GET | `Node.Audit` |
| `/v3/projects/{project_id}/nodes/{node_id}/files/{*file_path}` | GET, POST, DELETE | `Node.Audit`, `Node.Allocate`, `Node.Allocate` (upstream writes/deletes expect `Node.Modify`) |
| `/v3/projects/{project_id}/nodes/{node_id}/qemu/disk_image/{disk_name}` | POST, PUT, DELETE | `Node.Allocate`, `Node.Modify`, `Node.Allocate` |
| `/v3/projects/{project_id}/nodes/{node_id}/dynamips/auto_idlepc`, `/idlepc_proposals` | GET | `Node.Audit` |
| `/v3/images` | GET | `Image.Audit` |
| `/v3/images/upload/{*image_path}` | POST | `Image.Allocate` |
| `/v3/images/{*image_path}` | GET, DELETE | `Image.Audit`, `Image.Allocate` |
| `/v3/symbols`, `/v3/symbols/default_symbols` | GET | `Symbol.Audit` |
| `/v3/symbols/{symbol_id}/raw` | GET, POST | `Symbol.Audit`, `Symbol.Allocate` |
| `/v3/symbols/{symbol_id}/dimensions` | GET | `Symbol.Audit` |
| `/v3/symbols/{symbol_id}` | DELETE | `Symbol.Allocate` |

### Auth and access-management routes

| Registered route pattern(s) | Method(s) | Auth-enabled gate |
| --- | --- | --- |
| `/v3/access/users/login`, `/authenticate` | POST | Public credential endpoints |
| `/v3/access/users/refresh` | POST | Public route, but a valid refresh token in the body is required |
| `/v3/access/settings/status` | GET | Public; reveals auth enabled/disabled state |
| `/v3/access/users/logout`, `/v3/access/users/me` | POST logout; GET/PUT me | Valid bearer required; bypass generic privilege check; handlers operate on the authenticated user |
| `/v3/access/api-keys`, `/v3/access/api-keys/{key_id}/revoke`, `/restore`, `/v3/access/api-keys/{key_id}` | GET/POST; POST; POST; DELETE | Valid bearer required; bypass generic privilege check; handlers scope list/create/revoke/restore/delete to the authenticated owner |
| `/v3/access/users` | GET, POST | `User.Audit`, `User.Allocate` |
| `/v3/access/users/{user_id}` | GET, PUT, DELETE | `User.Audit`, `User.Modify`, `User.Allocate` |
| `/v3/access/users/{user_id}/groups` | GET | `User.Audit` |
| `/v3/access/groups` | GET, POST | `Group.Audit`, `Group.Allocate` |
| `/v3/access/groups/{group_id}` | GET, PUT, DELETE | `Group.Audit`, `Group.Modify`, `Group.Allocate` |
| `/v3/access/groups/{group_id}/members` | GET | `Group.Audit` |
| `/v3/access/groups/{group_id}/members/{user_id}` | PUT, DELETE | `Group.Modify` |
| `/v3/access/roles` | GET, POST | `Role.Audit`, `Role.Allocate` |
| `/v3/access/roles/{role_id}` | GET, PUT, DELETE | `Role.Audit`, `Role.Modify`, `Role.Allocate` |
| `/v3/access/roles/{role_id}/privileges` | GET | `Role.Audit` |
| `/v3/access/roles/{role_id}/privileges/{privilege_id}` | PUT, DELETE | `Role.Modify` |
| `/v3/access/privileges` | GET | `Role.Audit` |
| `/v3/access/acl` | GET, POST | `ACE.Audit`, `ACE.Allocate` |
| `/v3/access/acl/endpoints` | GET | `ACE.Audit` |
| `/v3/access/acl/{ace_id}` | GET, PUT, DELETE | `ACE.Audit`, `ACE.Modify`, `ACE.Allocate` |
| `/v3/pools` | GET, POST | `Pool.Audit`, `Pool.Allocate` |
| `/v3/pools/{pool_id}` | GET, PUT, DELETE | `Pool.Audit`, `Pool.Modify`, `Pool.Allocate` |
| `/v3/pools/{pool_id}/resources` | GET | `Pool.Audit` |
| `/v3/pools/{pool_id}/resources/{resource_id}` | PUT, DELETE | `Pool.Modify` |
| `/v3/access/settings` | GET, PUT | `Server.Audit`, `Server.Modify`; PUT also checks `principal.is_superadmin` in its handler |

## Other requested checks

- **Roles/groups/own ACL/API keys:** there is no self-service route that changes a caller’s `is_superadmin`, role links, or ACL entries through `/me`. `/me` updates profile/password only. User/group/role/ACL handlers are protected by the mapped privileges above. API-key handlers enforce owner IDs in their SQL filters; an authenticated user cannot revoke, restore, list, or delete another user’s keys by changing the path ID. The local ACL create handler accepts paths that do not match registered routes, unlike GNS3. This can create ineffective or surprising ACEs; in combination with the pool descendant check, path scope is covered by finding 4. The ability to write any ACE still requires `ACE.Allocate` on the ACL management route. GNS3 also explicitly permits `/` as its “All endpoints” ACE path, so that path alone is not a local privilege escalation.
- **Last administrator:** deleting any superadmin row is blocked, but the last active superadmin can be disabled (finding 7). No API handler sets `is_superadmin=true`; initial bootstrap is the only creation path.
- **ACL matching:** segment-boundary checking avoids `/projects/abc` matching `/projects/abcd`; deeper matching ACEs are processed first, exact-path entries apply regardless of `propagate`, and a more-specific user ACE is evaluated before group ACEs [src/auth/mod.rs:655-729](/home/teodor/projects/network-simulator/src/auth/mod.rs:655). Denies at an ancestor are inherited regardless of `propagate`, matching upstream `_check_path_with_aces`; pool-derived access has the separate issues in finding 4. **Suspected:** when multiple matching ACEs of equal path length grant and deny the same privilege through different roles, SQL orders only by path length and evaluation returns the first; the winner is not explicitly deterministic [src/auth/mod.rs:672-681, 704-729](/home/teodor/projects/network-simulator/src/auth/mod.rs:672).
- **SQL injection:** none identified in the reviewed auth SQL. Request values are bound parameters. The one dynamically formatted query in endpoint enumeration uses only hard-coded table/column names from a fixed tuple [src/auth/mod.rs:1644-1664](/home/teodor/projects/network-simulator/src/auth/mod.rs:1644). Pool-delete `LIKE` uses a bound parameter [src/auth/mod.rs:1713-1720](/home/teodor/projects/network-simulator/src/auth/mod.rs:1713), so this is not SQL injection.
- **CSRF/CORS:** no CORS layer or cookie-based auth was found. The current UI sends bearer tokens in `Authorization`, so a cross-origin form cannot attach the victim’s token as ambient credentials; I found no direct CSRF bypass. The query-token WebSocket design has the logging risk above. There is no visible WebSocket `Origin` allowlist.
- **Frontend auth-only URL consumers:** console WebSockets use token query parameters and are handled by the server gate. Two non-WebSocket browser consumers do not attach credentials: `linkCaptureStreamUrl()` returns a bare URL that is opened/copied directly [frontend/src/api.ts:664-665](/home/teodor/projects/network-simulator/frontend/src/api.ts:664), while query-token auth is accepted only for `/ws`; the capture stream therefore returns 401 with auth enabled. `symbolRawUrl()` also returns a bare asset URL used in SVG `<image>` [frontend/src/api.ts:732-734](/home/teodor/projects/network-simulator/frontend/src/api.ts:732), so custom symbols fail to load for authenticated users. These are verified auth-compatibility failures, not anonymous access.
- **Direct Docker HTTP console:** this is not a controller route. The UI opens the configured host/port without the bearer token [frontend/src/App.tsx:484-493](/home/teodor/projects/network-simulator/frontend/src/App.tsx:484), but the controller’s Docker HTTP-console proxy binds to `127.0.0.1` [src/runtime.rs:737-752](/home/teodor/projects/network-simulator/src/runtime.rs:737), so no remote network bypass was verified from this code. If a deployment separately republishes that local port, it is outside `auth::protect` and needs its own access control.

## Fix status

Implemented and checked against the findings above:

- Bind-derived auth is re-evaluated at startup; a non-loopback bind enables auth unless `NETWORK_SIMULATOR_AUTH_ENABLED=false` is explicitly set, in which case a loud error is logged. On loopback, forwarded headers force auth unless `NETWORK_SIMULATOR_TRUST_PROXY_HEADERS_UNAUTH=true`. Missing `auth_config` returns an error instead of opening access.
- Refresh tokens now have persisted token IDs and families, rotate once, expire after seven days, and revoke their family on reuse. JWT validation checks `iat`, `nbf`, issuer, audience, expiry, and token ID. Password verification uses a dummy hash for unknown/inactive users, per-username-and-peer-IP exponential backoff, and a global hash concurrency cap.
- Privilege mappings cover the requested GNS3 corrections. The generic authorization fallback now denies unclassified routes, and a route-registration test checks that every registered `/v3` method has a classification. Docker pull uses `Compute.Modify`, matching the GNS3 route; appliance import/bindings, iShare downloads, custom appliance import, and catalog reads have explicit classifications. Pool resource PUT/DELETE both require `Pool.Modify`.
- Pool access now honors propagation, includes direct-user denies, considers only pool-root and exact project-resource ACEs, and resolves equal-depth ACE ties deterministically with deny first.
- WebSocket request spans record the URI path without its query. One-use, path-bound, 60-second tickets are available at `POST /v3/access/ws-ticket` and used by the UI for WebSockets and capture streams. Bearer query-token support remains only as a `/ws` fallback.
- The last active superadmin cannot be disabled or deleted, and bootstrap checks for an active superadmin. Generated bootstrap passwords are printed once, written to `initial-admin-password.txt` with mode `0600` on Unix, and require a password change at first login.
- Frontend access tokens now live in memory and `sessionStorage`; the legacy `localStorage` token is removed. `sessionStorage` was chosen to retain a login across reloads in the current tab while keeping it out of persistent cross-session storage. Authenticated capture streams and custom symbols use authorized fetch/blob URLs, and socket URLs use tickets.

No reviewed claim was found to be false. One privilege detail was clarified by checking the corresponding GNS3 route: Docker image pull is `Compute.Modify`.

Verification performed: the focused auth suite passed (15/15); the trace-span redaction test passed (1/1); frontend typecheck passed; frontend tests passed (93/93); `cargo fmt` completed successfully. The requested full `cargo test --locked` run compiled but finished with 25 failures, 111 passes, and 15 ignored tests. The API integration fixtures build migrated stores without running auth initialization, so the fail-closed middleware rejects them because `auth_config` and the bootstrap admin are absent. One separate SPA test still expects an unknown `/v3` route to return 404, while the explicit deny-by-default classification returns 403. Those fixture/expectation updates are outside this auth implementation because another agent owns the test files.
