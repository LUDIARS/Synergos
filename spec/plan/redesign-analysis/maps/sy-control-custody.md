# Subsystem map: synergos-control (control plane), synergos-custody (opaque object custody), synergos-admin-ui session/API client

Repository root: `/home/user/Synergos`. All paths below are relative to that root unless absolute.
Files read completely: `synergos-control/src/{main,config,auth,error,reconcile}.rs`, `synergos-control/src/api/{mod,nodes,orgs,heartbeat,mesh_setup,reconcile_api,ui}.rs`, `synergos-control/src/cloudflare/{mod,client}.rs`, `synergos-control/src/store/{mod,json_store}.rs`, `synergos-control/control.example.toml`, `synergos-control/Cargo.toml`, `synergos-custody/src/{lib,config,contract,handlers,store,durable_file,signature,error}.rs`, `synergos-custody/tests/custody.rs`, `synergos-custody/Cargo.toml`, `synergos-admin-ui/src/{session,app}.rs`, `synergos-admin-ui/src/api/{client,paths,models}.rs`, `spec/feature/opaque-custody.md`, `docs/mesh-operations.md`, `docs/admin-ui.md`.
Additionally consulted (partial, for cross-checks only): `synergos-admin-ui/src/api/mod.rs`, `synergos-admin-ui/src/views/login.rs`, `synergos-admin-ui/src/views/mesh_setup.rs` (grep), `synergos-core/src/control_heartbeat.rs` (lines 56-144), `/home/user/Curare/curare-server/src/storage/{synergos-storage,durable-receipt,receipt-policy}.ts` (grep), workspace `Cargo.toml`.

Statements marked **[inference]** are reasoned from the code but not directly evidenced by a line; everything else cites `file:line`.

---

## 1. Responsibilities & data flow

### 1.1 synergos-control (binary `synergos-control serve --config <toml>`)

Purpose per `docs/mesh-operations.md:101-109`: an org-scoped node registry plus Cloudflare Mesh automation whose stated goal is to prevent "dark nodes" (Mesh participants not known to the registry).

What enters:
- TOML config file (`config.rs:89-98`) and two secrets from environment variables: admin bearer token and Cloudflare API token (`config.rs:119-133`).
- HTTP requests on a single TCP listener (`main.rs:82-90`), default `127.0.0.1:4250` (`config.rs:56-58`):
  - Admin REST API under `/v1/orgs...`, `/v1/reconcile`, `/v1/mesh/*` guarded by the admin bearer token (`api/mod.rs:42-77`).
  - `POST /v1/heartbeat` from Synergos daemons, guarded by a per-node key (`api/mod.rs:87-93`, `api/heartbeat.rs:41-108`).
  - `GET /v1/health` unauthenticated (`api/mod.rs:89,100-105`).
  - `GET /ui`, `/ui/`, `/ui/*` static admin UI, unauthenticated (`api/mod.rs:81-85`, `api/ui.rs`).
  - When `SYNERGOS_CUSTODY_ENABLED=1`, the custody router is merged into the same listener (`main.rs:77-80`).

What leaves:
- Cloudflare API v4 calls over HTTPS (`cloudflare/client.rs`): create/list/delete `warp_connector`, fetch connector token, list/revoke `devices/registrations`, `user/tokens/verify`.
- A JSON registry file on local disk (`store/json_store.rs:44-82`), default path `synergos-control-store.json` relative to cwd (`config.rs:60-62`).
- `tracing` logs to stdout; default filter `synergos_control=info` (`main.rs:44-49`).

Internal modules and call graph:
- `main.rs` -> `ControlConfig::load` + `resolve_secrets` -> `JsonStore::open` -> `CloudflareClient::new` -> `api::build_router` -> optional `synergos_custody::router` merge -> `axum::serve` with graceful shutdown on Ctrl+C/SIGTERM (`main.rs:57-117`).
- `api/nodes.rs` calls `store` and `cloudflare` (connector create/token/delete).
- `api/orgs.rs` calls `store` only.
- `api/heartbeat.rs` calls `store.try_mutate_node` only (no Cloudflare).
- `api/reconcile_api.rs` calls `store.snapshot`, `cloudflare.list_*`, `reconcile::classify`, and optionally `cloudflare.revoke_device_registrations` / `delete_mesh_connector`.
- `api/mesh_setup.rs` builds a request-scoped `CloudflareClient` from a token supplied in the request body and reuses `reconcile_with` (`api/mesh_setup.rs:213-226`, `api/reconcile_api.rs:35-65`).
- `api/ui.rs` reads files from `ui.dist_path` synchronously.
- No module talks to Cernere, Infisical, Curare, or synergos-net/synergos-core. The registry is never pushed to the data plane (no code path exports an allowlist). Grep for `cernere|infisical` across the repo matches only CSS/README references in `synergos-tauri` and the custody spec text (`spec/feature/opaque-custody.md:58`).

### 1.2 synergos-custody (library crate, embedded in control)

Purpose per `spec/feature/opaque-custody.md:4-5`: Curare Server owns versions/submissions/permissions; Synergos custody only transfers and stores opaque objects and issues durability receipts.

What enters:
- Seven required env vars (`config.rs:17-40`).
- Admin commands from Curare Server over HTTP with a static bearer token: `health`, `admissions`, `verify`, `head`, `download`, `reclaim` (`lib.rs:29-40`).
- Browser/CLI transfers `PUT|GET /v1/custody/transfer/:key?expires&signature&name` authorized by HMAC capability in the query string (`lib.rs:41-51`, `handlers.rs:199-273`).

What leaves:
- Files under `SYNERGOS_CUSTODY_ROOT` (flat directory): `<key>.json` (admission), `<key>.data` (bytes), `<key>.reclaimed` (tombstone), `<sha256("operation:"+operationId)>.operation`, `<uuid>.part` (in-flight), `custody.lock` (`store.rs:76-78, 94-129, 171-222`, `durable_file.rs:19-34`).
- JSON responses: upload capability URL + required headers; signed receipts; download capability URL (`handlers.rs:91-185`).
- `tracing::warn!` only on filesystem errors and partial-cleanup failures (`error.rs:17-22`, `durable_file.rs:11-17`). No info-level logging of any operation.

Counterparty (Curare) behaviour observed: `curare-server/src/storage/synergos-storage.ts:17` sends `Authorization: Bearer <token>` to `/v1/custody/<action>` with a 60 s timeout and `redirect:'error'`; `:25` requires capability URLs to share the configured origin and path prefix; `:29-30` validates health `{ready:true, backend:'synergos', durability:'durable', nodeId}` and requires `nodeId` to equal `SYNERGOS_STORAGE_NODE_ID`; `:44` requires `receipt.locations` to include the configured node id; `durable-receipt.ts:11-21` verifies the HMAC with the shared secret and rejects expired receipts.

### 1.3 synergos-admin-ui (Dioxus WASM, served from `/ui/`)

- `session.rs` keeps the admin token in `sessionStorage` under key `synergos-admin-token` (`session.rs:6-35`).
- `api/client.rs` attaches `Authorization: Bearer <token>` to every relative-path request and maps HTTP 401 to `ApiError::Unauthorized` (`api/client.rs:82-84, 109-111`).
- `api/paths.rs` percent-encodes path segments (`api/paths.rs:27-37`).
- `api/models.rs` mirrors the control DTOs; `RegisterNodeResponse` carries `node_key` and `connector_token` in the clear to the browser (`api/models.rs:90-98`).
- `app.rs` switches between `Login` and the app body depending on the token signal; sign-out and any 401 clear the token (`app.rs:16-24, 29-57`).
- `api/mod.rs:52-55, 73-83` hard-codes `revoke_dark: false` for UI-initiated reconciles.
- `views/login.rs:27` validates a token by calling `GET /v1/orgs`; `views/mesh_setup.rs:125` clears the Cloudflare API token input after the run.

---

## 2. Key types / APIs / wire formats

### 2.1 synergos-control

Global: `DefaultBodyLimit::max(64 * 1024)` applied to every control route including UI and heartbeat (`api/mod.rs:96`); every response gets `Cache-Control: no-store` and `Pragma: no-cache` (`api/mod.rs:97, 120-129`). Error body is `{"error": "<message>"}` with `Config|Store -> 500 "internal error"`, `Cloudflare -> 502 "upstream service error"`, `NotFound 404`, `Conflict 409`, `InvalidRequest 400` (message echoed verbatim), `Unauthorized 401` (`error.rs:30-63`).

Store types (`store/mod.rs`):
- `Org { id: slug, name, members: Vec<email>, created_at_ms }` (`store/mod.rs:9-17`).
- `NodeKind = mesh_node | client_device` (`store/mod.rs:20-27`).
- `Node { id: uuid v4, org_id, display_name, owner_email, kind, cf_connector_id?, synergos_peer_id?, mesh_ip?, reported_mesh_ip?, node_key_hash? (blake3 hex), last_heartbeat_ms?, synergos_version?, created_at_ms, updated_at_ms }` (`store/mod.rs:31-63`).
- `RegistrySnapshot { orgs, nodes }` persisted as one pretty-printed JSON document (`store/mod.rs:67-72`, `json_store.rs:45`). No schema version field.

Admin API (all require `Authorization: Bearer <SYNERGOS_CONTROL_ADMIN_TOKEN>`; `api/mod.rs:42-77`):
| Method/path | Request | Response | Notes |
|---|---|---|---|
| `POST /v1/orgs` | `{id, name, members[]}` | `Org` | `id` must match `[a-z0-9-]{1,64}` (`orgs.rs:66-79`); members trimmed, lowercased, deduped, must contain `@` (`orgs.rs:81-95`) |
| `GET /v1/orgs` | - | `Vec<Org>` incl. all member emails (`orgs.rs:40-42`) | |
| `GET/PUT /v1/orgs/:org` | `{name?, members?}` | `Org` | PUT refuses removing a member that still owns a node (`json_store.rs:105-118`) |
| `POST /v1/orgs/:org/nodes` | `{display_name, owner_email, kind, synergos_peer_id?}` | `{node: NodeView, connector_token?, node_key, enroll_hint?}` (`nodes.rs:33-42`) | For `mesh_node`: creates CF connector named `syn-<org>-<first 8 of node id>` then fetches its token (`nodes.rs:123-142`) |
| `GET /v1/orgs/:org/nodes` | - | `Vec<NodeView>` | `NodeView` omits `node_key_hash` (`nodes.rs:44-80`) |
| `GET/PATCH/DELETE /v1/orgs/:org/nodes/:id` | PATCH `{display_name?, owner_email?, synergos_peer_id?, mesh_ip?}` | `NodeView` | DELETE deletes the CF connector first (`nodes.rs:249-261`) |
| `POST .../connector-token` | - | `{connector_token}` | Re-fetches `GET warp_connector/{id}/token` (`nodes.rs:263-277`) |
| `POST .../node-key` | - | `{node_key}` | Replaces `node_key_hash` under lock (`nodes.rs:279-295`) |
| `POST /v1/reconcile` | `{revoke_dark?: bool}` (body optional) | `ReconcileReport` | Uses the startup CF token (`reconcile_api.rs:24-31`) |
| `GET /v1/mesh/context` | - | `{account_id, api_base}` (`mesh_setup.rs:206-211`) | |
| `POST /v1/mesh/token-check` | `{api_token}` | `{token_status, expires_on, account_id, mesh_node_count}` | Calls `user/tokens/verify` then lists connectors (`mesh_setup.rs:91-114`) |
| `POST /v1/mesh/reconcile` | `{api_token, revoke_dark?}` | `ReconcileReport` | Request-scoped CF client (`mesh_setup.rs:117-127`) |
| `POST /v1/mesh/connector-tokens` | `{api_token, org_id}` | `{org_id, issued, entries[{node_id, display_name, connector_token?, skipped_reason?, enroll_command?}]}` | Token is embedded inside `enroll_command` string too (`mesh_setup.rs:168-172`) |

Node-authenticated API:
- `POST /v1/heartbeat` with `Authorization: Bearer <node_key>` and body `{node_id, peer_id, mesh_ip?, synergos_version?}` -> `{"ok": true}` (`heartbeat.rs:17-25, 41-108`).
- `node_key` = two `uuid::Uuid::new_v4().simple()` concatenated = 64 lowercase hex chars (~244 bits of OS randomness) (`heartbeat.rs:27-34`). Stored as unsalted `blake3` hex (`heartbeat.rs:36-39`).
- Validation: `peer_id` 1..=256 chars, no control chars; `synergos_version` <=128 chars, no control chars; `mesh_ip` must be IPv4 inside `100.96.0.0/12` (`heartbeat.rs:66-87`, `api/mod.rs:107-118`).

Reconcile report (`reconcile.rs:10-27`): `known_connectors[{connector, node}]`, `dark_connectors[MeshConnector]`, `missing_connectors[NodeRef]`, `known_devices[{registration, org_ids}]`, `dark_devices[DeviceRegistration]`, `mesh_ip_mismatches[{node, reported_mesh_ip, expected_mesh_ip}]`, `actions[String]`, `generated_at_ms`. `NodeRef` and `DeviceRegistration` include `owner_email` / `user_email` (`reconcile.rs:51-57`, `cloudflare/mod.rs:21-36`).

Cloudflare client (`cloudflare/client.rs`): `reqwest` with 30 s timeout, no redirects, rustls, user-agent `synergos-control/<ver>` (`:43-48`); URL `<api_base>/accounts/<account_id>/<path>` (`:57-59`); envelope `{success, errors[{code,message}], result}` (`:23-37`); lists use `per_page=1000` with no pagination loop (`:157, 217`); `delete_mesh_connector` treats 404 as success (`:175-188`); `revoke_device_registrations` sends repeated `id=` query params and no body (`:235-252`). `api_base` must be `https://` (plain `http` only for loopback) with no credentials/query/fragment (`:263-294`); `account_id` and `connector_id` restricted to `[A-Za-z0-9_-]` (`:296-308`).

Request-scoped Cloudflare token (`mesh_setup.rs:229-241`): 1..=512 bytes, all `is_ascii_graphic`.

Config TOML (`config.rs:13-54`): `bind_addr` (default `127.0.0.1:4250`), `store_path` (default `synergos-control-store.json`), `admin_token_env` (default `SYNERGOS_CONTROL_ADMIN_TOKEN`), `[cloudflare] account_id` (required non-empty), `api_token_env` (default `CLOUDFLARE_API_TOKEN`), `api_base` (default `https://api.cloudflare.com/client/v4`), `[ui] dist_path` (optional; must exist at startup). Admin token must be >=32 bytes (`config.rs:122-127`); Cloudflare token only non-empty (`config.rs:128, 136-143`).

### 2.2 synergos-custody

Admin routes (`lib.rs:29-40`) require `Authorization: Bearer <SYNERGOS_CUSTODY_TOKEN>` (`handlers.rs:21-40`) and are limited to 64 KiB bodies (`lib.rs:40`). All JSON inputs are `camelCase` with `deny_unknown_fields` (`contract.rs:7, 46, 51, 58`). Error bodies: `{"error": code}` with `invalid_custody_request 400`, `custody_access_denied 403`, `custody_object_not_found 404`, `custody_operation_conflict 409`, `custody_object_exists 412`, `custody_unavailable 503` (`error.rs:23-35`).

| Route | Request | Response |
|---|---|---|
| `GET /v1/custody/health` | - | `{ready:true, backend:"synergos", durability:"durable", nodeId}` (`handlers.rs:86-90`) |
| `POST /v1/custody/admissions` | `Admission {operationId, opaqueObjectId, contentHash, byteLength, expiresAt, contentType}` | `{uploadUrl, requiredHeaders:{"Content-Type", "If-None-Match":"*", "X-Content-SHA256"}}` (`handlers.rs:91-106`) |
| `POST /v1/custody/verify` | `{opaqueObjectId}` | `{receipt:{operationId, opaqueObjectId, verifiedDigests:[{algorithm:"sha256", value}], byteLength, durability:"durable", locations:[nodeId], expiresAt}, signature}` (`handlers.rs:107-132`) |
| `POST /v1/custody/head` | `{opaqueObjectId}` | `{contentLength, contentHash, contentType}` or `null` (`handlers.rs:133-150`) |
| `POST /v1/custody/download` | `{opaqueObjectId, expiresIn (1..=900 s), downloadName (1..=100, `[A-Za-z0-9._-]`)}` | `{downloadUrl}` (`handlers.rs:151-174`) |
| `POST /v1/custody/reclaim` | `{opaqueObjectId}` | `{reclaimed:true}` (`handlers.rs:175-185`) |
| `PUT /v1/custody/transfer/:key?expires&signature&name=` | headers `If-None-Match: *`, `X-Content-SHA256: <admission.contentHash>`, `Content-Type: <admission.contentType>` exact; raw body | `201` (`handlers.rs:212-244`) |
| `GET /v1/custody/transfer/:key?expires&signature&name=<downloadName>` | - | bytes, `Content-Type: application/octet-stream`, `Content-Length`, `Content-Disposition: attachment; filename="<name>"` (`handlers.rs:245-273`) |
| `OPTIONS /v1/custody/transfer/:key` | - | `204` (`lib.rs:46`) |

Admission limits (`contract.rs:17-34`): `operationId` UUID; `opaqueObjectId` 1..=2000 chars (unrestricted content); `contentHash` = `sha256:` + 64 lowercase hex; `byteLength <= SYNERGOS_CUSTODY_MAX_OBJECT_BYTES`; `contentType` 1..=200 chars, no CR/LF; `expiresAt` RFC 3339 in `(now, now + 24h + 1min]`.

Keys and signatures (`signature.rs`): `key = hex(sha256(opaqueObjectId))` (`:5-7`); `canonical()` is a bespoke recursive sorted-key JSON serialization (`:8-32`); `sign = hex(HMAC-SHA256(receipt_secret, msg))` (`:33-38`); `verify` requires 64 lowercase hex and compares via `Mac::verify_slice` (constant time) (`:39-51`); capability message = `canonical({"method","key","expires","name"})` (`:52-54`). Upload capability expiry = `min(admission.expiresAt, now + 15 min)` in ms (`handlers.rs:100`); download capability expiry = `now + expiresIn*1000` ms (`handlers.rs:172`); receipt `expiresAt` = `min(admission.expiresAt, now + 5 min)` (`handlers.rs:120-123`).

Capability query struct `Capability { expires: i64 (ms), signature, name (default "") }` with `deny_unknown_fields` (`contract.rs:57-64`): any additional query parameter makes axum's `Query` extractor reject the request.

Concurrency/limits: per-key async mutex with 10 s acquisition timeout -> `503` (`store.rs:58-75`); lock map pruned only when >1024 entries (`store.rs:61-63`); PUT wrapped in 1800 s timeout (`handlers.rs:237-242`); no body limit on PUT other than the admission `byteLength` check during streaming (`store.rs:189-193`); `verify_file` read buffer 128 KiB (`store.rs:153`); GET uses `ReaderStream::new(file)` default capacity (4 KiB per read [inference from tokio-util default]) (`handlers.rs:257`).

Config env (`config.rs:17-59`): `SYNERGOS_CUSTODY_ENABLED` (`0|1`, anything else is an error), `SYNERGOS_CUSTODY_ROOT` (absolute), `SYNERGOS_CUSTODY_PUBLIC_URL` (origin only: http(s), host, no creds/query/fragment, path `/`), `SYNERGOS_CUSTODY_NODE_ID` (UUID), `SYNERGOS_CUSTODY_TOKEN` (>=32 chars), `SYNERGOS_CUSTODY_RECEIPT_SECRET` (>=32 chars), `SYNERGOS_CUSTODY_MAX_OBJECT_BYTES` (1..=2^53-1), `SYNERGOS_CUSTODY_CLIENT_ORIGINS` (comma list; each must equal its own ASCII origin serialization).

### 2.3 synergos-admin-ui

- Session key `synergos-admin-token` in `sessionStorage` (`session.rs:6`).
- `ApiError { Unauthorized, Server{status,message}, Transport(String) }` (`api/client.rs:12-19`).
- Paths: `/v1/orgs`, `/v1/reconcile`, `/v1/mesh/{context,token-check,reconcile,connector-tokens}`, `/v1/orgs/{org}/nodes[/{id}[/connector-token]]` (`api/paths.rs:6-23`).
- DTOs mirror control (`api/models.rs`); `ReconcileReport` in the UI drops `known_connectors` / `known_devices` (`api/models.rs:143-157`).

---

## 3. Security controls PRESENT

### 3.1 Authentication / authorization
- Admin API bearer token compared with a constant-time loop (`synergos-control/src/auth.rs:12-28, 39-48`); applied as `middleware::from_fn_with_state` to every admin route including `/v1/mesh/*` (`api/mod.rs:73-76`). Router tests assert 401 for missing and wrong tokens on mesh routes (`api/mod.rs:172-204`).
- Admin token must be >=32 bytes and present at startup; missing secrets abort startup (fail-fast) (`config.rs:119-143`).
- Heartbeat authenticated by per-node key; hash comparison is constant-time and done inside the store lock together with the update (`api/heartbeat.rs:52-98`); unknown `node_id` is reported as 401 rather than 404 to avoid enumeration (`api/heartbeat.rs:100-105`); nodes without an issued key are rejected (`api/heartbeat.rs:59-62`).
- Node key material is never persisted; only the blake3 hash (`api/nodes.rs:31-32, 114`, `store/mod.rs:52-54`); `NodeView` omits the hash and a test asserts it (`api/nodes.rs:44-80, 413-434`).
- Node key reissue replaces the hash atomically under the store lock and old keys are immediately invalid (`api/nodes.rs:279-295`, `json_store.rs:202-238`).
- Owner must be a member of the org at registration, at PATCH, and re-checked inside every mutation (`api/nodes.rs:89-100, 209-221`, `json_store.rs:144-160, 220-234`); org update refuses to drop a member who still owns a node (`json_store.rs:105-118`).
- Org-scoped lookups: `get_node`, `remove_node`, `list_nodes`, `mutate_node(Some(org))` filter by `org_id` (`json_store.rs:184-198, 240-274`). Only the heartbeat uses the org-agnostic path (`api/heartbeat.rs:58`).
- Custody admin routes require a bearer token verified by comparing HMACs of the provided and expected tokens (length-independent, constant time) (`synergos-custody/src/handlers.rs:21-40`); health is also behind the token and tested (`tests/custody.rs:169-179`).
- Custody transfer routes require an HMAC-SHA256 capability bound to method, object key, expiry (ms) and download name (`handlers.rs:186-211`, `signature.rs:52-54`); key must be 64 hex (`handlers.rs:200`); a PUT capability used with GET is rejected (tested `tests/custody.rs:180-187`).
- Custody CORS: requests with an `Origin` header are rejected (403) unless the origin exactly matches a configured origin; allowed methods `GET, PUT, OPTIONS`, allowed headers limited; `Vary: Origin` (`handlers.rs:52-85`); tested (`tests/custody.rs:188-217`).
- UI: login is validated by an actual API call before the token is stored (`views/login.rs:17-47`); token input is `type=password`, `autocomplete=off` (`views/login.rs:63-70`); token held in `sessionStorage` only, cleared on sign-out and on any 401 (`session.rs:24-35`, `app.rs:20-24, 51-54`); UI never sends `revoke_dark: true` (`api/mod.rs:52-55, 73-83`).

### 3.2 Cryptography
- Node key: 244 bits from `uuid::Uuid::new_v4()` (OS RNG via `getrandom` [inference]) (`api/heartbeat.rs:27-34`); hash blake3 (`:36-39`).
- Custody: SHA-256 for object keys and content verification (`signature.rs:5-7`, `store.rs:143-170, 184-203`); HMAC-SHA256 for capabilities, receipts and token compare (`signature.rs:33-51`); shared fixture test with Curare for the receipt signature (`signature.rs:60-73`).
- Cloudflare client uses rustls over HTTPS and refuses non-HTTPS `api_base` except loopback (`cloudflare/client.rs:263-294`, `Cargo.toml:15`).

### 3.3 Input validation
- Org slug `[a-z0-9-]{1,64}` (`api/orgs.rs:66-79`); members normalized (trim, lowercase, dedupe) and must contain `@` (`api/orgs.rs:81-95`).
- `mesh_ip` must be IPv4 in `100.96.0.0/12` for both PATCH and heartbeat (`api/mod.rs:107-118`, `api/nodes.rs:222-225`, `api/heartbeat.rs:83-87`).
- Heartbeat `peer_id` / `synergos_version` length and control-character checks, rejected before any mutation (tested `api/heartbeat.rs:253-266`).
- Request-scoped Cloudflare token restricted to ASCII graphic 1..=512 bytes to prevent header injection (`api/mesh_setup.rs:229-241`, tests `:247-258`); router test asserts 400 before any upstream call (`api/mod.rs:206-215`).
- Cloudflare path segments restricted to `[A-Za-z0-9_-]` (`cloudflare/client.rs:296-308`).
- Custody admission validation (`contract.rs:17-34`); download name whitelist (`handlers.rs:155-163`), so the `Content-Disposition` header cannot be injected (`handlers.rs:269`); PUT must present exact `If-None-Match: *`, `X-Content-SHA256`, `Content-Type` (`handlers.rs:222-235`); byte count cannot exceed `byteLength` while streaming, with overflow-checked addition (`store.rs:189-193`).
- UI path building percent-encodes path segments (`api/paths.rs:27-37`).
- Static UI serving: lexical traversal rejection plus `canonicalize` containment check that also defeats symlinks out of `dist` (`api/ui.rs:56-73, 92-113`, tests `:163-209`).

### 3.4 Limits
- 64 KiB body limit on all control routes (`api/mod.rs:96`) and custody admin routes (`lib.rs:40`).
- Cloudflare HTTP timeout 30 s, redirects disabled (`cloudflare/client.rs:43-47`).
- Custody PUT 1800 s timeout (`handlers.rs:237-242`); lock acquisition 10 s (`store.rs:72-74`); `max_object_bytes` explicit and capped at 2^53-1 (`config.rs:46-47`).
- Upload capability <=15 min, download <=900 s, receipt <=5 min (`handlers.rs:100, 120-123, 155, 172`).
- Admission expiry window <=24 h + 1 min (`contract.rs:29`).

### 3.5 Atomicity / durability / locking
- Registry: single `tokio::sync::Mutex<RegistrySnapshot>`; every mutation clones, persists to `<path>.<uuid>.tmp` created with `create_new`, chmod `0600` (unix), `write_all`, `sync_all`, then `rename` over the live file; memory is updated only after persistence succeeds (`json_store.rs:44-82, 90-103, 202-238`); tests `failed_persist_does_not_change_memory_state` (`:350-356`).
- Heartbeat validation and mutation inside the same lock to avoid accepting an old key right after reissue (`api/heartbeat.rs:52-58`).
- Node registration cleans up the Cloudflare connector if token fetch or store insert fails (`api/nodes.rs:128-134, 152-162`; test `:347-411`).
- Custody: process-wide OS file lock `custody.lock` via `File::try_lock` (Rust 1.89) refuses a second store on the same root (`store.rs:33-39`; tested `tests/custody.rs:118-121`); startup reaps `<uuid>.part` leftovers and performs a publication probe (`store.rs:41-51`).
- Per-object async mutex serializes admit/verify/head/download/put/get/reclaim for the same key (`store.rs:58-75`; used at `handlers.rs:112, 138, 166, 220, 251`, `store.rs:96-98, 211`).
- Durable publish: write to `<uuid>.part`, `flush`, `sync_all`, then same-directory `hard_link` (create-if-absent) followed by directory `fsync` on unix, or `MoveFileExW(MOVEFILE_WRITE_THROUGH)` without `REPLACE_EXISTING` on Windows (`durable_file.rs:19-77`); `PartialFile` Drop removes the temp (`durable_file.rs:9-18`).
- Admissions are immutable and idempotent per `operationId`; different content for the same operation or same object -> 409 (`store.rs:94-129`; tests `tests/custody.rs:133-145`).
- Receipt issuance re-reads and re-hashes the full object and requires `sync_all` (`store.rs:143-170`, `handlers.rs:117-119`).
- Tombstone written before data deletion; admission kept so old PUT capabilities cannot recreate data; survives restart (`store.rs:210-222`; tested `tests/custody.rs:146-163`).

### 3.6 Secret handling / logging hygiene
- Secrets only from env var names in TOML (`config.rs:8-11, 22-24, 47-50`); `ResolvedSecrets`, `AdminToken`, `RequestScopedToken`, `MeshReconcileRequest`, `ConnectorTokensRequest`, `ApiClient` all have redacting `Debug` impls (`config.rs:83-87`, `auth.rs:33-37`, `api/mesh_setup.rs:26-59`, `admin-ui api/client.rs:41-45`; test `api/mesh_setup.rs:261-280`).
- Internal/Cloudflare error details are logged server-side and replaced with generic messages for clients (`error.rs:44-63`).
- Responses are `no-store` (control `api/mod.rs:120-129`; custody `handlers.rs:41-51` also sets `Referrer-Policy: no-referrer`).
- Custody receipts and store metadata never contain the token or secret (`config.rs:5`; test `tests/custody.rs:100`).
- Custody io errors are logged with only the error kind (`error.rs:17-22`).
- UI clears the Cloudflare API token input after use (`views/mesh_setup.rs:125`).

---

## 4. Weaknesses, TODOs, missing controls FOUND

Severity is a guess for the design phase. "Exploitable" means reachable by a party without full admin credentials or causing concrete damage with ordinary credentials; "design gap" means a missing control.

### Security theme

W1. Single static shared admin token is the only identity and the only authorization boundary. [design gap, high]
- `auth.rs:12-28` compares one env-derived string; `api/mod.rs:42-77` places every org/node/reconcile/mesh route behind it. There are no per-admin identities, no roles, no per-org scoping, no expiry, no rotation without restart (`config.rs:119-133`), no Cernere integration (grep of repo: no references). Docs acknowledge "管理トークンが唯一の防壁" (`docs/mesh-operations.md:126-131`, `docs/admin-ui.md:68-69`). A leaked token gives full read/write over every org, every node key reissue, every Cloudflare action and (with `revoke_dark`) the ability to revoke every device in the Cloudflare account.

W2. No audit trail with a principal for admin or destructive actions; custody has no operation logging at all. [design gap, high]
- Control logs only `info!` lines with ids (`api/nodes.rs:135, 256, 293`, `api/reconcile_api.rs:52-58`, `api/mesh_setup.rs:104-107, 186`) and no actor (there is none). Custody `admit`, `verify`, `download`, `reclaim` emit no log line (`handlers.rs:91-185`); only io failures produce `warn!` (`error.rs:19`). A reclaim (data deletion) leaves no record of when or by whom. Additionally the default log filter `synergos_control=info` (`main.rs:44-49`) does not enable the `synergos_custody` target, so custody warnings are dropped unless `RUST_LOG` is set [inference about `EnvFilter` semantics].

W3. No rate limiting or lockout anywhere. [design gap, medium]
- Grep for `rate`, `governor`, `tower_http` in the three crates returns nothing. Admin token guessing, heartbeat key guessing, custody token guessing, and capability-URL brute force are bounded only by key entropy and CPU.

W4. Unauthenticated heartbeat path takes the global registry lock and clones the whole registry before verifying the key. [exploitable DoS by anyone who can reach the listener, medium]
- `api/heartbeat.rs:56-58` calls `try_mutate_node`, which does `let mut next = state.clone()` (`json_store.rs:211-212`) before the closure checks the key (`api/heartbeat.rs:59-65`). Each junk POST costs an O(N) clone under the mutex; on success it also costs a full JSON serialize + fsync + rename (`json_store.rs:44-82, 235`). When `bind_addr` is widened to the Mesh IP as the docs require for heartbeat (`docs/mesh-operations.md:126-131`), every Mesh participant (including not-yet-revoked dark nodes) can do this.

W5. Heartbeat `mesh_ip` and `peer_id` are self-attested; the control plane never consults the TCP peer address. [design gap, medium]
- `api/heartbeat.rs:83-92` stores `req.mesh_ip` and overwrites `synergos_peer_id` with the client's claim; no `ConnectInfo` extractor is used anywhere in `api/`. The "peer_id <-> Mesh IP 照合" promised at `docs/mesh-operations.md:193-218` therefore only compares two values the node itself (or whoever holds its key) supplied, against an optional admin-entered expectation.

W6. Registry is a parallel identity store of personal data (emails) in a plaintext JSON file, contrary to the org rule that identity lives in Cernere and service DBs hold no personal data. [design gap, high]
- `store/mod.rs:13-15, 35-36` persist member and owner emails; `api/orgs.rs:40-42` returns all of them to any token holder; reconcile reports carry `owner_email` / `user_email` (`reconcile.rs:51-57`, `cloudflare/mod.rs:27-28`); file defaults to cwd-relative path (`config.rs:60-62`). Email validation is just `contains('@')` (`api/orgs.rs:85`, `api/nodes.rs:91`).

W7. Registry membership is never enforced on the data plane; prevention/detection/exclusion all depend on Cloudflare and on an operator running reconcile by hand. [design gap, high]
- No code in `synergos-control` exports an allowlist or is consumed by `synergos-net`/`synergos-core` (only the heartbeat client exists in core: `synergos-core/src/control_heartbeat.rs`). `docs/mesh-operations.md:220, 296-303` recommend periodic manual `curl`; there is no scheduler in `main.rs`. A node that enrolled via a shared connector token keeps full P2P reachability until someone runs `revoke_dark`.

W8. "Connector token reissue" does not rotate anything; a leaked connector token stays valid for the connector's lifetime. [design gap, medium; inference about Cloudflare semantics]
- `api/nodes.rs:263-277` and `api/mesh_setup.rs:162-172` only re-call `GET warp_connector/{id}/token` (`cloudflare/client.rs:165-173`), which returns the same long-lived tunnel token. The UI/doc wording "再発行" (`docs/mesh-operations.md:159, 266-267`) suggests revocation that does not happen. The only revocation path is deleting the node (`api/nodes.rs:249-261`). Tokens are also embedded in `enroll_command` strings in the bulk response (`api/mesh_setup.rs:168-172`), increasing copy/paste exposure.

W9. Admin API, heartbeat, static UI and the public-facing custody transfer endpoint share one listener and one bind address; there is no TLS in process. [design gap, high]
- `main.rs:76-82` merges everything on `config.bind_addr`; custody's `PUBLIC_URL` is meant for browsers (`spec/feature/opaque-custody.md:50`, `handlers.rs:191-197`). Making `/v1/custody/transfer/*` reachable from end users also exposes `/v1/orgs`, `/v1/heartbeat`, `/v1/mesh/*` and `/ui/` to the same network position, with the admin token as the only wall (`docs/admin-ui.md:68-69`). No path-level listener split or built-in reverse-proxy allowlist exists.

W10. Custody authority is a single static bearer that can admit, read metadata, mint download URLs and irreversibly reclaim any object; receipts are symmetric HMACs under the same secret that the verifier (Curare) holds. [design gap, high]
- `handlers.rs:21-40` (one token), `handlers.rs:175-185` + `store.rs:210-222` (unconditional reclaim), `handlers.rs:124-131` (receipt signed with `receipt_secret`), and Curare verifies with the same secret (`curare-server/src/storage/durable-receipt.ts:11-21`). Any party holding the secret (Curare itself, or whoever reads its env) can forge "durable" receipts, so a receipt does not prove Synergos stored anything. There is no key id, so neither token nor secret can be rotated without a flag day (`config.rs:28-29`).

W11. Capability and receipt secrets have no versioning; rotation invalidates all in-flight URLs/receipts. [design gap, medium]
- `signature.rs:33-54` uses one secret with no `kid`; `handlers.rs:186-211` verify against `state.config.receipt_secret` only.

W12. `reclaim` writes a tombstone for any id, even ids that were never admitted, and unlinks data immediately with no retention window. [exploitable with the custody token, medium]
- `store.rs:212-214` creates `<key>.reclaimed` unconditionally; `store.rs:215-219` removes `.data`. A compromised Curare credential (or a Curare bug) can delete every stored asset instantly and pre-tombstone arbitrary future ids (`store.rs:80-82` turns them into permanent 409s). No trash, no delayed GC, no "referenced by a version" check on the custody side (the spec delegates that decision entirely to Curare: `spec/feature/opaque-custody.md:41`).

W13. A `.data` file whose size no longer matches the admission makes the object permanently unusable: every read returns 409, PUT cannot overwrite (hard link create-if-absent), and the only exit (reclaim) tombstones the id forever. [design gap, medium]
- `store.rs:133-140` returns `Conflict` on size mismatch; `store.rs:175-177` returns `Precondition` only when sizes match, else `Conflict` propagates; `durable_file.rs:45` fails if destination exists; `store.rs:80-82` blocks re-admission after tombstone. No admin repair/scrub endpoint exists.

W14. Metadata is never garbage-collected: expired never-uploaded admissions, `.operation` files and tombstones accumulate forever in one flat directory. [design gap, medium; transfer/ops]
- `store.rs:94-129` writes two files per admission; nothing deletes `.json`/`.operation`; `store.rs:220` deliberately keeps them after reclaim; `store.rs:76-78` puts everything in `root/`. With Curare issuing an admission per upload attempt, directory entries grow without bound.

W15. A slow PUT holds the per-object lock for up to 30 minutes, making verify/head/download/reclaim for that object return 503 after a 10 s wait; concurrent partial uploads have no global cap or disk quota. [exploitable DoS by a capability holder, medium]
- `handlers.rs:219-242` acquires the lock before streaming; `store.rs:72-74` other callers time out to `Unavailable`; `store.rs:178-197` writes a `.part` of up to `byteLength` bytes per in-flight PUT; nothing limits the number of simultaneous PUTs or total bytes on disk beyond `max_object_bytes` per object (`config.rs:46-47`). The same capability URL can be replayed repeatedly until expiry (`handlers.rs:199-211`, no nonce).

W16. No browser hardening headers on the admin UI or API responses: no `Content-Security-Policy`, `X-Frame-Options`, `X-Content-Type-Options`, `Strict-Transport-Security`. [design gap, medium-low]
- `api/mod.rs:120-129` sets only cache headers; `api/ui.rs:81-88` sets only `Content-Type`. The admin token lives in `sessionStorage` (`session.rs:24-28`), which any injected script could read. Custody GET sets `Content-Disposition: attachment` and octet-stream (`handlers.rs:258-271`) but not `nosniff`.

W17. Admin-supplied fields are weakly bounded: `display_name`, org `name`, `members` count and `synergos_peer_id` (register/PATCH) have no length or printable-character checks, unlike the heartbeat path. [design gap, low]
- `api/nodes.rs:14-29, 104-119, 227-244`, `api/orgs.rs:12-24` rely solely on the 64 KiB body limit (`api/mod.rs:96`). `display_name` flows into UI tables and `issue_connector_tokens` responses.

W18. Registry durability gap: no directory `fsync` after `rename`, and the temp file is created with umask permissions before `chmod 0600`. [design gap, low]
- `json_store.rs:51-63` (create then set_permissions), `json_store.rs:71-73` (rename without fsyncing the parent directory). Custody does fsync the directory (`durable_file.rs:46-51`); control does not. On Windows no permission restriction is applied at all.

W19. Reconcile reads at most 1000 connectors and 1000 device registrations and never pages; participants beyond page one are silently invisible, i.e. dark nodes could go undetected and `revoke_dark` would act on an incomplete picture. [design gap, medium; inference about Cloudflare page limits]
- `cloudflare/client.rs:153-162, 214-220` hard-code `per_page=1000` with no `page`/`cursor` loop.

W20. `revoke_dark` is account-wide and one-shot with no dry-run token, no diff approval, no two-person rule and no rollback; a stale or wrong registry revokes real users' devices. [design gap, medium]
- `api/reconcile_api.rs:60-62, 67-97` revokes every registration whose email is not in any org and deletes every connector not in the registry; docs warn but code does not enforce (`docs/mesh-operations.md:170-172`). Revocation of devices is one POST with all ids in the query string (`cloudflare/client.rs:239-250`): either all succeed or all fail, and the URL length grows linearly with the number of dark devices.

W21. Bearer secrets travel in plaintext HTTP whenever the listener is widened beyond loopback; the daemon explicitly permits plaintext to Mesh IPv4 targets. [design gap, low-medium]
- `config.rs:14-16` default loopback; docs instruct binding to the Mesh IP for heartbeat (`docs/mesh-operations.md:126-131`); `synergos-core/src/control_heartbeat.rs:56-60` allows `http://` to Mesh addresses; the UI sends the admin token and the Cloudflare API token in requests to the same origin (`api/client.rs:82-84`, `api/mod.rs:63-71` in admin-ui). Confidentiality then depends entirely on the WARP tunnel.

W22. UI logout and token invalidation are client-side only; there is no server-side session, idle timeout, or revocation of the admin token short of restarting the server with a new env var. [design gap, low]
- `session.rs:31-35`, `app.rs:20-24`; server has no session state (`auth.rs`).

W23. Request-scoped Cloudflare token path lets any admin-token holder route an arbitrary high-privilege Cloudflare token through the control server, and the only functional check is `user/tokens/verify` plus a list call (`api/mesh_setup.rs:91-114`); the required scopes listed in `control.example.toml:6-7` are never verified. [design gap, low]

W24. `InvalidRequest` errors echo caller-supplied strings (emails, org ids) back verbatim in JSON (`api/nodes.rs:97-99, 214-216`, `api/orgs.rs:85-88`), and `Conflict` errors from the store echo owner emails (`json_store.rs:114-117, 156-159, 230-233`). Not an injection (JSON-encoded) but leaks membership facts to anyone with the token and supports probing. [low]

W25. Custody `verify` re-hashes the entire object on every call with no caching; a token holder can trigger O(size) disk reads repeatedly. [design gap, low]
- `store.rs:143-170`; `handlers.rs:117-119`.

W26. Receipt canonicalization is a bespoke format rather than RFC 8785 JCS; cross-language divergence (e.g. string escaping of U+2028/U+2029 or non-ASCII, number formatting) would silently invalidate signatures. [design gap, low; inference]
- `signature.rs:8-32` uses `serde_json` `to_string` per scalar; Curare has its own implementation (`durable-receipt.ts`). Only one shared fixture is tested (`signature.rs:60-73`).

W27. Heartbeat and PATCH both allow `synergos_peer_id` to be changed at will by the key holder/admin, with only an info log (`api/heartbeat.rs:88-91`, `api/nodes.rs:236-238`); there is no binding between node key and peer identity, and no key expiry. [design gap, low]

W28. Org/node updates have no optimistic concurrency (no version/ETag): two admins (or UI + CLI) can lose updates on `PUT /v1/orgs/:org` which replaces `members` wholesale (`api/orgs.rs:51-64`). [other, low]

W29. `GET /v1/custody/health` requires the bearer token (`lib.rs:30-39`, tested `tests/custody.rs:169-179`), so infrastructure probes and load balancers cannot check readiness without the Curare credential. [other/ops, low]

### Transfer theme

W30. Custody transfer is single-stream, non-resumable, non-parallel, with no `Range`/`ETag` support on GET and no multipart on PUT; an interrupted multi-GB upload restarts from zero and a 15-minute upload capability must be re-minted via a new admission call. [design gap, medium]
- `handlers.rs:212-273`; `store.rs:171-209`; spec acknowledges "バイト位置からの途中再開ではなく、未完成オブジェクト全体の再送" (`spec/feature/opaque-custody.md:40`).

W31. GET streams through `ReaderStream::new` with the default 4 KiB capacity [inference from tokio-util default] and PUT writes each hyper body chunk with an unbuffered `write_all` (`handlers.rs:257`, `store.rs:195`); for multi-GB game assets this is syscall-heavy. [performance, low-medium]

W32. The object key is `sha256(opaqueObjectId)`, not the content hash: identical content under two ids is stored twice; there is no chunking, deduplication, delta or compression anywhere in custody. [design gap, medium; ai-game/transfer]
- `signature.rs:5-7`, `store.rs:94-129`.

W33. Custody is a single-node local-filesystem blob store with no replication, scrub or backup, yet advertises `durability: "durable"` in health and receipts which Curare treats as a policy literal. [design gap, medium]
- `handlers.rs:88, 126`; `store.rs:130-140` (`present()` checks only size on read; bit-rot is served); spec narrows "durable" to "single node persistent volume" (`spec/feature/opaque-custody.md:26-31`); Curare requires the literal (`receipt-policy.ts:13`).

W34. PUT requires byte-exact `Content-Type` and `X-Content-SHA256` headers and the capability query struct rejects any extra query parameter; intermediaries that normalize `Content-Type` (e.g. appending `; charset=`) or append tracking parameters break uploads. [design gap, low]
- `handlers.rs:222-235`; `contract.rs:57-64` (`deny_unknown_fields`).

### AI/game theme

W35. The control plane models only "org -> node -> owner email"; there is no concept of project, dataset, GPU/render node class, storage capacity, or asset-type policy, and nothing relates a registered node to the content it is allowed to serve or receive. [design gap, medium]
- `store/mod.rs:9-63`; custody stores no tenant/org identifier on objects (`contract.rs:8-15`), so multi-tenant isolation in custody is impossible to express.

W36. The admin UI delivers `node_key` and `connector_token` to the browser DOM/clipboard (`api/models.rs:90-98`, `views/nodes/registered_secrets.rs` per `docs/admin-ui.md:92`), and a bulk endpoint returns many connector tokens at once (`api/mesh_setup.rs:132-192`); provisioning secrets for unattended render/build fleets by copy-paste does not scale and widens exposure. [design gap, low]

---

## 5. Performance characteristics

Control plane:
- Store: one global `tokio::sync::Mutex` over the whole registry (`json_store.rs:14-17`). Every write clones the snapshot (`json_store.rs:98, 119, 167, 212, 264`), serializes pretty JSON (`:45`), writes a temp file, `sync_all`, `rename` (`:51-73`). Every read of a node or org clones that record (`:131-140, 251-260`); `snapshot()` clones everything (`:84-86`).
- Heartbeat: each node POSTs every `interval_secs` (default 60 per `docs/mesh-operations.md:202`; daemon `control_heartbeat.rs:84-86`) and each accepted heartbeat performs a full registry rewrite + fsync; total write volume is O(N^2) bytes per interval. Rejected heartbeats still clone under the lock (`json_store.rs:211-212`).
- Reconcile: 2 serial Cloudflare GETs (`reconcile_api.rs:41-42`) with 30 s timeout each, then `classify` which is O(C*N) for connectors (`reconcile.rs:82-86`, linear `find` per connector) plus O(N*C) for missing (`:110-119`) and O(R*O*M) for registrations (`:124-151`). Revocation is 1 POST + 1 DELETE per dark connector, serial (`reconcile_api.rs:67-97`).
- Register mesh node: 2 serial Cloudflare calls (create + token) then 1 persist (`api/nodes.rs:123-162`); delete: 1 Cloudflare call + 1 persist.
- Hashing: blake3 for node keys (`api/heartbeat.rs:37-39`).
- Cloudflare client: HTTP/2 capable (`Cargo.toml:15`), 30 s timeout, no retry/backoff, no pagination.
- UI serving: `std::fs::read` and `canonicalize` are blocking calls inside async handlers (`api/ui.rs:61-71`), the whole asset (multi-MB WASM) is loaded into memory per request, no ETag/conditional GET, and the global `no-store` header (`api/mod.rs:97, 120-129`) forces browsers to re-download the WASM bundle on every load.
- Body limit 64 KiB for all control routes (`api/mod.rs:96`).

Custody:
- PUT: streamed; per-chunk `write_all` to `tokio::fs::File` (no `BufWriter`) and SHA-256 update (`store.rs:184-197`); then `flush`, `sync_all`, `spawn_blocking` hard link + directory fsync (`store.rs:204-207`, `durable_file.rs:35-53`). Wall-clock cap 1800 s (`handlers.rs:237-242`).
- verify: full sequential re-read with 128 KiB buffer + SHA-256 + `sync_all` (`store.rs:143-170`), i.e. O(size) disk and CPU per receipt.
- GET: `tokio::fs::File` + `ReaderStream::new` default capacity (4 KiB [inference]) (`handlers.rs:256-257`); `Content-Length` from admission; lock released when the handler returns, before the body finishes streaming.
- Metadata: 2-4 small files per object in one flat directory (`store.rs:76-78`); `admission()` costs `try_exists` + `read` + JSON parse per call (`store.rs:79-93`); `present()` one `symlink_metadata` (`:133-140`).
- Locks: per-key `Arc<AsyncMutex<()>>` kept via `Weak` in a `HashMap`, pruned when >1024 entries (`store.rs:58-75`); 10 s acquisition timeout.
- Round trips per upload: Curare -> custody `admissions` (1), client -> custody `PUT` (1), Curare -> custody `verify` (1, full rehash). Per download: `download` (1) + `GET` (1).
- Hashing: SHA-256 (`sha2` 0.10; hardware acceleration where the crate detects it [inference]).

Admin UI: `gloo_net` fetch per call; no caching; `list_orgs` used as the login probe (`views/login.rs:27`).

---

## 6. Config knobs & defaults

| Knob | Default | Secure by default? |
|---|---|---|
| `bind_addr` (`config.rs:15-16, 56-58`) | `127.0.0.1:4250` | Yes, but docs require widening to the Mesh IP for heartbeat, which also exposes the admin API (`docs/mesh-operations.md:126-131`) |
| `store_path` (`config.rs:18-20, 60-62`) | `synergos-control-store.json` (cwd-relative) | Partly: file is `0600` on unix (`json_store.rs:57-63`); cwd-relative default risks landing in a shared directory; no Windows ACL |
| `admin_token_env` / `SYNERGOS_CONTROL_ADMIN_TOKEN` (`config.rs:22-24, 64-66, 120-127`) | required, >=32 bytes | Length check only; no entropy check; no rotation |
| `cloudflare.account_id` (`config.rs:45, 100-105`) | required non-empty | validated `[A-Za-z0-9_-]` at client construction (`cloudflare/client.rs:42`) |
| `cloudflare.api_token_env` / `CLOUDFLARE_API_TOKEN` (`config.rs:47-50, 128`) | required non-empty | no scope verification |
| `cloudflare.api_base` (`config.rs:52-53, 72-74`) | `https://api.cloudflare.com/client/v4` | HTTPS enforced except loopback (`cloudflare/client.rs:263-294`) |
| `ui.dist_path` (`config.rs:35-40, 106-115`) | none (`/ui/` -> 503) | fail-fast if missing |
| `RUST_LOG` (`main.rs:44-49`) | `synergos_control=info` | custody target not enabled by default [inference] |
| `SYNERGOS_CUSTODY_ENABLED` (`config.rs:18-22`) | off | off by default; must be exactly `1` |
| `SYNERGOS_CUSTODY_ROOT` | required absolute | ok |
| `SYNERGOS_CUSTODY_PUBLIC_URL` | required origin | http allowed (not forced https) (`config.rs:67-75`) |
| `SYNERGOS_CUSTODY_NODE_ID` | required UUID | ok |
| `SYNERGOS_CUSTODY_TOKEN` / `..._RECEIPT_SECRET` | required >=32 chars | length only; shared static; no kid |
| `SYNERGOS_CUSTODY_MAX_OBJECT_BYTES` | required, 1..=2^53-1 | explicit, good; no total quota |
| `SYNERGOS_CUSTODY_CLIENT_ORIGINS` | required exact origins | strict |
| Hard-coded: 64 KiB bodies (`api/mod.rs:96`, `lib.rs:40`), PUT 1800 s (`handlers.rs:238`), lock 10 s (`store.rs:72`), upload cap 15 min (`handlers.rs:100`), download cap 900 s (`handlers.rs:155`), receipt 5 min (`handlers.rs:123`), admission window 24 h (`contract.rs:29`), Cloudflare timeout 30 s (`cloudflare/client.rs:45`), `per_page=1000` (`cloudflare/client.rs:157, 217`), Mesh range `100.96.0.0/12` (`api/mod.rs:112`) | - | not tunable |

---

## 7. Test coverage observed

Control (`cargo test -p synergos-control`):
- `auth.rs:50-60`: constant-time equality basics.
- `api/mod.rs:131-222`: mesh routes 401 without/with wrong token; malformed CF token -> 400 before upstream; `/ui/` -> 503 without dist.
- `api/nodes.rs:308-435`: Cloudflare token failure deletes the new connector and does not persist the node (mock axum upstream); `NodeView` omits `node_key_hash`.
- `api/heartbeat.rs:110-295`: valid key accepted and fields recorded; wrong key / unknown node / empty key / missing header all 401 and do not mutate; node without key rejected; unprintable `peer_id` rejected without mutation; heartbeat does not clobber admin `mesh_ip`.
- `api/mesh_setup.rs:243-281`: token validation; Debug redaction.
- `reconcile.rs:165-295`: dark/missing connectors; device classification by membership (case-insensitive); revoked registrations ignored; mesh-ip mismatch flagged / not flagged.
- `cloudflare/client.rs:310-390`: revoke uses repeated query params and empty body; deleting an absent connector succeeds.
- `store/json_store.rs:277-368`: file round trip; duplicate org / unknown org rejected; member removal guard; failed persist leaves memory unchanged; remove scoped by org.
- `api/ui.rs:158-210`: traversal rejection; normal path resolution; MIME; symlink escape rejected.

Not tested in control: org handlers (`create_org`, `update_org`, `validate_slug`, `normalize_members`); `patch_node`; `remove_node` (incl. Cloudflare failure); `reissue_node_key`; `reissue_connector_token`; `run_reconcile`/`revoke_dark` action recording; `issue_connector_tokens`; `check_token` happy path; `list_mesh_connectors`/`list_device_registrations` parsing; `verify_token`; `ControlConfig::load/validate/resolve_secrets`; error response redaction; heartbeat through the full router (body limit, JSON rejection); concurrent heartbeat vs admin mutation; registry file permission; Cloudflare pagination; UI asset happy path with a real `dist`.

Custody (`cargo test -p synergos-custody`):
- `tests/custody.rs:70-128`: admit -> verify 404 before upload -> wrong bytes 409 -> correct PUT 201 -> duplicate PUT 412 -> receipt fields and 64-hex signature, token absent from response -> download URL -> GET bytes -> second store on same root fails -> reopened store still has the object.
- `tests/custody.rs:129-164`: idempotent re-admission; changed admission 409; reclaim twice OK; PUT after reclaim 409; persists across reopen; re-admission after reclaim 409.
- `tests/custody.rs:165-218`: health without token 403; GET with a PUT capability 403; OPTIONS with foreign origin 403; OPTIONS with allowed origin 204 + ACAO.
- `signature.rs:59-73`: shared receipt fixture with Curare; tampered message rejected.

Not tested in custody: `Config::validate` / `from_env`; `Admission::validate` boundaries (expiry >24 h, past expiry, bad hash prefix, oversize `byteLength`, CR/LF in content type); PUT with missing/wrong required headers (400); PUT exceeding `byteLength` (400); PUT on expired admission (409); expired capability (403); tampered signature over HTTP; `name` mismatch on GET; CORS on actual GET/PUT (only OPTIONS tested); lock timeout / concurrent PUT; `.part` reaping at startup; partial upload cleanup on client abort; `head` null for absent/reclaimed; download name validation; `verify` after admission expiry (409); `present()` size-mismatch 409; Windows publish path; large objects (max in tests is 1024 bytes, `tests/custody.rs:24`).

Admin UI: no `#[cfg(test)]` in `session.rs`, `api/client.rs`, `api/paths.rs`, `api/models.rs`, `app.rs`; the crate is excluded from the workspace (`Cargo.toml:15-20` at repo root) and only compile-checked in CI per `docs/admin-ui.md:39-44`.

---

## 8. Documented-vs-implemented deltas

1. `docs/mesh-operations.md:159, 266-267` and `api/nodes.rs:263` call `POST .../connector-token` a "再発行" (reissue). Implementation re-fetches the existing Cloudflare connector token (`cloudflare/client.rs:165-173`); nothing is revoked or rotated [inference about Cloudflare API semantics].
2. `docs/mesh-operations.md:303` claims `revoke_dark` makes the participant unreachable "Synergos 層でも". Control never informs the data plane; reachability loss is purely a consequence of Cloudflare Mesh membership. Must be confirmed against the transport map (relay/DHT paths outside the Mesh).
3. `docs/mesh-operations.md:220` recommends periodic reconcile but no scheduler exists (`main.rs`); `docs/mesh-operations.md:296-303` describes prevention/detection/exclusion as a system, while the code offers only manual endpoints.
4. `docs/mesh-operations.md:193-218` presents heartbeat as "peer_id <-> Mesh IP 照合"; implementation only stores self-reported values and compares them to an optional admin-entered `mesh_ip` (`api/heartbeat.rs:83-95`, `reconcile.rs:87-100`).
5. `control.example.toml:6-7` lists required Cloudflare token scopes; nothing validates them (`api/mesh_setup.rs:91-114` checks only `status == "active"` and one list call).
6. `spec/feature/opaque-custody.md:26-28, 31` defines "durable" narrowly (single node, fsync) and disclaims replication; the wire literal `durability: "durable"` (`handlers.rs:88, 126`) and Curare's policy (`receipt-policy.ts:13`) treat it as a guarantee. Consistent with the spec text, but the term overstates what the receipt proves (see W10, W33).
7. `spec/feature/opaque-custody.md:70` says router tests cover "偽造/用途違いURL拒否"; only wrong-method use is tested over HTTP (`tests/custody.rs:180-187`); forged signatures are covered only by the unit test on `verify` (`signature.rs:72`).
8. `spec/feature/opaque-custody.md:58` says secrets are injected via Infisical/Excubitor; neither crate integrates with Infisical; they read plain env vars (`config.rs:136-143`, custody `config.rs:61-66`). Injection is assumed to be external.
9. `spec/feature/opaque-custody.md:42` says the custody root is excluded from Synergos history GC; nothing in these crates references the history GC, so this is unverifiable here (open question for the core map).
10. `docs/admin-ui.md:16-19, 86-98` and `docs/mesh-operations.md:254-255, 280-285` accurately describe sessionStorage use, request-scoped CF token handling, and the UI not revoking (`session.rs`, `views/mesh_setup.rs:125`, `admin-ui api/mod.rs:52-55, 73-83`).
11. `docs/mesh-operations.md:148` ("health を除く") is accurate for control; it omits that `/ui/*` is also unauthenticated (documented separately in `docs/admin-ui.md:16`).
12. `docs/mesh-operations.md:355` says heartbeat is plaintext inside the Mesh; `synergos-core/src/control_heartbeat.rs:56-60` enforces plaintext only to loopback or Mesh IPv4, consistent.
13. `spec/feature/opaque-custody.md:73-75` records that Anatomia did not analyze Rust for spec linkage; the `@implements` tags exist (`lib.rs:1`, `handlers.rs:1-2`, etc.) but no tooling in this subsystem verifies them.

---

## 9. Open questions for designers

1. Identity: should the control plane consume Cernere-issued identities (OIDC/JWT with org claims) instead of a static bearer, and should org membership be *read from* Cernere rather than stored as emails here (W1, W6)?
2. Enforcement: which component should consume the registry to admit/deny peers on the data plane (QUIC handshake allowlist, DHT/gossip filtering, relay auth)? Who signs node identities (`synergos_peer_id`) so a heartbeat cannot reassign them (W5, W7, W27)?
3. Node credentials: should node keys become short-lived, bound to the peer public key and the socket source address, with an automatic renewal protocol, rather than copy-pasted 64-hex strings (W4, W5, W36)?
4. Listener segregation: should custody transfer, heartbeat, admin API and UI bind to separate addresses/ports (or separate processes) so that exposing one does not expose the others, with TLS terminated in-process or by a mandated proxy (W9, W21)?
5. Receipts: should receipts be Ed25519-signed by a per-custody-node key with a published key id, so Curare can verify but not forge, and so secrets can rotate (W10, W11)?
6. Reclaim: should custody enforce a retention/trash window and require a signed, logged deletion intent referencing the operation, instead of a bare object id (W12, W2)?
7. Scale: at what object count does the flat directory and per-admission file growth become a problem, and is a manifest/index (SQLite or similar) plus GC of expired admissions required (W14)?
8. Transfer: should custody adopt resumable chunked uploads (tus / multipart with per-chunk hashes), range GET, and content-addressed dedup to serve multi-GB game/AI assets, or should custody delegate large transfers to the Synergos P2P content layer with custody only as the durable anchor (W30-W32)?
9. Durability: what replication factor and scrub cadence must back the `durable` literal before Curare treats it as sufficient for release artifacts (W33)?
10. Reconcile: what are the real Cloudflare page limits, and should reconcile run as a scheduled job with a signed diff that an operator approves before `revoke_dark` (W19, W20)?
11. Observability: what audit event schema (actor, action, target, result, request id) should both crates emit, and where does it go (W2)?
12. Multi-tenancy in custody: should admissions carry an org/project id so storage quotas, isolation and reporting per tenant are possible (W35)?
13. Is the `history GC` exclusion for the custody root enforced anywhere in synergos-core (`spec/feature/opaque-custody.md:42`)?
