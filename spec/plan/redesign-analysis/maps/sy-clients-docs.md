# Synergos subsystem map: clients (Tauri, egui GUI, ars plugin), IPC usage, docs-vs-implementation

Scope read completely: `synergos-tauri/src-tauri/src/{main,lib,ipc,config}.rs`, `tauri.conf.json`, `capabilities/default.json`, `frontend/src/lib/tauri.ts`, `frontend/src/pages/{Settings,Dashboard}.tsx`, `frontend/src/components/*.tsx`, `frontend/src/{App,main}.tsx`, `frontend/{package.json,index.html,vite.config.ts}`; `synergos-gui/src/{connection,app,lib,main}.rs`, `synergos-gui/src/ui/*.rs`, `synergos-gui/tests/ui_snapshot.rs`; `ars-plugin-synergos/src/{lib,bridge,events}.rs`; `synergos-ipc/src/{transport,client,command,response,event,lib}.rs` (the protocol every client speaks); `synergos-core/src/ipc_server.rs` lines 60-420, 505-620, 828-845, 1251-1340, 1441-1475, 1995-2070 (server-side trust model and the handlers the clients call); `DESIGN.md` sections 1-2, 4.4-4.5, 5, 6, 9, 13-14, 15.2-15.4 (heads), 15.6-15.8; `docs/{README,getting-started,operations-readiness,two-node-operations,platforms,setup-macos}.md`; `spec/tasks/*.md`; `review/2026-05-13/*`; `OPERATIONAL-TEST.md`; `SETUP.md`; `.github/workflows/{ci,harness}.yml`; `.cargo/config.toml`; root and every crate `Cargo.toml`; `Cargo.lock` (inventory greps).

All paths below are relative to `Synergos` unless absolute.

---

## 1. Responsibilities & data flow

Three client front-ends plus the shared IPC crate sit on top of the `synergos-core` daemon. None of them touches the network layer directly; every client action becomes an `IpcCommand` on a local Unix socket / Named Pipe.

| Component | Entry | What enters | What leaves | Calls |
|---|---|---|---|---|
| **synergos-tauri** (Tauri 2 + React 19 + Vite 8) | `src-tauri/src/lib.rs:199 run()` | User clicks in webview -> `invoke()` of 15 `#[tauri::command]` functions (`lib.rs:37-194`) | `IpcCommand` to daemon (`ipc.rs:265-268`), or direct TOML file writes (`config.rs:100-162`) | `synergos_ipc::IpcClient` (`ipc.rs:266`), `std::fs` for `synergos-net.toml`, `tauri_plugin_dialog` (file picker, `PublishModal.tsx:23`), `tauri_plugin_opener` (registered `lib.rs:208`, never used by the frontend) |
| **synergos-gui** (egui/eframe 0.31) | `src/main.rs:427` -> `app::SynergosApp` | Immediate-mode UI inputs (`app.rs:274-294 UiInputs`) | `IpcCommand` via `CoreConnection::send_command` (`connection.rs:114-121`), results cached in `ConnectionCache` (`connection.rs:24-33`) | `synergos_ipc::IpcClient`; a private 1-thread tokio runtime (`connection.rs:43-47`) driven by `block_on` from the UI thread |
| **ars-plugin-synergos** | library only (`src/lib.rs`) | `IpcEvent` values (caller must obtain them) | `BridgedEvent` enum (`bridge.rs:34-41`) | Nothing: no IPC connection, no Ars EventBus, no `ars-core` dependency (`ars-plugin-synergos/Cargo.toml:12-24`) |
| **synergos-ipc** (shared) | `IpcClient::connect()` (`client.rs:434`) | `IpcCommand` (`command.rs:12-214`) | `IpcResponse` (`response.rs:9-75`) and pushed `IpcEvent` (`event.rs:9-82`) multiplexed as `ServerMessage` (`transport.rs:49-52`) | tokio UnixStream / NamedPipeClient (`client.rs:452-488`) |

Daemon-side flow the clients depend on: `IpcServer::run` binds the socket (`ipc_server.rs:78-146`), rejects connections whose peer UID differs (`ipc_server.rs:114-119, 231-250`), runs `command.validate()` before dispatch (`ipc_server.rs:507-517`), handles `Subscribe`/`Unsubscribe` inline (`ipc_server.rs:520-549`) and everything else through `dispatch_command` (`ipc_server.rs:828`).

Tauri data flow in detail:
1. `frontend/src/lib/tauri.ts:85-129` wraps each command; React Query polls: `daemon_ping` every 3 s (`App.tsx:8-12`), `daemon_status` 5 s (`Dashboard.tsx:257-261`), `project_list` 5 s (`Dashboard.tsx:26-30`), `peer_list` 5 s (`Dashboard.tsx:182-187`), `transfer_list` 3 s (`Dashboard.tsx:94-98`).
2. Each invoke -> `call_daemon` -> fresh `IpcClient::connect()` + one `send` (`ipc.rs:265-268`); comment at `ipc.rs:232-238` explains the no-shared-client decision.
3. Settings page: `config_get`/`config_set`/`config_paths` read/write `synergos-net.toml` directly, bypassing the daemon (`lib.rs:181-194`, `config.rs:49-162`); the UI tells the user to restart the daemon (`Settings.tsx:317-321`).
4. Publish: file picker returns absolute paths (`PublishModal.tsx:23-32`) -> `project_publish` -> `IpcCommand::PublishUpdate` (`lib.rs:97-107`) -> daemon canonicalizes root and resolves each path under it (`ipc_server.rs:1255-1286`).

egui data flow: `refresh_if_needed` every 2 s does `Status`, `ProjectList`, `NetworkStatus` serially (`connection.rs:67-111`); tab panels call `refresh_peers` / `refresh_transfers` / `refresh_conflicts`; the Transfers tab refreshes on **every frame** (`ui/transfers.rs:46-47`). Mutations: `open_project`, `close_project`, `create_invite`, `peer_connect`, `peer_disconnect`, `cancel_transfer`, `request_transfer`, `resolve_conflict`, `update_config` (`connection.rs:146-254`).

Neither GUI ever sends `IpcCommand::Subscribe`; both are pure pollers (confirmed by grep: `Subscribe` appears only in `synergos-ipc` and `synergos-core`). The Tauri README states this (`synergos-tauri/README.md:57`).

---

## 2. Key types / APIs / wire formats

### IPC framing (all clients)
- Frame = 4-byte little-endian length + MessagePack payload (`transport.rs:7, 87-104`).
- `MAX_MESSAGE_SIZE = 1 MiB` enforced on write (`transport.rs:94-99`) and on read before allocation (`transport.rs:124-129`); payload read in `READ_CHUNK = 64 KiB` steps (`transport.rs:42, 133-145`).
- Serialization is `rmp_serde::to_vec` (positional, not named) (`transport.rs:92`); only two fields carry `#[serde(default)]` for compatibility (`command.rs:52 peer_info_url`, `response.rs:237 synergos_version`). No protocol version field exists anywhere in `IpcCommand`/`IpcResponse`/`DaemonStatus`.
- Server -> client envelope `ServerMessage::{Response, Event}` (`transport.rs:49-52`). Client correlates responses FIFO via a `VecDeque<oneshot::Sender>` (`client.rs:429, 498-512, 542-552`); events go to an **unbounded** mpsc (`client.rs:436, 553-557`).
- Socket path: Linux `$XDG_RUNTIME_DIR/synergos/synergos.sock`, fallback `/tmp/synergos/synergos.sock` (`transport.rs:56-62`); macOS `~/Library/Application Support/Synergos/synergos.sock` (`transport.rs:64-72`); Windows `\\.\pipe\synergos` (`transport.rs:74-77`).

### `IpcCommand` surface (what any local client can do) — `command.rs:12-214`
Daemon control `Ping`, `Shutdown`, `Status`; projects `ProjectOpen{project_id, root_path, display_name}`, `ProjectClose`, `ProjectList`, `ProjectGet`, `ProjectUpdate`, `ProjectCreateInvite{expires_in_secs: Option<u64>, peer_info_url: Option<String>}`, `ProjectJoin{invite_token, root_path}`; peers `PeerList`, `PeerConnect`, `PeerDisconnect`, `PeerAddByUrl{url}`; transfer `TransferRequest`, `TransferList`, `TransferCancel`, `PublishUpdate{file_paths: Vec<PathBuf>}`, `ProjectCheckout`, `ProjectRestore`; history `HistoryList`, `HistoryGc{purge}`, `HistoryRotate`, `HistoryOffloaded`, `HistoryFetch`; hooks `HooksList`, `HooksRun`; tags `TagAdd/TagLs/TagShow/TagRm`; conflicts `ConflictList`, `ConflictResolve{resolution: "keep_local"|"accept_remote"|"manual_merge"}`; `ConfigUpdate{mesh_n, max_concurrent_streams, tunnel_hostname}`; monitoring `NetworkStatus`, `Subscribe{events: Vec<EventFilter>}`, `Unsubscribe`; `PeerSendStream{peer_id, magic: [u8;4], payload: Vec<u8>}`.

`IpcCommand::validate()` (`command.rs:225-550`): ids non-empty and <= 1024 bytes; paths <= 4096 bytes, no `..` component, no NUL (`command.rs:243-264`); rel paths must be `/`-separated, not absolute, not `.synergos`, no control chars, no `:` segments (`command.rs:266-285`); tag names `[A-Za-z0-9._-]{1,64}` (`command.rs:288-303`); `PublishUpdate` 1..=1024 paths (`command.rs:369-387`); `PeerAddByUrl` must start with `http://` or `https://` (`command.rs:343-350`); `PeerSendStream` payload <= 1 MiB (`command.rs:535-548`); `ConfigUpdate` and `Subscribe` unvalidated (`command.rs:514, 533`).

### Response / event DTOs used by the clients
`DaemonStatus{pid, started_at, project_count, active_connections, active_transfers}` (`response.rs:186-197`), `ProjectInfo` (`response.rs:201-207`), `PeerInfo{..., synergos_version}` (`response.rs:227-239`), `TransferInfo` (`response.rs:243-252`), `NetworkStatusInfo{primary_route, total_bandwidth_bps, used_bandwidth_bps, active_connections, max_connections, avg_latency_ms}` (`response.rs:256-263`), `ConflictInfoDto` (`response.rs:172-182`), `InviteToken{token, expires_at}` (`response.rs:29-32`). Events: `PeerConnected`, `PeerDisconnected`, `TransferProgress`, `TransferCompleted`, `TransferFailed`, `ConflictDetected`, `ConflictResolved`, `NetworkStatusUpdated`, `PeerStreamReceived{magic, payload}` (`event.rs:9-82`); filters `All | Project(String) | Category(Peer|Transfer|Conflict|Network|PeerStream)` (`event.rs:86-104`).

### Tauri bridge types
- `BridgeError` tagged enum `{kind: daemon_not_running|daemon|io|config|unexpected, message}` (`ipc.rs:243-257`), mirrored in TS (`tauri.ts:74-82`). `Unexpected` embeds `format!("{:?}", resp)` of the whole daemon response (`ipc.rs:260-262`).
- `ConfigFile{bootstrap_urls, force_relay_only, auto_promote, quic_listen_addr, peer_info_listen_addr}` (`config.rs:24-36`), `ConfigPaths{config_path, identity_path, projects_path}` (`config.rs:15-20`).
- TS `DaemonStatus` has an index signature `[key: string]: unknown` (`tauri.ts:13`) to tolerate new fields.

### Tauri app config
- `tauri.conf.json`: identifier `io.ludiars.synergos.tauri`, single window 1200x760, `security.csp: null` (`tauri.conf.json:276-278`), `bundle.active: true, targets: "all"` (`:280-287`), `devUrl http://127.0.0.1:5173`.
- `capabilities/default.json`: window `main`, permissions `core:default`, `opener:default`, `dialog:default` (`:293-299`).
- Plugins registered: `tauri_plugin_opener`, `tauri_plugin_dialog` (`lib.rs:208-209`).

### ars plugin types
`BridgedEvent::{PeerConnected, PeerDisconnected, TransferProgress, TransferCompleted, NetworkStatus, ConflictDetected}` (`bridge.rs:34-41`); `bridge_event` maps 6 of 9 `IpcEvent` variants and returns `None` for `TransferFailed`, `ConflictResolved`, `PeerStreamReceived` (`bridge.rs:118`); `NetworkStatusUpdated.max_connections` is hardcoded `0` (`bridge.rs:101`).

---

## 3. Security controls PRESENT (with evidence)

IPC transport / daemon-side trust boundary (what the clients rely on):
1. Unix socket `chmod 0600` after bind (`synergos-core/src/ipc_server.rs:91-98`), stale socket removed before bind (`:86-87`).
2. Peer-credential UID check on every accepted connection: Linux `SO_PEERCRED` (`ipc_server.rs:252-270`), macOS/BSD `getpeereid` (`:272-290`), rejection logged and stream dropped (`:114-119`). Best-effort fallthrough on other Unix (`:292-307`) and when credentials unavailable (`:235-241`).
3. Windows: `first_pipe_instance(true)` to reserve the pipe name (`ipc_server.rs:164-169`); caller SID compared with daemon SID via `GetNamedPipeClientProcessId` -> `OpenProcess` -> `OpenProcessToken` -> `GetTokenInformation` -> `EqualSid` (`:332-375`); handles closed via RAII/explicit `CloseHandle` (`:377-398`). All `unsafe` blocks carry `// SAFETY:` comments (`:242, 254, 280, 305, 356, 368, 389, 395, 403, 418`).
4. Frame size limit 1 MiB on both directions and staged allocation (`synergos-ipc/src/transport.rs:39, 94-99, 124-145`).
5. Input validation before dispatch (`ipc_server.rs:507-517` calling `command.rs:225-550`): empty/oversize ids, `..` traversal, NUL bytes, `.synergos/` protection, tag-name charset, publish batch cap, URL scheme allow-list, stream payload cap.
6. Publish path containment: project root canonicalized, each path resolved under it, whole batch validated before any hook runs, paths re-resolved after `pre-publish` hooks and identity re-checked (`ipc_server.rs:1255-1322`); error messages redact paths (`:1333-1336`).
7. Accept-loop exponential backoff on `accept()` errors (fd exhaustion) capped at 1 s (`ipc_server.rs:104-105, 128-133, 172-173, 209-213`).
8. Client-side: `ConnectionRefused`/`NotFound` mapped to `DaemonNotRunning` instead of leaking raw errors (`client.rs:455-463`); pending responders cleared on disconnect so callers get `ConnectionClosed` (`client.rs:561-564`); writes serialized under a mutex with FIFO correlation (`client.rs:502-508`).
9. Per-connection event relay is aborted on `Unsubscribe` and on disconnect (`ipc_server.rs:521-524, 544-548, 557-559`).

Tauri client:
10. Only an explicit allow-list of 15 commands is exposed (`lib.rs:210-226`); the frontend uses typed wrappers only (`tauri.ts:85-129`); no `withGlobalTauri`, no `dangerousRemoteDomain`, no custom protocol beyond defaults (grep of `tauri.conf.json`).
11. No `dangerouslySetInnerHTML` / `innerHTML` / `eval` in the React code (grep of `frontend/src`); daemon-supplied strings are rendered through JSX text nodes (`Dashboard.tsx:63-65, 228-240`).
12. Config save preserves untouched keys by editing a `toml::Value` table rather than regenerating from the partial struct (`config.rs:106-157`); `create_dir_all` on the parent (`config.rs:102-104`).
13. Vite dev server bound to `127.0.0.1` with `strictPort` (`frontend/vite.config.ts:9-11`); sourcemaps off in production build (`:20`).
14. Tauri bridge uses short-lived connections, avoiding shared-state races between concurrent commands (`ipc.rs:232-238`).

egui client:
15. `UiInputs::new_project_root_path` trims and rejects empty paths (`app.rs:401-408`); all mutation helpers map non-`Ok` responses to `false` so a daemon rejection is never shown as success (`connection.rs:146-254`).
16. Invite expiry is user-settable (`ui/overview.rs:160-163`).

Supply chain / CI:
17. Workflow `permissions: contents: read` (`.github/workflows/ci.yml:9-10`); `RUSTFLAGS: -D warnings` (`:14`); fmt + clippy `-D warnings` gates (`:17-54`); 3-OS test matrix (`:56-86`); `cargo audit` run on both lockfiles (`:157-173`); `Cargo.lock` committed (229 KB, 865 packages) with **zero git-sourced dependencies** (grep `source = "git` = 0); frontend `package-lock.json` lockfileVersion 3 present; `synergos-admin-ui/Cargo.lock` present.
18. No `unsafe` in any client crate (`synergos-tauri`, `synergos-gui`, `ars-plugin-synergos`: grep = 0); `synergos-ipc` has 5 `unsafe` occurrences, all in `#[cfg(test)]` env-var manipulation serialized by `serial_test` (`transport.rs:273-318`).

---

## 4. Weaknesses, TODOs, missing controls FOUND

Severity is my estimate. "Exploitable" = reachable today; "design gap" = missing capability.

### Security

**S-1 (high, exploitable on shared hosts) — Linux `/tmp` socket fallback allows socket hijack; clients never authenticate the server.**
`transport.rs:58` falls back to `/tmp/synergos/synergos.sock` when `XDG_RUNTIME_DIR` is unset. The daemon does `create_dir_all` on `/tmp/synergos` with default mode (`ipc_server.rs:82-84`) and ignores `remove_file` failure (`:87`). If another local user pre-creates `/tmp/synergos` (no sticky bit), they can unlink the daemon's socket and bind their own; every client (`client.rs:452-465`) connects by path with no `SO_PEERCRED`/owner check of the server, so the attacker receives every command the user's CLI/GUI sends (including `ProjectJoin{invite_token}` bearer tokens and project paths) and can feed arbitrary `IpcResponse`/`IpcEvent` data back. `SETUP.md:302-305` and `two-node-operations.md:257` document exactly the systemd scenario where `XDG_RUNTIME_DIR` is unset and the socket lands in `/tmp`. Redesign: refuse to start without a private runtime dir (or create `/tmp/synergos-<uid>` with 0700 and verify ownership), and have clients verify the listener's UID (`SO_PEERCRED` on the connected stream / `getpeereid`) before sending anything.

**S-2 (medium, exploitable) — Windows named-pipe name squatting and no client-side server verification.**
Any local user can create `\\.\pipe\synergos` before the daemon starts; the daemon's `first_pipe_instance(true)` then fails (`ipc_server.rs:165-169`, DoS) and clients happily connect to the squatter (`client.rs:467-488`) with no `SECURITY_IDENTIFICATION`-level impersonation guard or server SID check. The pipe has no DACL (`ipc_server.rs:313-315`), so other users can also consume listening instances in a loop (`:185-201`). Redesign: per-user pipe name (`\\.\pipe\synergos-<SID>`), explicit DACL via `ServerOptions::security_attributes` (or raw `CreateNamedPipeW`), and client-side `GetNamedPipeServerProcessId` + SID comparison.

**S-3 (high, design gap; exploitable by any same-user code) — IPC authorization is "same UID = full admin"; no per-client capability, confirmation, or audit.**
Every `IpcCommand` is available to any process running as the user: `Shutdown` (`ipc_server.rs:832-836`), `ProjectOpen` of any directory then `PublishUpdate` of its files to all peers (`:1251-1340`), `HistoryGc{purge:true}` (`command.rs:111-118`), `ProjectCreateInvite` (`:45-54`), `PeerAddByUrl` (daemon performs HTTP GET to a caller-chosen URL, `:68-71`), `PeerSendStream` raw bytes to any peer (`:209-213`), `HooksRun` (`:145-151`). Game/AI pipelines routinely run untrusted code (asset importers, pip/npm packages, engine plugins, Tauri webview JS) under the same account, so a compromised package can exfiltrate or poison the shared project silently. There is no record of which client issued a command beyond `tracing` lines. Redesign: per-client authorization tokens (daemon-issued, stored 0600, presented in a `Hello` frame), command classes (read / mutate / destructive) with destructive ones requiring an interactive confirmation or a short-lived elevated token, structured audit log of IPC mutations, and origin labelling (`gui`, `cli`, `plugin`).

**S-4 (medium, design gap) — Tauri runs with `csp: null`.**
`tauri.conf.json:276-278` disables the Content-Security-Policy entirely. The React code has no raw-HTML sinks today, but any future XSS (e.g., a peer `display_name` rendered via a third-party component) immediately gets `invoke()` to all 15 commands and therefore S-3. Redesign: set a strict CSP (`default-src 'self'; connect-src ipc: http://ipc.localhost`), keep `withGlobalTauri` off, and add the Tauri `isolation` pattern for the mutating commands.

**S-5 (low, design gap) — Over-broad Tauri capabilities.**
`opener:default` is granted (`capabilities/default.json:296`) and the plugin initialised (`lib.rs:208`) although no frontend code imports `@tauri-apps/plugin-opener` (grep = 0) — this hands a future XSS the ability to open `http(s)://`/`mailto:` URLs in the OS browser. `dialog:default` grants open/save/message/ask/confirm but only `open` is used (`PublishModal.tsx:23`). Redesign: remove opener, scope dialog to `dialog:allow-open`.

**S-6 (medium, design gap) — Invite tokens never expire by default and are handled as plain strings in both GUIs.**
`expires_in_secs: None` means unlimited (`command.rs:47`); egui's default input is blank = never (`ui/overview.rs:161, 128`); Tauri passes `expiresInSecs ?? null` (`tauri.ts:104-108`). The token is a bearer credential that lets a holder join the project (`docs/two-node-operations.md:20` — no join ACL). egui shows it in a text field and copies to clipboard (`ui/overview.rs:164-175`); Tauri returns it to the webview (`lib.rs:120`). No revoke UI/command exists in `IpcCommand`. Redesign: default TTL (e.g., 1 h), single-use or bound-to-peer tokens, `InviteRevoke`/`InviteList` commands, and a GUI that masks tokens.

**S-7 (medium, design gap) — Tauri Settings writes an unauthenticated-servlet address with no warning, and non-atomic, world-readable config writes.**
`peer_info_listen_addr` placeholder `127.0.0.1:7780` with help text "公開ノード用" (`Settings.tsx:300-316`) while the `/peer-info` servlet is unauthenticated and leaks version/endpoint (`review/2026-05-13/REVIEW_VULNERABILITY.md:5-7`); docs recommend binding `0.0.0.0` (`two-node-operations.md:55`). `config::save` uses `std::fs::write` (`config.rs:160`) — not atomic (crash = truncated TOML, daemon then "using defaults" `daemon.rs:565-568`) and inherits umask (the file may carry `tunnel.api_token_ref`, bootstrap URLs). Redesign: temp-file + rename, 0600 on Unix, and a UI warning that this exposes an anonymous endpoint.

**S-8 (low) — Daemon responses are stringified into webview error payloads.**
`BridgeError::unexpected` embeds `format!("{:?}", resp)` (`ipc.rs:260-262`), so an unexpected `InviteToken{token,..}` or `ProjectDetail` lands in JS error strings/console. Redesign: map to a fixed code and log the body only at `debug` on the Rust side.

**S-9 (medium, design gap) — No IPC protocol versioning / hello.**
Positional MessagePack (`transport.rs:92`), no version field, no `Hello`; only two `#[serde(default)]` fields (`command.rs:52`, `response.rs:237`). A mismatched GUI (e.g., Tauri bundle from an older release) against a newer daemon fails with opaque `Deserialize` errors or, worse, decodes a reordered variant as the wrong command. Redesign: a `Hello{protocol_version, client_kind}` first frame with negotiated feature flags, and `rmp_serde::to_vec_named` or a schema-evolving encoding.

**S-10 (low) — Over-privileged event delivery.**
`Subscribe` is unvalidated (`command.rs:514`) and `EventFilter::All` delivers `PeerStreamReceived{payload}` (raw bytes from any peer, `event.rs:71-75`) to any same-user subscriber; combined with S-3 this is a local sniffing channel for upper-layer protocols (Susurrus). Redesign: capability-scoped subscriptions.

**S-11 (medium, process gap) — Supply-chain posture is permissive.** See the dedicated section below: tag-pinned actions, floating `@stable` toolchain and `@main` reusable workflow, non-blocking `cargo audit`, no `cargo deny`/`vet`, no Dependabot/Renovate, no MSRV, no frontend `npm ci`/`npm audit`/`tsc` in CI, duplicated vulnerable-prone crates (`rustls 0.21.12` next to `0.23.44`).

**S-12 (medium, process gap) — Unsigned binaries and docs that normalize Gatekeeper bypass.**
`setup-macos.md:54-78` instructs `xattr -d com.apple.quarantine` on downloaded artifacts; there is no signing/notarization, no checksum file, and no Tauri updater/signing config (`tauri.conf.json:280-287`); `cloudflared` is fetched from `releases/latest/download` with no digest check (`platforms.md:56`, `SETUP.md:68-70`, `OPERATIONAL-TEST.md:55-57`). Redesign: signed releases + SHA256SUMS, pinned cloudflared version + checksum, Tauri updater with a signing key held in Infisical.

**S-13 (medium, design gap) — No Cernere / Infisical integration anywhere in the client tree.**
Org rules require auth centralised in Cernere and secrets via Infisical; the only "Cernere" mention in Synergos is a CSS design-system note (`synergos-tauri/README.md:6,16`); `DESIGN.md:1013-1017` still describes "Ars Auth" + Cloudflare Access + per-peer Permission, none of which exist (`two-node-operations.md:20`, `operations-readiness.md:68`). Clients have no login, no identity display beyond `peer_id`, and the Tauri Settings UI edits `api_token_ref`-adjacent config by hand.

### Transfer / performance

**T-1 (medium) — No RPC timeout; UI-thread blocking in egui.**
`IpcClient::send` awaits `resp_rx` with no timeout (`client.rs:491-512`). egui drives every RPC with `runtime.block_on` from `eframe::App::update` (`connection.rs:69, 115`), so a slow handler (`PublishUpdate` streaming CRC over a multi-GB asset `ipc_server.rs:1323-1326`, a `pre-publish` hook with the 60 s default timeout `spec/tasks/2026-08-24-publish-hooks.md:33`) freezes the whole GUI; Tauri commands hang until the daemon answers. Redesign: `tokio::time::timeout` per command class, background tasks + channel for egui, cancellation on window close.

**T-2 (medium) — Per-frame IPC storm on the Transfers tab.**
`ui/transfers.rs:46-47` calls `connection.refresh_transfers()` inside `show()`, i.e., one new Unix connection + `TransferList` RPC + server task spawn per repaint (egui repaints on every input event, otherwise every 2 s via `app.rs:389`). With a mouse moving over the panel this is tens of connections per second against a daemon that is also moving data. Redesign: timer-driven refresh (like `refresh_if_needed`) or a single long-lived `Subscribe` connection.

**T-3 (low) — Short-lived connection per RPC in Tauri.**
Each command = connect + spawn reader task + oneshot + teardown (`ipc.rs:265-268`, `client.rs:434-450`); polling intervals of 3-5 s across 5 queries produce roughly 1.5 connections/s per open window. Acceptable now, but the daemon's `handle_client` spawns 7 broadcast subscriptions only for subscribers, so cost is mainly syscalls. Redesign: a shared connection with multiplexed request ids once a `Hello` exists.

**T-4 (medium) — Event path can silently drop under load.**
Server broadcast capacity is 256 per event type (`synergos-core/src/event_bus.rs:20, 57`); a `Lagged` error is swallowed with `continue` (`ipc_server.rs:607`), so a subscriber never learns it missed `TransferProgress`/`TransferCompleted`. Client side the event queue is unbounded (`client.rs:436`); a subscriber that stops calling `recv_event()` grows memory without limit. Redesign: coalesce progress events per transfer, bounded client queue with back-pressure, explicit `EventsDropped{n}` notification.

**T-5 (high, design gap) — The transfer-tuning features the GUI advertises do not exist.**
egui Settings shows "Chunk Size (Large) 1 MiB / Medium 256 KiB / Small 64 KiB" and "Bandwidth Ratio Large 60% / Medium 30% / Small 10%" as facts (`ui/settings.rs:129-150`); `NetworkStatusInfo.total_bandwidth_bps` is rendered as "Bandwidth" (`ui/overview.rs:37-39`, `Dashboard.tsx:239`). Implementation: `StreamAllocationConfig`, `SpeedTestConfig`, `PeerSelectionConfig`, `MonitorConfig` are parsed and validated (`synergos-net/src/config.rs:18-21, 634-687, 765`) but never read by any module (grep of `.speed_test`/`.peer_selection`/`.monitor`/`.stream_allocation` outside `config.rs` = 0); `ConnectionCalibrator::calibrate` is never called (only the definition `synergos-net/src/quic/mod.rs:108` and re-export `lib.rs:37`); `total_bandwidth_bps` is a sum of per-peer `bandwidth_bps` from presence with no measurement behind it (`ipc_server.rs:1448`). Users will tune numbers that do nothing. Redesign: either implement speed test + allocation + per-peer rate limit (DESIGN 15.2-15.4, `DESIGN.md:1032`) or remove the knobs and label the metrics "reported by peer".

**T-6 (medium) — `ConfigUpdate` is a no-op that the egui UI reports as applied.**
Daemon logs and returns `Ok` without changing anything (`ipc_server.rs:2027-2032`); egui shows "Applied (restart daemon for full effect)" (`ui/settings.rs:107-112`), and nothing is persisted, so a restart does not apply it either. Redesign: either persist to `synergos-net.toml` and hot-apply the safe subset, or return `Error{code: 501}` and hide the form.

**T-7 (medium) — Tauri Settings edits a config file the daemon never reads.**
`config.rs:196-198` writes `<identity_dir>/synergos-net.toml` (Linux `~/.config/synergos/`, macOS `~/Library/Application Support/synergos/`, Windows `%APPDATA%/synergos/`), but the daemon only loads an explicit `start --config <path>` and otherwise uses defaults (`daemon.rs:557-571`; `two-node-operations.md:34`; `setup-macos.md:139`). macOS docs recommend `~/.config/synergos/synergos-net.toml` (`two-node-operations.md:40`, `setup-macos.md:90-91`), a different directory from the one Tauri writes. Result: "保存しました。daemon を再起動すると反映されます" (`Settings.tsx:319`) is false unless the operator happened to start the daemon with that exact path. Redesign: one canonical config path resolver shared by daemon and GUIs (in `synergos-ipc` or `synergos-net`), daemon auto-discovery with precedence, and a `ConfigPath` IPC query so the GUI edits whatever the running daemon loaded.

**T-8 (low) — Config round-trip drops comments and reorders keys.**
`config::save` re-serialises through `toml::Value` (`config.rs:108-160`), destroying operator comments in the documented hand-written config (`two-node-operations.md:53-84`).

### AI / game asset production fit

**G-1 (high, design gap) — No real-time events in either GUI; polling only.**
Tauri polls at 3-5 s (`Dashboard.tsx`, `App.tsx`), egui at 2 s (`connection.rs:104`); `TransferProgress`/`ConflictDetected` are never consumed. Artists get stale progress bars and no conflict notification. `synergos-tauri/README.md:57` acknowledges. Redesign: a daemon-event bridge (`Subscribe` -> Tauri `emit`) with per-transfer coalescing.

**G-2 (high, design gap) — Tauri (the declared primary GUI) lacks every versioning/collaboration surface.**
No conflicts, history, checkout/restore, tags, hooks, invite-with-URL, join-by-token, transfer cancel, peer disconnect UI (`lib.rs:37-194` exposes none of `ProjectJoin`, `ProjectCheckout`, `ProjectRestore`, `History*`, `Tag*`, `Hooks*`, `ConflictList/Resolve`, `TransferCancel`, `TransferRequest`); `synergos-tauri/README.md:54-58` lists only two gaps. `project_invite` hardcodes `peer_info_url: None` (`lib.rs:117`), and egui likewise (`connection.rs:172`), so GUI-issued invites are the legacy same-daemon tokens that fail across machines (`getting-started.md:125`, `two-node-operations.md:246`, `operations-readiness.md:33, 57`). Redesign: GUI command surface parity with the CLI, driven from a shared command catalog.

**G-3 (medium, design gap) — Conflict resolution is a three-button stub.**
egui `Keep Local / Accept Remote / Manual` (`ui/conflicts.rs:62-78`) call `ConflictResolve` which only flips `ConflictManager` state (`ipc_server.rs:2001-2026`); detection is limited to same-version size/CRC mismatch, with no ancestor-based fork detection, no side-by-side quarantine, no diff/preview for binary assets (`operations-readiness.md:28`). "Manual Merge" has no merge workflow behind it. Redesign: conflict objects with both blobs retained, preview hooks per asset type, and resolution recorded in the manifest/history.

**G-4 (medium, design gap) — Publish is file-list driven with free-text paths.**
Tauri `AddProjectModal` takes the root as free text (`AddProjectModal.tsx:80-88`); `PublishModal` lets the OS picker choose files outside the root and only the daemon rejects them afterwards (`PublishModal.tsx:19-36`, `ipc_server.rs:1280-1285`); there is no "publish all changed", status view, or ignore rules (`operations-readiness.md:51-54`). Redesign: directory picker, worktree status diff (manifest vs disk), batch publish with per-file preflight.

**G-5 (medium, design gap) — The Ars plugin is a type-conversion stub.**
No `ars-core` dependency (`ars-plugin-synergos/Cargo.toml` vs `DESIGN.md:1310, 1396`), no `ProjectModule`, no IPC connect/`Subscribe` loop (contrast `DESIGN.md:745-800`), `max_connections: 0` hardcoded (`bridge.rs:101`), three events dropped (`bridge.rs:118`) so Ars would never see `TransferFailed`; zero tests. `review/2026-05-13/REVIEW_MISSING_FEATURES.md:44` scoring it 60 % overstates it.

**G-6 (low) — Tauri shows wrong daemon paths.**
`config.rs:38-47, 164-198` reports `identity` (no extension) under `~/.config/synergos` (Linux) / `.../synergos` (lower-case, macOS/Windows), while the daemon uses `identity.key` under `$XDG_STATE_HOME/synergos` or `~/.local/state/synergos` (Linux), `~/Library/Application Support/Synergos` (macOS), `%APPDATA%/Synergos` (Windows) (`synergos-net/src/identity.rs:186-217`), and `projects.json` beside `identity.key` (`daemon.rs:153-156`). On case-sensitive file systems the macOS/Windows variants are different directories. Users told to back up the identity (`two-node-operations.md:42-43`) via the GUI will back up nothing.

### Robustness / quality

**Q-1 (low) — Error details swallowed in egui.** `send_command` maps any error to `None` (`connection.rs:117`); daemon `IpcResponse::Error{message}` (e.g., "pre-publish hook rejected", "unknown project") is reduced to "failed" toasts (`ui/overview.rs:91`, `ui/transfers.rs:39`).

**Q-2 (low) — Mutex poisoning panics.** `cache.lock().unwrap()` throughout (`connection.rs:80, 103, 129, 138`; `app.rs:345`; `ui/*.rs`); a panic in one frame takes the app down on the next.

**Q-3 (low, not exploitable today) — Byte-index slicing of ids.** `&id[..12]` in `ui/peers.rs:97` and `ui/conflicts.rs:89` panics on a multi-byte boundary; inputs are hex `PeerId`s today (`synergos-core/src/conflict/mod.rs:45, 48`) so safe, but fragile if display names are ever substituted.

**Q-4 (low) — Tauri `daemon_ping` folds every send error into "not running"** (`lib.rs:49-52`), hiding protocol errors from the status LED (`App.tsx:13-18`).

**Q-5 (medium, process) — Stale and self-contradicting operator docs** (details in §8): `OPERATIONAL-TEST.md` expects delta transfer and `publish` without file args; `SETUP.md` shows `inv-xxxx` tokens; `platforms.md:15` claims a Named-Pipe ACL; `DESIGN.md` section numbering is broken (`## 9` contains `### 6.1`, `## 10` has `### 7.1`, `## 16` has `### 13.1`) and its `IpcCommand` listing (`DESIGN.md:161-187`) omits ~20 variants.

---

## 5. Performance characteristics

- **IPC frame**: 4 B header + MessagePack; max 1 MiB; staged 64 KiB reads (`transport.rs:39-42, 131-145`). Each `send` = 1 write (`write_all` header + payload + `flush`, `transport.rs:100-102`) and 1 round trip; no pipelining in practice because every client does one command per connection.
- **Connection model**: Tauri and egui open a new socket per RPC (`ipc.rs:265-268`, `connection.rs:114-121`); every connect spawns a client reader task (`client.rs:444`) and a server `handle_client` task (`ipc_server.rs:121-126`). Daemon-side per-client cost: a `Mutex<WriteHalf>` and, only if subscribed, 7 broadcast receivers (`ipc_server.rs:581-589`).
- **egui runtime**: dedicated tokio runtime with `worker_threads(1)` (`connection.rs:43-47`); all I/O is `block_on` on the UI thread (`connection.rs:69, 115`) -> UI stalls equal RPC latency; refresh cadence 2 s (`connection.rs:104`) with three serial RPCs; repaint request every 2 s (`app.rs:389`); Transfers tab issues an RPC per frame (`ui/transfers.rs:47`); Settings tab holds the cache mutex for the whole panel render (`ui/settings.rs:9`).
- **Tauri polling**: React Query intervals 3 s (ping, transfers), 5 s (status, projects, peers) with `staleTime 5 s`, `retry false`, `refetchOnWindowFocus false` (`main.tsx:8-17`, `Dashboard.tsx`, `App.tsx`). Five concurrent pollers => ~1.5 connections/s steady state per window.
- **Events**: server broadcast capacity 256 per event type (`event_bus.rs:20`), lag dropped silently (`ipc_server.rs:607`); client event queue unbounded (`client.rs:436`).
- **Config I/O**: full read -> parse -> serialize -> write on every save (`config.rs:100-162`); non-atomic; load on every `config_get` (`config.rs:49-58`).
- **Hashing used on the client paths**: none; CRC32 (streaming) and blake3 happen daemon-side (`ipc_server.rs:1323-1326`, `operations-readiness.md:20, 22`).
- **O(n) hot spots**: `NetworkStatus` iterates all presence nodes and all transfers per call (`ipc_server.rs:1442-1474`) and is polled by both GUIs every 2-5 s; `transfer_list`/`project_list` clone whole vectors per poll.
- **Build/CI**: `.cargo/config.toml` forces `jobs = 1` for every build (`:1-3`), which slows all CI jobs; Tauri Linux builds need webkit2gtk etc. (`ci.yml:38-42`).
- **Frontend**: Vite ES2022 target, esbuild minify, no sourcemaps (`vite.config.ts:17-21`).

---

## 6. Config knobs & defaults (clients)

| Knob | Where | Default | Secure-by-default? |
|---|---|---|---|
| `bootstrap_urls` | Tauri `ConfigFile` -> `synergos-net.toml` top level (`config.rs:60-68, 118-126`) | `[]` | yes (no auto-connect) |
| `force_relay_only` | same (`config.rs:70-73, 127-130`) | `false` | n/a |
| `auto_promote` | same (`config.rs:75-78, 131-134`); when file missing `true` (`config.rs:51-56`) | `true` -> runs IPv6/UPnP/cloudflared probes at start (`Settings.tsx:279-281`) | debatable: UPnP mapping attempts by default |
| `quic.listen_addr` | `[quic]` table (`config.rs:80-84, 145-157`) | none -> daemon default `[::]:0` (`Settings.tsx:297`) | yes (ephemeral port) |
| `peer_info_listen_addr` | top level (`config.rs:86-89, 135-142`) | none -> servlet off | yes by default; UI placeholder `127.0.0.1:7780`, docs push `0.0.0.0:7780` (`two-node-operations.md:55`) with no auth on the servlet |
| Tauri CSP | `tauri.conf.json:277` | `null` | **no** |
| Tauri capabilities | `capabilities/default.json:294-298` | `core:default`, `opener:default`, `dialog:default` | over-broad (S-5) |
| Tauri logging | `RUST_LOG`, default `synergos_tauri=info` (`lib.rs:200-205`) | info to stderr | ok |
| egui logging | `RUST_LOG`, default `synergos_gui=info` (`main.rs:429-434`) | info | ok |
| egui window | 1024x768, min 800x600 (`main.rs:436-442`) | — | — |
| egui "Network Tuning" (`mesh_n`, `max_concurrent_streams`, `tunnel.hostname`) | `ui/settings.rs:74-124` -> `ConfigUpdate` | none | misleading: no effect (T-6) |
| Invite expiry | egui text field blank = never (`ui/overview.rs:161`); Tauri `undefined` -> `null` (`tauri.ts:104-108`) | never | **no** (S-6) |
| IPC socket path | env `XDG_RUNTIME_DIR` / `HOME` (`transport.rs:55-78`) | `/tmp/synergos/synergos.sock` fallback on Linux | **no** (S-1) |
| IPC frame limit | `MAX_MESSAGE_SIZE` const (`transport.rs:39`) | 1 MiB | yes |
| Vite dev server | `127.0.0.1:5173`, `strictPort` (`vite.config.ts:8-15`) | loopback | yes |
| CI audit | `continue-on-error: true` (`ci.yml:163`) | non-blocking | **no** |
| Toolchain | `dtolnay/rust-toolchain@stable` (`ci.yml:23`), no `rust-toolchain.toml`, `synergos-custody` alone declares `rust-version = "1.89"` (`synergos-custody/Cargo.toml:5`) | floating | **no** |

---

## 7. Test coverage observed

Present:
- `synergos-ipc/src/transport.rs:151-380`: 8 tests — round trip, EOF -> `ConnectionClosed`, oversize write/read rejection, partial header, multiple frames, Linux socket path with/without `XDG_RUNTIME_DIR` (env mutation under `serial_test`), full serde of command/response/event/filter.
- `synergos-ipc/src/command.rs:553-696`: 12 `validate()` tests — empty/whitespace/long ids, empty publish list, rel-path traversal/absolute/`.synergos`/backslash, `PeerSendStream` size, tag charset, tag pins exclusivity and versions.
- `synergos-core/tests/ipc_handlers.rs` (17 tests): ping, status, project list/get/open, history node + tags + gc, transfer list, unsubscribe, `network_status_returns_hardcoded_shape`, pre-publish hook abort/batch/opt-in/timeout, post-receive env. `ipc_integration.rs`: ping over duplex, ordering, malformed frame closes gracefully, client disconnect cleanup. `ipc_subscribe.rs`: subscribe-all delivery, unsubscribe stops push, category filter. `ipc_client_events.rs`: response/event multiplex on one stream, `is_daemon_running` false.
- `synergos-gui/tests/ui_snapshot.rs`: 9 `egui_kittest` headless renders — overview (empty/populated), transfers (empty/rows), peers (empty/rows), conflicts (empty/rows + buttons), settings headers. These seed `ConnectionCache` directly; no IPC is exercised (`ui_snapshot.rs:16-23`).
- `synergos-tauri/src-tauri/src/config.rs:200-251`: 1 test that rebuilds a `toml::Table` by hand and checks substrings — it does **not** call `load()`/`save()`/`paths()`.

Absent:
- No test of the UID/SID rejection path on either OS (grep of `synergos-core/tests` for `verify_peer_uid`/`SO_PEERCRED` = 0; the functions at `ipc_server.rs:231-307, 342-375` are untested).
- No test for socket hijack / pre-existing `/tmp/synergos` ownership, nor for Windows pipe squatting.
- No tests at all in `ars-plugin-synergos` (no `tests/`, no `#[test]`), so `bridge_event` mapping and the dropped variants are unverified.
- No tests for `synergos-tauri` commands, `BridgeError` mapping, `config::load/save` on disk, or path resolution; no frontend tests (no `*.test.*` files) and no CI step that even type-checks the React code.
- egui `CoreConnection` logic (`try_connect`, `send_command`, per-frame refresh) untested; no timeout tests because there is no timeout.
- No test that `Subscribe` + `Lagged` loses events, or for the unbounded client event queue.
- Coverage job exists but is `continue-on-error` and uploads an artifact only (`ci.yml:120-155`).

---

## 8. Documented-vs-implemented deltas

### 8.1 DESIGN.md promises vs current status

| Promise (DESIGN.md) | Status | Evidence |
|---|---|---|
| Bandwidth control: size-class stream allocation 60/30/10, chunk 1 MiB/256 KiB/64 KiB (15.2, `:1468-1489`), per-peer rate limit (`:1032`) | **Not implemented** (config only) | `StreamAllocationConfig` parsed/validated (`synergos-net/src/config.rs:634-657, 765`) but no consumer; no `RateLimit`/`throttle` symbol anywhere in `synergos-net`/`synergos-core` (grep); `CalibratedParams.initial_bandwidth_limit` never used (`quic/mod.rs:100, 127`); egui still displays the ratios as fact (`ui/settings.rs:146-148`) |
| Speed test at connect, retest interval, probe count (15.3, `:1571-1599`; config `:2095-2102`) | **Not implemented** | `SpeedTestResult`/`ConnectionCalibrator::calibrate` defined (`quic/mod.rs:77-130`), zero call sites; `SpeedTestConfig` unused outside `config.rs` |
| Bandwidth-aware peer selection, `PeerBandwidthProfile`, composite score (15.4, `:1666-1700`; config `:2104-2111`) | **Not implemented** | `PeerSelectionConfig` unused; no `PeerBandwidthProfile`/`composite_score` symbols (grep) |
| QUIC application-level multicast `BroadcastDispatcher` (15.6, `:1811-1906`) | **Not implemented** | grep `BroadcastDispatcher|broadcast_file|multicast` in `synergos-net`/`synergos-core` = only IP-address `is_multicast()` checks (`mesh/mod.rs:187`, `promotion.rs:145`); `operations-readiness.md:69` says gossip "全 QUIC 接続へ flood" |
| Network monitoring: `NetworkMonitor`, snapshot history 3600, bandwidth graph, packet loss (15.7, `:1908-2068`; config `:2113-2120`) | **Not implemented** | `MonitorConfig` unused; `NetworkStatus` computed ad hoc from presence + exchange (`ipc_server.rs:1441-1475`); no history, no graph in either GUI (`ui/overview.rs:10-58`, `Dashboard.tsx:256-283`); test named `network_status_returns_hardcoded_shape` (`tests/ipc_handlers.rs:484`) |
| Chain (17, `:2419-2611`): per-file `FileChain` of `ChainBlock`s with fork detection, text diff vs binary CID | **Partially present, not wired** | `FileChain`/`ChainBlock` exist and `CatalogManager` holds `chains` (`synergos-net/src/catalog/manager.rs:5, 14, 63-86`), but `record_update`/`get_chain` have no callers outside the manager (grep); versioning actually uses `.synergos/manifest.json` + `state.json` (`operations-readiness.md:14, 67`); text diff/delta explicitly dropped ("差分管理はしない", `operations-readiness.md:25, 59`) |
| Catalog system (18, `:2743-2941`): chunks, update detection, sync (`[catalog]` `:2143-2150`) | **Skeleton** | `CatalogManager` (`catalog/manager.rs`, 300 lines) and `CatalogSyncService` (`synergos-core/src/catalog_sync.rs`, 245 lines) exist, `[catalog]` is the only config section consumed (`synergos-net/src/lib.rs:87-88`, `daemon.rs:305-306`); but "RootCatalog を Bitswap で取ってくるが実ファイル同期には使われていない" and not persisted (`operations-readiness.md:23`; `review/.../REVIEW_MISSING_FEATURES.md:13-15`) |
| Conflict UI "Alice vs Bob v4 fork [Resolve]" (6.3, `:843-845`; 19, `:2942-3160`) | **Stub in egui, absent in Tauri** | egui three buttons -> `ConflictResolve` -> `ConflictManager::resolve_conflict` state flip (`ui/conflicts.rs:62-78`, `ipc_server.rs:2001-2026`, `conflict/mod.rs:231`); detection only same-version size/CRC mismatch; fork detection/quarantine/merge UI unimplemented (`operations-readiness.md:28`); Tauri has none (`synergos-tauri/README.md:56`) |
| GUI push events "IPC クライアント接続・イベント受信" (Phase 5-1, `:1362`) | daemon side done, **no GUI consumes it** | `Subscribe` handled (`ipc_server.rs:520-543`); neither GUI calls it; Tauri README `:57` |
| Ars plugin `ProjectModule` + bridge (5.2, `:711-801`; Phase 6, `:1369-1376`; opt-out guarantee item 4 `:1396`) | **Stub** | No `ars-core` dep (`ars-plugin-synergos/Cargo.toml`), no `ProjectModule`, no IPC loop; only `bridge_event` conversion (`bridge.rs:44-120`) |
| Security stack: Ars Auth, project share settings, Cloudflare Access JWT, mTLS, per-peer Permission (9, `:1009-1025`) | **Only mTLS/SPKI pinning exists** | "同一 daemon 内 ACL 無し" (`two-node-operations.md:20`); ACL is P2 #6 (`operations-readiness.md:68`); history nodes must be kept inside a trusted mesh (`two-node-operations.md:226-227`) |
| File-transfer security: size limit, per-peer rate limit (9.2, `:1029-1032`) | checksum + path validation yes; **size limit & rate limit no** | `operations-readiness.md:21-22` (path escape, blake3); no size/rate limiter symbols (grep) |
| Cloudflare Tunnel for QUIC, mobile IPv6/TURN (1, `:12, 30-31`) | **Abandoned / unimplemented** | `operations-readiness.md:29, 32`; `OPERATIONAL-TEST.md:326-327`; `review/.../REVIEW_MISSING_FEATURES.md:5-7` |
| E2E tests in `tests/` for plugin integration (Phase 6-3, `:1375`) | **No** | no `ars-plugin-synergos/tests` |
| `IpcCommand` set (2.4, `:161-187`) | **Subset** | actual enum has 40+ variants (`command.rs:12-214`); DESIGN omits History*, Tag*, Hooks*, ProjectJoin, PeerAddByUrl, ConfigUpdate, PeerSendStream, ProjectUpdate/Get, Checkout/Restore |
| Linux socket at `/tmp/synergos/synergos.sock` (2.4 diagram, `:224-226`) | partly | actual prefers `$XDG_RUNTIME_DIR` (`transport.rs:58`); `/tmp` is only the insecure fallback |
| Config paths `.ars/synergos.toml` (10, `:1039`; 15.8 `:2073`) | **Different** | real file is `synergos-net.toml` under per-OS config dirs (`two-node-operations.md:37-40`); Tauri writes yet another location (T-7) |

### 8.2 Other doc deltas
- `platforms.md:15` "Named Pipe ACL で呼び出しユーザ限定": implementation explicitly says no ACL is set and uses a post-connect SID comparison (`ipc_server.rs:313-315`).
- `platforms.md:19` identity at `$XDG_STATE_HOME/synergos/identity.key` matches daemon (`identity.rs:186-198`) but contradicts `two-node-operations.md:42-43` (`~/.config/synergos/identity.key`) and Tauri (`config.rs:185-198`).
- `OPERATIONAL-TEST.md:184, 237, 243` expects "帯域制御 (Large 60% / Med 30% / Small 10%)", chunk-level delta pull, and received bytes smaller than the file; `:196, 229` run `project publish myproj` with no file list; `:267-269` relies on CatalogSync catch-up. All contradicted by `operations-readiness.md:23, 25` and `two-node-operations.md:16` (publish is per-file, whole-file). Operators following T3/T4 will log false failures.
- `SETUP.md:317-320` shows `inv-xxxx` invite tokens; current tokens are `syn1.<payload>.<sig>` (`two-node-operations.md:134-138`, `setup-macos.md:207-213`). `SETUP.md:248-266` lists `[stream_allocation]`, `[speed_test]`, `[peer_selection]`, `[monitor]` as if functional.
- `OPERATIONAL-TEST.md:32` says Rust "最低 stable 1.89+"; `platforms.md:39` says no minimum is fixed; no `rust-toolchain.toml` exists.
- `OPERATIONAL-TEST.md:390` troubleshooting says "token は 1 回限り使い捨ての可能性" — no single-use logic exists in `IpcCommand`/docs; tokens are reusable until expiry.
- `review/2026-05-13/REVIEW_QUALITY.md:13` credits CI "audit" as a gate; it is `continue-on-error` (`ci.yml:163`). The review predates `synergos-control`, `synergos-custody`, history rotation and hooks (git log: `51ba2c6`, `aaca8dd`, `c5d4ca4`), so the vulnerability grade B does not cover the current surface.
- `spec/tasks/2026-08-24-publish-hooks.md:92-93` and `2026-08-24-admin-ui-dioxus.md:111-115`: tests were not executed in the implementing sessions; rustfmt drift across `synergos-core`/`synergos-ipc` acknowledged (`2026-08-24-rustfmt-drift-core-ipc.md`), meaning the fmt CI gate (`ci.yml:17-27`) was red at that point.
- `docs/getting-started.md:17` says the daemon "CatalogSyncService を持つ" and `:109-116` describes an auto-sync flow via `CatalogSync` events, while `operations-readiness.md:23` says catalog sync is not used for real data.

### 8.3 Supply-chain posture (summary)
- **CI permissions**: `permissions: contents: read` at workflow level (`ci.yml:9-10`); `harness.yml` sets none (inherits defaults) and calls `LUDIARS/AIFormat/.github/workflows/harness-checks.yml@main` (`harness.yml:17`) — a mutable branch ref of an external public repo controlling the gate.
- **Action pinning**: tags only — `actions/checkout@v4` (`ci.yml:21, 33, 64, 97, 127, 165`), `dtolnay/rust-toolchain@stable` (floating branch, `:23, 35, 66, 99, 129, 167`), `actions/cache@v4`, `actions/upload-artifact@v4`. No SHA pins.
- **Audit/deny**: `cargo audit` on both lockfiles but `continue-on-error: true` (`ci.yml:157-173`); tool installed unpinned (`cargo install --locked cargo-audit`). No `deny.toml`, no `cargo vet`, no `.github/dependabot.yml`/`renovate.json` (directory listing).
- **Lockfiles**: `Cargo.lock` (865 packages, 0 git deps), `synergos-admin-ui/Cargo.lock`, `frontend/package-lock.json` v3, root `package-lock.json` v3. Caret ranges in `frontend/package.json:16-31`.
- **Toolchain**: no MSRV/`rust-toolchain.toml`; `synergos-custody/Cargo.toml:5` alone pins `rust-version = "1.89"`.
- **Frontend in CI**: none — no `npm ci`, `tsc`, `vite build`, or `npm audit` step; the React app is never built on CI. `cargo build --workspace` compiles the Tauri crate whose `generate_context!()` embeds `../frontend/dist` (`tauri.conf.json:262`, `lib.rs:227`); whether codegen tolerates a missing dist is not verifiable here (registry absent) — see open questions.
- **Duplicate/old crates in lock**: `rustls 0.21.12` and `0.23.44`; `toml 0.8.2/0.9.12/1.1.5`; `reqwest 0.12.28/0.13.5`; `tokio-tungstenite 0.24.0` (older line); `wry 0.55.1`, `tao 0.35.3`, `tauri 2.11.5`, `tauri-plugin-dialog 2.7.3`, `tauri-plugin-opener 2.5.5`, `egui/eframe 0.31.1`, `quinn 0.11.11`, `hickory-resolver 0.24.4`, `object_store 0.11.2`, `jsonwebtoken 9.3.1`.
- **unsafe inventory**: client crates 0; `synergos-ipc` 5 (tests only); `synergos-core` 25 (libc/Win32 credential checks, `ipc_server.rs:231-420`); `synergos-net` 0; `synergos-control` 1; `synergos-custody` 1; `synergos-relay` 0.
- **Signing/distribution**: no release workflow, no code signing, no checksums; docs instruct quarantine stripping (`setup-macos.md:62-68`).

### 8.4 Client attack surface (summary)
- **Tauri**: 15 commands reachable from webview JS (`lib.rs:210-226`); no CSP (`tauri.conf.json:277`); capabilities `core:default` + unused `opener:default` + over-broad `dialog:default` (`capabilities/default.json:294-298`); webview renders daemon- and peer-originated strings (`display_name`, `file_name`, `root_path`, `synergos_version`) via JSX (`Dashboard.tsx:63-65, 146, 228-240`); config file read/write with user privileges (`config.rs`); every command = unauthenticated same-UID IPC (S-3).
- **egui**: free-text project/peer/file ids and root paths (`ui/overview.rs:73-78`, `ui/peers.rs:18-24`, `ui/transfers.rs:18-28`); clipboard export of invite tokens (`ui/overview.rs:172-175`); UI-thread blocking I/O (T-1).
- **IPC trust**: server authenticates the client by UID/SID only (`ipc_server.rs:114-119, 321-325`); client never authenticates the server (`client.rs:452-488`); socket path depends on env (`transport.rs:55-78`); no per-command authorization, no audit, no version negotiation; 1 MiB frames; `Subscribe{All}` streams raw peer payloads (`event.rs:71-75`).
- **ars plugin**: none today (no runtime behaviour).

---

## 9. Open questions for designers

1. What is the intended trust boundary for IPC? If "any same-user process is fully trusted" is accepted, say so explicitly in DESIGN §9 and document the exfiltration implication for studios running untrusted tooling; otherwise S-3 needs a token/capability design that CLI, Tauri, egui, and Ars all implement.
2. Which GUI is canonical? `synergos-tauri/README.md:5` says Tauri is the new main line, yet only egui has conflicts/settings/transfer-request/cancel panels and the docs only ever launch `synergos-gui` (`getting-started.md:51-59`, `setup-macos.md:298-305`). Maintaining two pollers doubles the parity gap (G-2).
3. Should the daemon own a canonical config path (auto-discovery with `--config` override) so Tauri Settings edits the live config (T-7), or should config editing move behind an IPC command with persistence (T-6)?
4. Does `tauri::generate_context!()` tolerate a missing `frontend/dist` on CI? If not, CI is green only because `dist` existed on the builder, and the frontend must get its own CI job either way.
5. Is `/tmp/synergos` ever an acceptable socket location? If the systemd/`XDG_RUNTIME_DIR` footgun (`SETUP.md:302-305`) is common, the daemon should refuse `/tmp` or use `/run/user/<uid>` discovery and a per-uid 0700 directory.
6. The speed-test / allocation / peer-selection / monitor config sections: implement (DESIGN 15.2-15.7) or delete? Keeping them as validated-but-unused config and showing them in the GUI (`ui/settings.rs:129-150`, `SETUP.md:248-266`) misleads operators and the operational test plan.
7. Invite-token lifecycle: default TTL, single-use, revocation, and whether GUI-issued invites must carry `peer_info_url` automatically (currently `None` in both GUIs, `lib.rs:117`, `connection.rs:172`).
8. Event model for GUIs: one long-lived `Subscribe` connection with coalesced progress, or keep polling? Either way, the `Lagged` drop (`ipc_server.rs:607`) and unbounded client queue (`client.rs:436`) need a policy.
9. Where should the Ars plugin's `ars-core` dependency come from (`DESIGN.md:1310` says a git dependency on `LUDIARS/Ars`), and does the org allow git deps given the current zero-git-dep lockfile?
10. Who owns doc hygiene? `OPERATIONAL-TEST.md`, `SETUP.md`, `platforms.md`, `DESIGN.md` numbering and the 2026-05-13 review are out of sync with the code; a per-release "documented vs implemented" check (like this map) could be a harness gate.
11. Should `PeerSendStream`/`PeerStreamReceived` (the Susurrus escape hatch) be gated behind a capability so a generic local client cannot inject or sniff upper-layer protocol frames?
12. Release engineering: signing (macOS notarization, Windows Authenticode, Tauri updater keys via Infisical), SHA256SUMS, pinned cloudflared — none exist and the docs currently teach users to bypass Gatekeeper.
