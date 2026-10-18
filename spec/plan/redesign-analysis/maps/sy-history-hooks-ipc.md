# Subsystem map: Synergos history nodes, hook scripts, IPC server & protocol

Repo: `Synergos` (Rust workspace). Crates covered: `synergos-core` (`src/history/**`, `src/hooks/**`, `src/ipc_server.rs`, `src/cli_hooks.rs`, `src/cli_history.rs`), `synergos-ipc` (`src/*.rs`), docs (`docs/hooks.md`, `docs/versioning-design.md` §3, `spec/tasks/2026-08-24-publish-hooks.md`). Supporting code consulted for evidence: `synergos-net/src/config.rs` (HistoryConfig / HooksConfig / HookDef / RotationBackendConfig), `synergos-core/src/manifest.rs` (atomic replace, CRC, safe paths), `synergos-core/src/exchange/mod.rs` + `exchange/history_hooks.rs` (call sites), `synergos-core/src/daemon.rs` (wiring + rotation timer), `synergos-core/src/checkout.rs` (restore), `synergos-net/src/transfer/mod.rs` (`hash_file`).

All line numbers are 1-indexed and refer to the files as read on 2026-10-15. Statements marked **(inference)** are not directly evidenced by code and should be verified.

---

## 1. Responsibilities & data flow

### 1.1 History node (`synergos-core/src/history/`)

A node with `[history] enabled = true` keeps **every version** of every file it publishes or receives for covered projects as a whole-file, content-addressed object (blake3 of full file), plus a node-local JSON index. It answers `FileWant(version)` from peers with any version it holds, serves `project restore`/`checkout` locally, can GC by policy, can protect versions by tags, and can offload old versions to external storage (S3 / local path / Google Drive) and transparently pull them back.

**Enters:**
- `ArchiveRequest` (project_id, file_id, version, size, crc, publisher, source="published"|"received", path) from `Exchange::archive_to_history` (`exchange/mod.rs:464-469`), called from
  - the IPC `PublishUpdate` handler after manifest bump (`ipc_server.rs:1388-1406`), and
  - the network receive-complete path for every file received from any peer (`exchange/mod.rs:836-857`). Receive-path archive failures are only `warn!`ed (`exchange/mod.rs:849-856`); publish-path failures abort the IPC command (`ipc_server.rs:1402-1405`).
  - Both go through `history::wiring::build_hooks` → `HistoryStore::archive` (`history/wiring.rs:13-55`).
- Lookup requests `(project_id, file_id, version)` from `Exchange::serve_from_history` when a `FileWant` from **any peer** asks for a version that does not match the local shared record (`exchange/mod.rs:898-958`, `962-1005`), and from `checkout::restore_file` (`checkout.rs:134`).
- IPC commands `HistoryList/HistoryGc/HistoryRotate/HistoryOffloaded/HistoryFetch/TagAdd/TagLs/TagShow/TagRm` (`ipc_server.rs:1540-1884`).
- Daemon timer `spawn_rotation_timer` every `interval_hours` (`daemon.rs:653-708`).

**Leaves:** `StoredVersion{path,...}` for the exchange to stream the object file to the requester (`history/wiring.rs:56-76` → `exchange/mod.rs:985-1004`); report DTOs over IPC; objects uploaded to the rotation backend; `tracing` logs.

**Calls:** `synergos_net::config::HistoryConfig` (covers/validate), `crate::manifest::{replace_file_atomically, crc32_of_file, safe_rel_to_local}`, `synergos_net::transfer::hash_file` (blake3), `object_store` (S3), `reqwest` + `jsonwebtoken` (GDrive), `tokio::fs`.

Module roles:
| file | role |
|---|---|
| `history/mod.rs` | module tree + re-exports (`mod.rs:7-17`) |
| `history/store.rs` | `HistoryStore`: `store_dir`, `archive`, `lookup` (with auto-fetch), `list`, `tag_*`, `protected_versions`, `gc`, `rotate`, `offloaded`, `fetch_offloaded`; snapshot/validate helpers; temp sweeping |
| `history/index.rs` | `HistoryIndex` (index.json) load/rebuild/save, `ObjectMeta` sidecars, path helpers |
| `history/gc.rs` | pure `apply_retention` (victim selection) |
| `history/tags.rs` | tag files `<store>/tags/<name>.json`, validation, pin merge |
| `history/wiring.rs` | adapts `HistoryStore` to `exchange::history_hooks::{ArchiveHook, HistoryLookup}` |
| `history/rotation/mod.rs` | candidate selection, `rotate` (offload), `fetch` (restore), key format, backend factory |
| `history/rotation/backend.rs` | `RotationBackend` trait (put/get/exists/size/delete) |
| `history/rotation/{s3,local,gdrive}.rs` | backends |

### 1.2 Hook scripts (`synergos-core/src/hooks/`)

Two layers of `HookDef{event, command, match, timeout_sec}`: daemon-level `[hooks] hooks=[...]` in `synergos.toml` (always active) and project-level `<root>/.synergos/hooks.toml` (only if `hooks.allow_project_hooks = true`) (`hooks/mod.rs:1-6`, `runner.rs:119-143`).

**Enters:** `run_pre_publish(root, project_id, rel)` from `PublishUpdate` before any manifest/version mutation (`ipc_server.rs:1288-1304`); `spawn_post_hooks(PostPublish)` after `publish_updates` succeeds (`ipc_server.rs:1418-1431`); `spawn_post_hooks(PostReceive)` from `Exchange.post_receive_hook` after a received file is committed to the working tree (`exchange/mod.rs:858-865` → `hooks/wiring.rs:13-45`); `HooksList`/`HooksRun` over IPC (`ipc_server.rs:1887-1970`).

**Leaves:** child processes (`sh -c <command>` / `cmd /C <command>`) with cwd = project root and env vars `SYNERGOS_EVENT/PROJECT/FILE[/VERSION][/PEER]` (`runner.rs:319-347`); `HookOutcome` / `Err` for pre-publish; logs for post hooks.

### 1.3 IPC (`synergos-ipc` crate + `synergos-core/src/ipc_server.rs`)

`synergos-ipc` defines the wire types (`IpcCommand`, `IpcResponse`, `IpcEvent`, `ServerMessage`), the framing (`IpcTransport`), the socket path, and `IpcClient` (used by CLI, Tauri GUI, Dioxus admin, ars plugin). `ipc_server.rs` runs the listener (Unix domain socket or Windows named pipe), authenticates the peer (uid / SID), validates each command, dispatches it against `ServiceContext` (project manager, exchange, presence, conflict manager, history store, hook runner, QUIC, identity) and relays event-bus events to subscribers.

**Enters:** length-prefixed MessagePack frames from local clients. **Leaves:** `ServerMessage::Response` / `ServerMessage::Event` frames. **Calls:** essentially every daemon subsystem (`ipc_server.rs:23-64` imports; `dispatch_command` `828-2069`).

CLI wrappers `cli_hooks.rs` and `cli_history.rs` only translate argv → `IpcCommand` and print responses (`cli_hooks.rs:24-101`, `cli_history.rs:157-378`).

---

## 2. Key types / APIs / wire formats

### 2.1 IPC framing and transport
- Frame = `u32 LE length` + MessagePack (`rmp_serde`) payload (`transport.rs:7`, `87-104`, `110-148`).
- `MAX_MESSAGE_SIZE = 1 MiB` enforced on write and on read of the length header (`transport.rs:39`, `94-99`, `124-129`); read buffer grows in `READ_CHUNK = 64 KiB` steps (`transport.rs:42`, `133-145`).
- `ServerMessage { Response(IpcResponse), Event(IpcEvent) }` multiplexed on one stream (`transport.rs:49-52`). No request id: client matches responses FIFO (`client.rs:49`, `118-132`, `162-171`). No version negotiation / hello.
- Socket path: Linux `$XDG_RUNTIME_DIR/synergos/synergos.sock` with fallback `/tmp/synergos/synergos.sock` (`transport.rs:56-62`); macOS `$HOME/Library/Application Support/Synergos/synergos.sock` (`64-72`); Windows `\\.\pipe\synergos` (`74-77`).
- `IpcClient`: one background reader task, `pending: VecDeque<oneshot::Sender>`, unbounded event channel (`client.rs:30-50`, `54-70`, `145-184`).

### 2.2 Commands (`command.rs:12-214`)
Daemon: `Ping, Shutdown, Status`. Projects: `ProjectOpen{project_id, root_path, display_name}`, `ProjectClose`, `ProjectList`, `ProjectGet`, `ProjectUpdate`, `ProjectCreateInvite{expires_in_secs, peer_info_url}`, `ProjectJoin{invite_token, root_path}`. Peers: `PeerList, PeerConnect, PeerDisconnect, PeerAddByUrl{url}`. Transfer: `TransferRequest, TransferList, TransferCancel, PublishUpdate{file_paths: Vec<PathBuf>}`, `ProjectCheckout{manifest_path}`, `ProjectRestore{rel_path, version}`. History: `HistoryList{rel_path?}`, `HistoryGc{purge, keep_manifests}`, `HistoryRotate{dry_run, keep_manifests}`, `HistoryOffloaded{rel_path?}`, `HistoryFetch{rel_path, version}`. Hooks: `HooksList`, `HooksRun{event, rel_path}`. Tags: `TagAdd{name, manifest_path?, pins}`, `TagLs`, `TagShow`, `TagRm`. Conflicts: `ConflictList`, `ConflictResolve`. `ConfigUpdate` (accepted, no-op: `ipc_server.rs:2027-2032`). `NetworkStatus`, `Subscribe{events: Vec<EventFilter>}`, `Unsubscribe{subscription_id}`, `PeerSendStream{peer_id, magic:[u8;4], payload}`.

`IpcCommand::validate` (`command.rs:225-550`): `MAX_ID_LEN = 1024`, `MAX_PATHS_PER_PUBLISH = 1024`, `MAX_PATH_LEN = 4096` (`226-228`); `check_path` rejects empty, `..` components, NUL (`243-264`) but accepts absolute paths; `check_rel_path` rejects leading `/`, `\`, `.synergos` prefix, control chars, empty/`.`/`..` segments, `:` (`266-285`); tag name `[A-Za-z0-9._-]{1,64}` (`288-303`); version must be in `1..u64::MAX-1` (`406-409`, `461-464`, `502-504`); `HooksRun.event` enum (`475-482`); `PeerSendStream.payload ≤ 1 MiB` (`540-547`); `PeerAddByUrl.url` must start with `http(s)://` (`346-348`). `ConfigUpdate` and `Subscribe` are unvalidated (`514`, `533`).

### 2.3 Responses / events
`IpcResponse` (`response.rs:9-75`): `Ok, Error{code:u32,message}, Pong, Status, ProjectList, ProjectDetail, InviteToken, PeerList, TransferList, NetworkStatus, ConflictList, CheckoutReport, HistoryList(Vec<HistoryVersionDto>), HistoryGcReport, HistoryRotationReport, HistoryOffloaded, HooksList(Vec<HookInfoDto>), HooksRunReport(Vec<HookRunResultDto>), Tag(TagDto), TagList, Subscribed{subscription_id}`. Error codes are ad hoc (400 = validation, 1..5 per handler, 4 = "not a history node"). DTOs: `HistoryVersionDto{rel_path, version, hash, size, crc, stored_at, publisher, source}` (`114-123`), `TagDto{name, created_at, pins: Vec<(String,u64)>}` (`156-160`), `HookInfoDto{source, event, command, match, timeout_sec, disabled_by_opt_in}` (`79-88`).

`IpcEvent` (`event.rs:10-83`): `PeerConnected, PeerDisconnected, TransferProgress, TransferCompleted, TransferFailed, ConflictDetected, ConflictResolved, NetworkStatusUpdated, PeerStreamReceived{peer_id, magic, payload}`. `EventFilter::{All, Project(String), Category(EventCategory)}` (`87-105`).

### 2.4 History on-disk model
- Store dir: relative `root` → `<canonical project root>/<root>` with symlink/escape checks (`store.rs:81-116`); absolute `root` → `<root>/<blake3_hex(project_id)>` (`87`, `635-638`).
- `index.json` = `HistoryIndex{format:1, project_id, entries: BTreeMap<rel, BTreeMap<version, IndexEntry>>}` (`index.rs:54-66`); `IndexEntry{hash (64 hex blake3), size, crc, stored_at (ms), publisher, source, offloaded: Option<OffloadedRef>}` (`22-40`); `OffloadedRef{backend: "s3"|"local_path"|"gdrive", key, config: Option<RotationBackendConfig>}` (`43-52`).
- Objects: `objects/<hh>/<hash>` and sidecar `objects/<hh>/<hash>.meta.json` = `ObjectMeta{project_id, refs: Vec<ObjectRef>}` (`index.rs:177-223`). Index is rebuilt from sidecars when corrupt (`114-157`, `289-348`).
- Tags: `<store>/tags/<name>.json` = `Tag{format:1, project_id, name, created_at, pins: BTreeMap<rel, version>}` (`tags.rs:14-31`).
- Rotation key: `<blake3_hex(project_id)>/<blake3_hex(content)>` (`rotation/mod.rs:96-110`); S3 object path = `<prefix>/<key>` (`s3.rs:53-62`); local path = `<root>/<key>` (`local.rs:22-32`); GDrive file named `key` with `/`→`_` and `appProperties.rotationKey = key` in configured folder (`gdrive.rs:223-227`, `166-172`).
- `RotationBackend` trait: `put(key, path)`, `get(key, dest)`, `exists`, `size -> Option<u64>`, `delete` (`backend.rs:12-23`).
- Temp names: `objects/.archive-<uuid>.tmp` (staging copy, `store.rs:656`), `index-<uuid>.tmp` (`index.rs:163`), `<hash>.meta.json.tmp-<uuid>`→ via `with_extension` = `<hash>.meta.tmp-<uuid>` (`index.rs:243`), `<hash>.rotation-verify-<uuid>.tmp` (`rotation/mod.rs:245-248`), `<hash>.fetch-<uuid>.tmp` (`409`), `tags/<name>-<uuid>.tmp` (`tags.rs:179`).

### 2.5 Hooks model
`HookDef{event: "pre-publish"|"post-publish"|"post-receive", command: String, match: Vec<glob>, timeout_sec (default 60)}` (`synergos-net/src/config.rs:113-128`). Limits: command 1..=32 KiB, timeout 1..=86400 s, patterns 1..=1024 bytes no control chars (`config.rs:130-169`). Minimal glob (`*` no `/`, `**` any, `?` one non-`/`), DP matcher (`config.rs:185-258`). `hooks.toml` max 1 MiB (`project_file.rs:16`, `34-43`), parsed as `[[hook]]` array, each validated (`44-54`). Execution: `sh -c` / `cmd /C` + `raw_arg` (`runner.rs:439-457`), cwd project root, 5 env vars, stdio null, own process group (unix), `kill_on_drop` (`runner.rs:325-347`); timeout → SIGKILL to process group / `taskkill /T /F` (`361-378`, `387-430`).

---

## 3. Security controls PRESENT

### IPC authentication / isolation
- C1. Unix socket `chmod 0600` after bind (`ipc_server.rs:91-99`).
- C2. Peer uid check via `SO_PEERCRED` (Linux `252-270`) / `getpeereid` (macOS/BSD `272-290`) compared to `geteuid()`; mismatch → connection dropped before any read (`114-119`, `231-250`).
- C3. Windows: caller PID → `OpenProcess` → `OpenProcessToken` → `GetTokenInformation(TokenUser)` → `EqualSid` against daemon SID; mismatch → drop (`316-375`, `385-463`); first pipe instance created with `first_pipe_instance(true)` to reserve the name (`164-169`).
- C4. Accept-loop exponential backoff on accept errors (fd exhaustion) (`104-134`, `172-213`).
- C5. Socket file removed on shutdown (`143-145`).

### IPC input validation / limits
- C6. 1 MiB frame cap both directions + incremental buffer growth (`transport.rs:39`, `94-99`, `124-129`, `133-145`).
- C7. `IpcCommand::validate` runs before dispatch and returns `Error{400}` without side effects (`ipc_server.rs:506-517`; rules `command.rs:225-550`, see §2.2).
- C8. `PublishUpdate`: project root canonicalised; each path canonicalised, must `starts_with(project_root)`, must be regular file, `.synergos` excluded (`ipc_server.rs:773-826`); whole batch validated before any hook runs (`1276-1286`); all pre-publish hooks run before any manifest/version/history mutation (`1288-1304`); paths re-resolved after hooks and identity re-checked (`1306-1322`); CRC streamed (`1323-1340`).
- C9. `ProjectJoin` / `ProjectCreateInvite` validate bootstrap URLs before side effects (`967-978`, `1061-1070`) (owned by other subsystem, noted for completeness).
- C10. History/tag IPC handlers require the project to be open and (for mutating/rotation/fetch) `history.covers(project_id)` (`1544-1549`, `1587-1592`, `1648-1653`, `1705-1710`, `1741-1746`, `1772-1777`).
- C11. `HistoryGc` without `purge` always protects the live manifest versions plus `--keep-manifest` files (`1593-1621`); keep manifests are parsed with project-id/format/path validation (`manifest.rs:126-162`).
- C12. Path display in errors is redacted to project-relative / file name (`ipc_server.rs:777`, `synergos-net/src/types.rs:231-239`).

### History store integrity
- C13. `HistoryConfig::validate`: non-empty root, no surrounding whitespace, relative root may not contain `.`/`..`/root/prefix components, non-empty projects, rotation backend validated when enabled (`config.rs:433-463`, `385-425`).
- C14. Relative store dir walks each component with `symlink_metadata`, refuses symlinks and escapes outside the canonical project root (`store.rs:89-115`); absolute root uses hashed per-project subdir (`87`, `635-638`).
- C15. Per-project `tokio::sync::Mutex` serialises `archive`, `tag_add`, `tag_remove`, `gc`, `rotate`, `fetch_offloaded` (`store.rs:118-123`, `144-145`, `334-335`, `387-388`, `431-432`, `533-534`, `592-593`).
- C16. Archive snapshots to an immutable staging copy, then verifies size + CRC32 against publish-time values and blake3 size; mismatch → `InvalidData`, staging removed (`store.rs:648-695`).
- C17. Archive refuses different content for an existing `(rel, version)` (`store.rs:154-162`); existing object revalidated (hash/size/CRC) before dedup (`163-177`, `697-716`).
- C18. Atomic writes via tmp + rename for objects (`185`), index (`index.rs:160-172`), sidecars (`226-252`, `254-286`), tags (`tags.rs:176-189`).
- C19. Index load rejects foreign project / unknown format (`index.rs:118-124`); entries validated (`safe_rel_to_local`, version range, hex hash) else rebuilt from sidecars (`125-143`); rebuild filters invalid refs and unrecoverable entries (`316-330`); `is_valid_object_hash` gates object file names (`207-209`, `store.rs:795`).
- C20. `lookup` revalidates object (size, CRC, blake3) before serving (`store.rs:289-297`).
- C21. GC: new index persisted **before** object/sidecar deletion so failures only orphan objects (`store.rs:482-488`); purge writes an empty index first (`458-460`); purge refused while offloaded entries exist (`436-447`); temp sweep runs under the project lock (`503-509`, `730-756`).
- C22. Tags: name charset/length, `.`/`..` rejected (`tags.rs:46-54`); tags dir must be a real non-symlink dir inside the store (`70-92`); tag file must be a regular file (`104-116`); tag content validated (format, project, name==file name, pin paths/versions) (`118-148`, `151-168`); corrupt tag → `all_pins` **errors** so GC/rotation fail closed (`266-295`).
- C23. `.gitignore` excludes `.synergos/history/` (`Synergos/.gitignore:13`).

### Rotation / backends
- C24. Key format strictly `<64hex>/<64hex>` validated by backend-independent `is_valid_object_key` (`rotation/mod.rs:101-110`), enforced by `LocalDirBackend::dest_path` (`local.rs:22-32`) and at fetch (`rotation/mod.rs:369-375`).
- C25. Post-put verification: `exists` + `size == entry.size` + full download + CRC/size/blake3 check before marking offloaded (`rotation/mod.rs:195-274`, `482-493`); index + sidecar updated and saved **before** local object deletion; local object kept while any local ref shares the hash (`276-311`); put failures only append to `skipped` (`184-191`).
- C26. Fetch validates downloaded content; remote copy deleted only if no other offloaded ref uses the key; local object re-used if already valid (`351-479`).
- C27. Local backend: mount reachability check, fsync, atomic rename, idempotent delete (`local.rs:34-45`, `50-72`, `101-109`).
- C28. S3: credentials from environment / IMDS, never from config (`s3.rs:37-46`; `config.rs:360`); `NotFound` mapped cleanly (`93-97`, `106`, `115`).
- C29. GDrive: `token_uri` must equal Google's token endpoint (`gdrive.rs:88-93`); HTTP client refuses redirects so the JWT/bearer never leaks to another origin (`174-180`); rustls TLS (`synergos-core/Cargo.toml:24`); scope limited to `drive.file` (`22`); query values escaped (`162-172`); credentials path redacted from errors (`77-80`); JWT lifetime 3600 s (`98-104`).
- C30. `fetch_offloaded` uses backend config recorded at rotation time (so later config changes do not strand data) but always uses the **current** gdrive credentials file (`store.rs:595-615`).

### Hooks
- C31. Project hooks opt-in, default `false` (`config.rs:99-104`; `runner.rs:133`); disabled project hooks are listed but flagged (`runner.rs:93-115`).
- C32. `HookDef::validate` (event enum, command size, timeout bounds, pattern size/control chars) applied to daemon config and project file (`config.rs:134-169`; `project_file.rs:46-53`); `hooks.toml` capped at 1 MiB (`project_file.rs:16`, `34-43`).
- C33. Timeout kill of the whole process group / tree (`runner.rs:342-347`, `361-378`, `387-430`); `kill_on_drop` (`341`); stdio null so hooks cannot make the daemon buffer output (`336-340`).
- C34. Post-hook concurrency bounded to 16 batches (`runner.rs:11`, `87-90`, `211`).
- C35. Pre-publish failure/timeouts abort the publish before any mutation (`ipc_server.rs:1288-1304`; tested `tests/ipc_handlers.rs:506-553`, `558-604`, `754-807`).
- C36. Windows `taskkill` invoked by absolute path from `SystemRoot` with numeric args (`runner.rs:407-417`).

### Logging
- C37. Rejected clients, hook outcomes, archive/lookup failures, rotation results are logged via `tracing` (`ipc_server.rs:116`, `322`; `runner.rs:277-309`; `history/wiring.rs:21-24`, `69-71`; `daemon.rs:686-698`). `publisher` and `source` recorded per version (`store.rs:228-229`).

---

## 4. Weaknesses, TODOs, missing controls FOUND

Severity is a guess. "Exploitable" = reachable by an adversary with the stated position today; "design gap" = not an attack path by itself but a missing brake.

**W1 — Hooks inherit the full daemon environment, including cloud credentials.** `execute` only *adds* `SYNERGOS_*` vars; no `env_clear` (`runner.rs:325-335`; grep confirms no `env_clear`/`env_remove` under `src/hooks/`). The S3 rotation backend takes `AWS_ACCESS_KEY_ID/SECRET/SESSION_TOKEN/AWS_ENDPOINT` from the daemon env (`s3.rs:37-46`), so every hook — daemon hooks always, project hooks when `allow_project_hooks = true` — can read and exfiltrate the archive credentials and any other secret in the daemon's environment. Exploitable by anyone who can commit `.synergos/hooks.toml` on an opted-in node. Theme: security. Severity: high.

**W2 — Daemon-level hooks that reference project-relative scripts are remotely writable → RCE on every node.** Hooks run `sh -c <command>` with cwd = project root (`runner.rs:326`, `453-456`) and the docs' canonical examples are `command = "python scripts/convert.py"` and `scripts/lint-assets.sh` (`docs/hooks.md:44`, `57`). Any peer in the project can push a file to `scripts/lint-assets.sh` through the normal receive path (only `.synergos/` is excluded: `manifest.rs:301-322`), after which the "always enabled, not repository-derived" daemon hook (`docs/hooks.md:22-25`) executes attacker code with the daemon's privileges even with `allow_project_hooks = false`. No sandbox, rlimits, network isolation, or script hash pinning exist. Exploitable by any project member. Theme: security. Severity: high.

**W3 — post-receive hooks run unsandboxed on attacker-controlled input and env.** `SYNERGOS_FILE`/`SYNERGOS_PEER` come from the network (`hooks/wiring.rs:17-43`, `exchange/mod.rs:858-865`), the file content is whatever the peer sent, and the converter runs as the daemon user (`runner.rs:228-243`, `319-347`). A malicious asset exploiting an image/mesh parser inside a converter gives code execution on every history/CI node. Design gap (depends on the hook's parser). Theme: security / ai-game. Severity: medium.

**W4 — Project-hook trust is daemon-global and unpinned.** `allow_project_hooks` applies to all projects opened by the daemon (`runner.rs:133`; `docs/hooks.md:22-24`); there is no per-project allow list, no content hash/signature of `hooks.toml`, and no approval step when the file changes (`project_file.rs:26-55` re-reads the file on every event). Design gap. Theme: security. Severity: medium.

**W5 — IPC authorisation is OS-uid only; every same-uid client is a full administrator.** `verify_peer_uid` is the only check (`ipc_server.rs:231-250`); afterwards any client (CLI, GUI, the ars game-engine plugin, or any process the user runs) can `Shutdown` (`832-836`), `ProjectOpen` any directory (`862-883`), `HistoryGc{purge}` (`1576-1635`), `TagRm`, `HooksRun`, `PeerSendStream`. No capability scoping, no client identity, no Cernere identity, no audit trail. A malicious editor plugin / supply-chain package becomes a daemon takeover. Design gap, exploitable via malicious local software. Theme: security / ai-game. Severity: medium.

**W6 — `PeerSendStream` can forge reserved protocol magics to remote peers.** Validation only checks payload size (`command.rs:535-548`); the handler opens a bidi stream and writes `magic` + `payload` verbatim (`ipc_server.rs:2033-2068`), so a local client can inject `HLO1/DHT1/TXFR/GSP1/BSW1` frames into a peer connection authenticated as this daemon. Exploitable from any same-uid process (see W5). Theme: security / transfer. Severity: medium.

**W7 — Linux socket path falls back to world-writable `/tmp`; no directory ownership checks; client never authenticates the server.** `socket_path()` uses `/tmp/synergos/synergos.sock` when `XDG_RUNTIME_DIR` is unset (`transport.rs:58-61`, typical for systemd services, cron, containers). The server does `create_dir_all` + unconditional `remove_file` + `bind` (`ipc_server.rs:82-89`) without checking that the directory is owned by it with mode 0700. `IpcClient::open_stream` connects without checking the server's uid (`client.rs:73-85`). Another local user who pre-creates `/tmp/synergos` can delete/replace the socket (DoS) or run a fake daemon that harvests `ProjectJoin` invite tokens and feeds fake responses. Exploitable on multi-user hosts. Theme: security. Severity: medium.

**W8 — Peer-credential check fails open.** If `SO_PEERCRED`/`getpeereid` errors, the client is accepted (`ipc_server.rs:235-241`); on unsupported Unixes the function returns the daemon's own uid (`303-307`). Design gap. Theme: security. Severity: low.

**W9 — Windows named pipe has no DACL; PID→SID check is best-effort.** Pipe instances are created with default security (`ipc_server.rs:165-169`, `187-189`; the code comment admits "ACL を細かく設定していない" `313-315`); only the first instance uses `FIRST_PIPE_INSTANCE`; the SID check resolves the caller PID (`355-366`) which is racy against PID reuse. Design gap. Theme: security. Severity: low.

**W10 — No brake on destructive history operations: no confirmation, soft-delete, or audit log.** `HistoryGc{purge: true}` deletes every object and ignores tags and the live manifest (`store.rs:435-472`; keep set is empty when purging `ipc_server.rs:1594-1601`); `TagRm` drops protection immediately (`tags.rs:216-229`); nothing records who issued the command (the IPC layer has no client identity, §W5) and there is no quarantine/trash period. For the user's stated goal (「脆弱性および侵害の歯止め」) a single IPC message from a compromised local process irreversibly wipes the archive. Design gap. Theme: security. Severity: medium.

**W11 — No provenance / signature on archived versions; first writer wins.** `archive` accepts whatever `(rel, version, hash)` arrives first and permanently rejects different content for the same version (`store.rs:154-162`); `publisher` is just the transfer header's sender string (`exchange/mod.rs:836-846`), unsigned and unverified against the manifest or a Cernere identity. A malicious project member can race the legitimate publisher and pre-seed bogus content for the next version number; the real content is then rejected on every history node (`AlreadyExists`), and on the publisher's own history node the publish aborts (`ipc_server.rs:1402-1405`). Exploitable by any project member. Theme: security / ai-game. Severity: medium.

**W12 — History store, tags and index live inside the project tree and are writable by hooks and any local process.** Default root is `.synergos/history` under the project (`config.rs:470`, `store.rs:85-115`); hooks run with cwd = project root and daemon privileges (`runner.rs:326`), so a pre-publish/project hook can rewrite `index.json`, sidecars, objects and tags. Fetch trusts `OffloadedRef.config` stored in the index (`store.rs:595-615`), so a tampered index can redirect a fetch to an attacker-chosen `local_path`/bucket. No HMAC/signature over the index. Design gap (becomes exploitable with W2/W4). Theme: security. Severity: medium.

**W13 — Any peer can force cold-storage fetches (and deletions) via `FileWant`.** `serve_from_history` is reachable from the gossip `FileWant` of any peer (`exchange/mod.rs:898-934`, `962-1005`); `HistoryStore::lookup` auto-fetches offloaded versions (`store.rs:271-288`) and `rotation::fetch` deletes the remote copy after restoring locally ("move semantics", `rotation/mod.rs:439-456`). A peer iterating over `history offloaded`-style version numbers forces egress cost, refills local disk, and un-offloads the archive; there is no rate limit, quota, or authorisation for old-version requests. Exploitable by any project member. Theme: transfer / security. Severity: medium.

**W14 — Rotation backends buffer whole objects in RAM and use single-shot uploads unsuitable for game/AI assets.** S3 `put` reads the entire file (`s3.rs:69-71`) and `get` buffers the full body (`83-87`); GDrive `put` builds a complete multipart body in memory (`gdrive.rs:202-234`) and uses `uploadType=multipart`, which Google limits to 5 MB **(inference from Drive API docs)**; S3 single `PUT` is capped at 5 GB **(inference)**; local `put` reads the whole file into memory before writing (`local.rs:57`). Multi-GB assets will OOM the daemon or fail. Theme: transfer. Severity: high.

**W15 — Per-project lock is held across all backend network I/O; no HTTP timeouts in the GDrive client.** `rotate` and `fetch_offloaded` hold the project mutex for the entire run (`store.rs:533-560`, `591-617`; `rotation/mod.rs:162-315`), and the receive path `await`s `archive_to_history` before completing a transfer (`exchange/mod.rs:836-857`). While a rotation or a peer-triggered fetch runs, every newly received version for that project blocks at `archive` (`store.rs:144-145`) and post-receive hooks/`complete_transfer` stall. The GDrive `reqwest::Client` has no connect/read timeout (`gdrive.rs:174-180`), so an unreachable endpoint can hold the lock indefinitely. Design gap. Theme: transfer. Severity: medium.

**W16 — 1 MiB IPC frame cap with no pagination silently kills the connection on large projects.** `HistoryList`, `TagShow`, `TagAdd` (response echoes all pins), `HooksList`, `ProjectList`, `PeerList` serialise unbounded vectors (`ipc_server.rs:1555-1569`, `1807-1812`, `1851-1855`); `write_message` fails above 1 MiB (`transport.rs:94-99`), `send_server_message` propagates the error with `?` (`ipc_server.rs:552`) and the client just sees `ConnectionClosed` (`client.rs:131`). For a 50k-file manifest `TagAdd` creates the tag and then drops the connection. Design gap at ai-game scale. Theme: transfer / ai-game. Severity: medium.

**W17 — Index is one pretty-printed JSON rewritten and fully re-parsed on every operation (O(N) per op, O(N²) cumulative).** `index.save` serialises everything (`index.rs:160-172`) on every archive (`store.rs:247`), rotate (`rotation/mod.rs:303`) and fetch (`478`); every `lookup`/`list`/`offloaded` re-reads and re-parses the whole file (`store.rs:267`, `311`, `571`); rotate's per-candidate dedup/ref scans are `iter_all().any` (`rotation/mod.rs:171-178`, `306-308`). Design gap. Theme: transfer / ai-game. Severity: medium.

**W18 — Redundant full-file passes on archive and on every lookup.** Archive = full copy + CRC pass + blake3 pass (`store.rs:648-695`) although the receive path already computed CRC and the transfer verified blake3; `lookup` re-reads and re-hashes the whole object before every `FileWant` answer (`store.rs:289-297`, `697-716`). The staging copy happens *before* the dedup check (`146` vs `163-177`), so even deduplicated versions pay a full copy. Theme: transfer. Severity: low.

**W19 — Temp files from rotation verification, fetch and tags are never reclaimed.** `sweep_stale_temporaries` only matches `index-*.tmp`, `.archive-*.tmp`, `*.meta.tmp-*` (`store.rs:730-756`); `<hash>.rotation-verify-<uuid>.tmp` (`rotation/mod.rs:245-254`, full object size), `<hash>.fetch-<uuid>.tmp` (`409-416`) and `tags/<name>-<uuid>.tmp` (`tags.rs:179`) are left behind after a crash; `collect_object_hashes` ignores them because they contain `.` (`store.rs:795`). Design gap (disk leak). Theme: transfer. Severity: medium.

**W20 — Availability bug: a locally valid fetched object is not served while the remote delete fails.** `fetch` returns `Err` when `backend.delete` fails even though the object has been restored and validated locally (`rotation/mod.rs:449-456`); `lookup` then logs and returns `None` (`store.rs:278-281`), so checkout/restore/FileWant fail for the duration of a backend outage, contradicting the stated intent (`rotation/mod.rs:349-350`; `docs/versioning-design.md:283-288`). Theme: transfer. Severity: medium.

**W21 — Daemon rotation timer protects only the live manifest; `--keep-manifest` protections cannot be made persistent.** `spawn_rotation_timer` passes only `manifest_entries` as `keep` (`daemon.rs:679-685`), whereas the design says rotation uses the same protection set as GC including `--keep-manifest` (`docs/versioning-design.md:275-277`). Release-manifest versions that an operator protects only on the CLI will be offloaded by the timer. Design gap. Theme: transfer. Severity: low.

**W22 — No automatic GC, no quotas: a peer can fill the history node's disk.** Retention knobs are applied only by the IPC `HistoryGc` command (`ipc_server.rs:1576-1635`); `spawn_periodic_cleanup` has no history GC (`daemon.rs:574-606`); `archive` has no per-project/per-peer size or count limit and every received version from any peer is stored (`exchange/mod.rs:836-857`, `store.rs:128-249`). Defaults are unbounded (`config.rs:296-307`). Exploitable DoS by a project member. Theme: security / transfer. Severity: medium.

**W23 — `PublishUpdate` mutates the manifest before archiving; archive failure leaves a bumped manifest with no Offer.** Order per file: `bump_file_version` (`ipc_server.rs:1356-1380`) → `register_file` (`1385-1386`) → `archive_to_history` (`1388-1406`, returns `Error` on failure) → only after all files `publish_updates` (`1418`). On an archive error (e.g. W11 collision, disk full) earlier files are bumped and archived but never offered. A retry re-offers (`1368-1373`) so this is recoverable, but the state is inconsistent in between. Theme: security / transfer. Severity: low.

**W24 — Full commands logged at debug level, including invite tokens and 1 MiB payloads.** `tracing::debug!("Received command: {:?}", command)` (`ipc_server.rs:504`) prints `ProjectJoin.invite_token` and `PeerSendStream.payload`. Design gap. Theme: security. Severity: low.

**W25 — Event relay loses events silently and dies silently.** Broadcast lag (`RecvError::Lagged`) is treated as `continue` (`ipc_server.rs:607`, `621`, `635`, `647`, `662`, `674`, `685`); any write error (including `MessageTooLarge` for a big `PeerStreamReceived` payload) `break`s the relay without telling the client (`691-694`); `Unsubscribe` ignores `subscription_id` (`544-549`); `TransferProgress/Completed` are relayed with empty `peer_id` (`628`, `642`); `EventFilter::Project` passes events that carry no project id (`721-725`). The client's event channel is unbounded (`client.rs:56`). Theme: transfer. Severity: low.

**W26 — No per-connection limits or timeouts; pre-publish hooks are unbounded.** Unlimited concurrent connections and no idle/read timeout (`ipc_server.rs:107-141`, `489-555`); pre-publish hooks run sequentially per file with timeout up to 24 h × up to 1024 files per publish and no cross-connection cap (`runner.rs:148-194`; only post hooks are capped `runner.rs:11`). Same-uid only. Theme: security. Severity: low.

**W27 — Second daemon instance silently hijacks the socket.** `remove_file(&path)` before bind with no lock/pid file (`ipc_server.rs:86-89`). Theme: security. Severity: low.

**W28 — Hook execution is unobservable.** stdout/stderr are discarded (`runner.rs:339-340`); outcomes carry only exit code (`HookStatus`, `58-67`); no record of the `hooks.toml` content hash, the command actually run, or the peer that triggered a post-receive hook is persisted. Incident response cannot reconstruct what ran. Theme: security. Severity: low.

**W29 — `hooks.toml` is re-read from disk per event (TOCTOU vs `hooks ls`, perf).** `matching_hooks` calls `project_file::load` for every file (`runner.rs:134`); a 1024-file publish parses the file ~2048 times, and the file listed by `hooks ls` may differ from the one executed. Theme: security. Severity: low.

**W30 — Tags are node-local, unsigned and not replicated.** Tags live in `<store>/tags/` per node (`tags.rs:1-6`, `14-15`); with multiple history nodes (allowed by design, `docs/versioning-design.md:104`) a release tag created on node A does not protect versions on node B, which may GC/offload them. Design gap. Theme: ai-game. Severity: medium.

**W31 — Eviction and offload are age-only.** `apply_retention` `max_bytes` and `select_candidates` sort purely by `stored_at` (`gc.rs:92-116`; `rotation/mod.rs:113-139`); there is no access-time tracking, size/importance weighting, or per-path policy, so large but still-needed datasets/assets are evicted or offloaded first. Design gap. Theme: ai-game. Severity: low.

**W32 — Lock-free read paths rebuild and save the index; TOCTOU between lookup and gc.** `load_or_rebuild` writes a rebuilt index from `lookup`/`list`/`offloaded` without the project lock (`index.rs:139-141`, `149-151`; callers `store.rs:267`, `311`, `571`), racing `archive`/`gc`/`rotate` saves (last rename wins). `gc` may delete an object right after a lock-free `lookup` validated it and handed its path to `share_and_send` (`store.rs:289-297` vs `485-502`). Theme: security. Severity: low.

**W33 — Index/sidecar/object writes are not fsync'ed.** `tokio::fs::write` + rename for index (`index.rs:166-167`), sidecars (`246-247`, `280-281`), tags (`tags.rs:182-186`) and the staged object rename (`store.rs:185`) never call `sync_all`/`sync_data`; only the local rotation backend fsyncs (`local.rs:62`). A power loss can leave a truncated `index.json` (recoverable via sidecars) or truncated objects that then fail validation and are GC'd as orphans. Design gap. Theme: transfer. Severity: low.

**W34 — Cold-storage objects are plaintext and keyed by content hash.** Rotation uploads raw objects under `<blake3(project)>/<blake3(content)>` (`rotation/mod.rs:96-99`; `s3.rs:67-75`; `gdrive.rs:199-255`); no client-side encryption, and the key reveals the content hash (dictionary attack on known assets). For an "opaque custody" product this contradicts the stated goals. Design gap. Theme: security. Severity: medium.

**W35 — No IPC protocol versioning / handshake.** `ServerMessage`/`IpcCommand` are bare MessagePack enums (`transport.rs:49-52`; `command.rs:11-12`); a client built against a different enum layout gets a decode error and a closed connection (`ipc_server.rs:496-501`). Theme: transfer. Severity: low.

**W36 — Archive accepts any `version` the sender claims.** The receive path takes `header.version.max(1)` from the wire (`exchange/mod.rs:750`) and the history node archives it (`836-846`); combined with W11, a peer can populate arbitrary version numbers (e.g. v1_000_000) for a path, which the index keeps forever (no automatic GC, W22). Theme: security / ai-game. Severity: low.

**W37 — Pre-publish auto-convert hooks mutate the working copy with no rollback.** Docs encourage hooks that rewrite the file in place (`docs/hooks.md:31`, `134-147`); if the hook fails after partially rewriting, the publish aborts (`ipc_server.rs:1296-1303`) but the user's source file is already damaged and nothing restores it. Design gap. Theme: ai-game. Severity: low.

**W38 — GDrive backend re-mints a service-account token for every call.** `access_token()` runs on each `put/get/exists/size/delete` (`gdrive.rs:200`, `258`, `288`, `294`, `308`), so one rotated object costs 4–5 JWT exchanges plus the verification download (`rotation/mod.rs:184-274`); Google quota and latency scale accordingly. Theme: transfer. Severity: low.

**W39 — Credentials/secret handling does not follow org rules (Infisical).** S3 creds come from process env (`s3.rs:37-46`), GDrive from a JSON file path in config (`gdrive.rs:75-83`, `config.rs:370-373`); nothing integrates a secret manager, and the env route is what W1 leaks. Design gap. Theme: security. Severity: low.

**W40 — Stale/misleading docs and comments.** `ipc_server.rs:5` says Windows named pipe is "future work" while it is implemented (`154-216`); `docs/hooks.md:22-25` claims daemon hooks are safe because they are not repository-derived, but the examples run repository files (W2). Theme: other. Severity: low.

---

## 5. Performance characteristics

- **IPC framing:** 4-byte header + MessagePack; max 1 MiB per frame; read in 64 KiB increments (`transport.rs:39`, `42`, `133-145`). One `tokio` task per connection; commands on a connection are processed strictly sequentially (`ipc_server.rs:489-555`) — a long `PublishUpdate` (pre-publish hooks up to `timeout_sec` × files, archive copies) blocks that client's later commands. Writer is a `Mutex` shared with the event relay (`563-569`). Client side: single reader task, FIFO oneshot queue, unbounded event mpsc (`client.rs:45-50`, `56`).
- **Hashing:** blake3 over the full file with a 64 KiB buffer (`synergos-net/src/transfer/mod.rs:25`, `289-313`); CRC32 (`crc32fast`) with a 256 KiB buffer (`manifest.rs:436-450`). `project_dir_name`/rotation key use blake3 of the project id (`store.rs:636-638`, `rotation/mod.rs:97-99`).
- **Archive cost per version:** 1 full copy (staging) + 1 CRC read + 1 blake3 read (+ 1 CRC + 1 blake3 read of the existing object when dedup hits, `store.rs:163-177`) + sidecar read/rewrite + whole-index rewrite (`store.rs:146-247`). Disk I/O pattern: copy → verify → rename into `objects/<hh>/`. No fsync. Dedup is per whole file only.
- **Lookup cost:** full index parse + CRC + blake3 of the object (`store.rs:267-297`), then the exchange streams the file. With rotation: + backend `size` + download + CRC + blake3 + rename + remote `delete` + sidecar + index rewrite, all under the project lock (`store.rs:591-617`; `rotation/mod.rs:351-479`).
- **GC:** `apply_retention` is O(N log N) (`gc.rs:27-120`); `gc` walks all shard directories (`store.rs:780-802`), computes `referenced_hashes` (set of all hashes), removes objects individually, then sweeps temps (`store.rs:489-509`).
- **Rotation:** sequential per candidate; per object: `put` (full read into RAM + upload) → `exists` → `size` → full `get` to a temp file → CRC + blake3 → sidecar + index rewrite → maybe local delete (`rotation/mod.rs:162-315`). Net ≈ 2× object bytes over the network, 4–5 backend round trips (GDrive: + 1 `files.list` and 1 token exchange per call, `gdrive.rs:134-159`, `86-131`). Candidate scans `iter_all().any` per candidate → O(C·N). Timer default every 24 h (`config.rs:352-354`; `daemon.rs:661-665`, first tick discarded).
- **Hooks:** `hooks.toml` (≤ 1 MiB) read + TOML parse per event (`runner.rs:134`); glob DP O(tokens × path length) per pattern (`config.rs:185-258`); hooks per file run sequentially (`runner.rs:157-192`, `228-243`); post hooks: ≤ 16 concurrent batches, excess batches wait as tokio tasks (unbounded queue) (`runner.rs:11`, `208-212`); pre-publish is synchronous within the IPC handler (`ipc_server.rs:1290-1304`).
- **Memory hot spots:** S3/GDrive/local `put` and S3/GDrive `get` hold the full object in RAM (`s3.rs:69`, `83-87`; `gdrive.rs:202`, `233`, `279-283`; `local.rs:57`). `PeerSendStream` payload up to 1 MiB in the command (`command.rs:540`). `PeerStreamReceived` events clone the payload per subscriber (`ipc_server.rs:676-686`).
- **Potential O(n²):** index rewrite per archive (W17); rotation candidate scans (W17); `apply_retention` max_bytes loop is O(N log N) but `victims.contains` on a `BTreeSet<(String,u64)>` with cloned strings per entry (`gc.rs:81`) allocates per version.

---

## 6. Config knobs & defaults

| Knob | Default | Where | Secure-by-default? |
|---|---|---|---|
| `[history] enabled` | `false` | `config.rs:296-307` | yes (feature off) |
| `history.projects` | `["*"]` | `config.rs:466-468` | when enabled, archives every open project |
| `history.root` | `.synergos/history` (relative) | `config.rs:470` | inside project tree (W12); validated `433-455` |
| `max_versions_per_file` / `max_age_days` / `max_bytes` | `0` = unlimited | `config.rs:302-304` | **no** — unbounded growth, and only applied by manual `history gc` (W22) |
| `[history.rotation] enabled` | `false` | `config.rs:328-337` | yes |
| `rotation.offload_after_days` | `90` | `config.rs:348-350` | `0` allowed → offload everything unprotected |
| `rotation.interval_hours` | `24` (`0` = manual only) | `config.rs:352-354`; `daemon.rs:658-661` | timer clamps to ≥ 60 s |
| `rotation.backend` | `Unset` → `validate` error when enabled | `config.rs:374-382`, `419-422` | yes |
| S3 creds/endpoint | env `AWS_*`, `AWS_ENDPOINT` | `s3.rs:37-46` | creds not in config; but env is inherited by hooks (W1) |
| GDrive `credentials_file` | path, `~/` expanded via `HOME`/`USERPROFILE` | `gdrive.rs:182-195` | file perms not checked |
| `[hooks] allow_project_hooks` | `false` | `config.rs:99-104` | yes |
| `hooks.hooks` | `[]` | `config.rs:105-107` | — |
| `HookDef.timeout_sec` | `60`, max `86400` | `config.rs:126-130` | 24 h max is very permissive |
| `HookDef.command` | ≤ 32 KiB, shell-interpreted | `config.rs:131`, `runner.rs:439-457` | no sandbox |
| `MAX_CONCURRENT_POST_HOOK_BATCHES` | `16` (const) | `runner.rs:11` | not configurable |
| `MAX_HOOKS_FILE_BYTES` | 1 MiB (const) | `project_file.rs:16` | — |
| IPC `MAX_MESSAGE_SIZE` | 1 MiB (const) | `transport.rs:39` | not configurable (W16) |
| IPC socket path | `$XDG_RUNTIME_DIR` → `/tmp` fallback | `transport.rs:56-62` | **no** on Linux without XDG (W7) |
| IPC socket mode | `0600` | `ipc_server.rs:95` | yes |
| Windows pipe ACL | default | `ipc_server.rs:165-169` | partial (W9) |
| Config file | `synergos.toml` via `--config`; missing → defaults with a warning | `daemon.rs:557-571` | — |

---

## 7. Test coverage observed

**Unit tests (in-crate):**
- `store.rs:804-1625`: archive/lookup/dedup/AlreadyIndexed, not-covered project, source-changed rejection, same-version-different-content rejection, absolute-root hashed subdir, gc retention + purge, tag protection under `max_versions_per_file=1`, purge ignores tags, `protected_versions` merge with missing pins, non-history node tag ops denied, temp reclamation, rotation offload→fetch round trip (local backend), transparent fetch on lookup, keep/tag protection in rotate, unreachable backend leaves state untouched, dry-run, shared-hash object retention, delete-after-last-reference, fetch uses recorded backend config, corrupt index rebuild keeps offloaded entries.
- `index.rs:350-446`: insert/remove/reference set, sharding, corrupt index rebuild from sidecar + persisted, foreign project rejected.
- `gc.rs:122-216`: unlimited, max_versions + protected, max_age, offloaded never removed, max_bytes counting unique objects.
- `tags.rs:297-449`: name validation, round trip, invalid names, invalid pins, corrupt tag fails `all_pins`, symlinked tags dir rejected (unix), foreign project rejected, pin merge, empty dir.
- `rotation/mod.rs:495-571`: wrong-size backend does not delete local object. `local.rs:112-168`: round trip, unreachable root error, unsafe keys. `s3.rs:121-137`: prefix path only. `gdrive.rs:329-354`: query escaping, folder scoping, `~` expansion. **No network-backed tests** for S3/GDrive (documented as manual, `docs/versioning-design.md:309-314`).
- `hooks/runner.rs:459-679`: pre-publish ok/fail/timeout, descendant kill (unix), non-matching skip, opt-in off/on, `effective_hooks` flags, `run_manual`. `project_file.rs:57-133`: missing/parse/malformed/invalid/oversize.
- `synergos-ipc/transport.rs:151-380`: round trip, EOF, too-large write/read, partial header, multiple frames, socket path env (linux), serde of command/response/event/filters. `command.rs:553-696`: validation cases incl. rel-path escapes, PeerSendStream size, tag names, pins.

**Integration tests (`synergos-core/tests`):**
- `history_node_e2e.rs`: two real QUIC+gossip nodes; history node archives received v1/v2/v3, normal node stores nothing, restore of v1 is served from the history node and manifest rewinds, re-publish jumps to v3, history node restores locally (`363-541`).
- `ipc_handlers.rs`: dispatch-level — history list/restore/gc purge (`188-275`), tags round trip + GC protection + rm (`280-417`), plain node refuses gc (`421-453`), pre-publish abort leaves manifest unchanged (`506-553`), later-file failure leaves whole batch unpublished (`558-604`), project hooks ignored when opt-in off (`609-634`), `hooks run` env (`640-697`), spawned post-receive env incl. version/peer (`703-750`), timeout kill + abort (`754-807`).
- `ipc_integration.rs`: duplex ping, ordering, malformed frame closes server, disconnect cleanup. `ipc_client_events.rs`: response/event multiplexing, `is_daemon_running` false. `ipc_subscribe.rs`: All filter delivery, unsubscribe stops push, category filter.

**Not tested:** uid/SID rejection (`verify_peer_uid` / `verify_windows_caller`), real UDS `IpcServer::run` (socket perms, `/tmp` fallback), concurrent archive vs gc/rotate, lock-free rebuild races (W32), fetch when remote delete fails (W20), rotation timer (`daemon.rs:653-708`), `--keep-manifest` on timer path (W21), responses > 1 MiB (W16), `PeerSendStream` with reserved magics (W6), event-relay lag/oversize behaviour (W25), hook env inheritance (W1), hooks writing into `.synergos/` (W12), S3/GDrive behaviour with large objects (W14), temp leak of rotation/fetch temps (W19), `EventFilter::Project` on project-less events, `offload_after_days = 0`.

---

## 8. Documented-vs-implemented deltas

- `docs/versioning-design.md:275-277` (rotation protection set = GC's incl. `--keep-manifest`): CLI path matches (`ipc_server.rs:1654-1676`), **daemon timer passes only the live manifest** (`daemon.rs:679-685`) — W21.
- `docs/versioning-design.md:283-288` and `rotation/mod.rs:349-350` ("retry from the verified local copy if only the remote delete failed"): `lookup` does not serve the local copy in that state — W20.
- `docs/hooks.md:22-25` / `spec/tasks/2026-08-24-publish-hooks.md:56-60` ("daemon hooks are always enabled because they are not repository-derived"): the documented example commands execute repository-relative scripts with cwd = project root (`docs/hooks.md:44`, `57`), which are writable by peers — W2. No warning is given.
- `docs/hooks.md:73-83` env table: implemented exactly (`runner.rs:327-335`); the doc does not state that the hook inherits the daemon's entire environment — W1.
- `docs/hooks.md:36-37` (16 concurrent post batches): implemented (`runner.rs:11`).
- `docs/hooks.md:31` / spec `:40-42` (pre-publish before version/manifest mutation): implemented (`ipc_server.rs:1288-1304` precede `1356`).
- `docs/versioning-design.md:148-149` (absolute root hashed subdir; relative root rejects `..`/symlinks): implemented (`store.rs:85-115`; `config.rs:443-455`).
- `docs/versioning-design.md:204` (object refcount by scanning the index): implemented via `referenced_hashes` (`store.rs:490-502`).
- `docs/versioning-design.md:303` (no cloud secrets in config): true; index/sidecar persist the **backend config** (bucket/prefix/region/path/folder_id/credentials_file path, not secrets) (`index.rs:48-51`, `rotation/mod.rs:277-281`).
- `docs/versioning-design.md:306-308` (purge refused while offloaded; normal gc never deletes offloaded): implemented (`store.rs:436-447`; `gc.rs:42`, `67`, `81`, `95`).
- `docs/versioning-design.md:123` (disabling history keeps the store): true — nothing deletes it; but `HistoryGc` is refused on a non-covering node (`ipc_server.rs:1587-1592`), so a store left behind after disabling cannot be purged through IPC without re-enabling (design wrinkle).
- `ipc_server.rs:3-5` header says Windows named pipe is "future"; implemented at `154-216` — stale.
- `docs/versioning-design.md:142` example shows `"size": 0, "stored_at": 0` for a published entry; implementation always records real size/time (`store.rs:217-246`) — doc example is stale.
- There is **no document** for the IPC protocol itself (framing, auth model, error codes, event filters); only code comments.

---

## 9. Open questions for designers

1. **Who is authoritative for `(project, rel, version)`?** Should history nodes require a publisher signature (node identity / Cernere identity) over `(project, rel, version, blake3, size)` before archiving, and how are conflicting candidates for the same version represented (W11, W36)?
2. **IPC trust model:** is the ars plugin (inside a game engine process) really meant to have the same authority as the CLI? Should IPC carry a per-client token with capabilities and map to a Cernere identity for audit (W5, W10)?
3. **Hook sandboxing:** which runtime (bwrap/firejail/AppContainer, WASM, or container) and which env allow-list? Should hook scripts be required to live outside the synced tree or be hash-pinned/signed (W1–W4)?
4. **Cold storage semantics:** copy vs move on fetch, who authorises old-version requests from peers, rate limits/quotas per peer, cost attribution (W13)?
5. **Encryption at rest for offloaded objects** and key management via Infisical/KMS; should keys be HMAC'd rather than raw content hashes (W34, W39)?
6. **Scale targets:** expected number of files × versions per project; is a single JSON index acceptable, or is an embedded KV store required (W17)? Should IPC responses be paginated/streamed and the frame cap raised for local clients (W16)?
7. **Automatic GC and quotas:** should retention run on a timer, and should archive enforce per-project/per-peer byte quotas with a disk watermark (W22)?
8. **Tag replication:** should tags live in git (signed) or be gossiped between history nodes so that multi-history-node setups protect the same versions (W30)?
9. **Durability:** required fsync policy for index/objects (W33)?
10. **Windows pipe DACL and Linux runtime-dir policy:** should the daemon refuse to start when it cannot create a private runtime directory (W7, W9)?
11. **Pre-publish auto-conversion:** should the daemon snapshot the source before running mutating hooks so a failed conversion can be rolled back (W37)?
12. **IPC protocol versioning** and a documented spec (W35, §8).
