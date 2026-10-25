# Frontend and backend route gaps

Generated inventory command: `python3 scripts/frontend_backend_gaps.py`.

The frontend expects these route families that are absent or explicitly unimplemented according to `docs/api-parity.md`:

| Method | Route | Status / scope |
| --- | --- | --- |
| POST | `/v3/computes` | Missing remote compute registration |
| PUT | `/v3/computes/{id}` | Missing remote compute update |
| DELETE | `/v3/computes/{id}` | Missing remote compute removal |
| POST | `/v3/projects/{id}/nodes/console/reset` | Registered, currently returns 501 |
| POST | `/v3/projects/{id}/nodes/{node_id}/console/reset` | Registered, currently returns 501 |

The script scans TypeScript API request sites (including `frontend/src/api.ts` and `frontend/src/api/`), normalizes interpolated path parameters, and compares them with Axum `.route` registrations throughout `src/api`. It also flags parity-matrix entries explicitly marked missing or unimplemented. A source-level scan can include false positives for requests delegated through wrappers; the table above is the reviewed list of requested gaps. Broader parity omissions (remote compute operations, appliance catalog/install workflows, image catalog/sync, and other parity-matrix gaps) are larger than the small-route scope and remain documented in `docs/api-parity.md`.
