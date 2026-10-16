# Map: synergos-core versioning & exchange

Subsystem: `synergos-core` — `exchange` (+ `history_hooks`), `manifest`, `project`, `checkout` / `restore`, `version_state`, `conflict`, `daemon`, `event_bus`, `cli`, `main`, `lib`.
All paths below are relative to `/home/user/Synergos/` unless absolute. Line numbers refer to the files as read on 2026-10-15.

Adjacent code read to verify claims (not part of this subsystem, cited where load-bearing): `synergos-net/src/transfer/mod.rs`, `synergos-net/src/chain/ledger.rs`, `synergos-net/src/gossip/{node,message,transport}.rs`, `synergos-net/src/quic/{mod,hello}.rs`, `synergos-net/src/config.rs`, `synergos-core/src/ipc_server.rs` (PublishUpdate / Checkout / Restore / Conflict handlers only), `synergos-core/src/peer_bootstrap.rs` (URL validation only), `synergos-core/src/history/wiring.rs`, `synergos-core/src/hooks/wiring.rs`.

---

## 1. Responsibilities & data flow

### 1.1 What this subsystem is

The "versioning" layer of Synergos is deliberately thin (docs/versioning-design.md §0): there is **no delta storage and no delta transfer**. Each project has a git-committed lock file `<root>/.synergos/manifest.json` mapping relative path → `{version, size, crc, updated_at, publisher}`. Whole files are pushed over QUIC (`TXFR` streams) and verified with BLAKE3; the receiver recomputes CRC32 and records the version into its own manifest. Old versions are only retained by nodes with `[history] enabled = true` (history store, outside this map but wired here via `history_hooks`).

### 1.2 Components and their roles

| Component | File | Role |
|---|---|---|
| `Daemon` | `synergos-core/src/daemon.rs` | Composition root. Builds net stack (`Identity`, `DhtNode`, `GossipNode`, `QuicManager`, `TunnelManager`, `Mesh`, `Conduit`), `ProjectManager`, `Exchange`, `HistoryStore`, `HookRunner`, `PresenceService`, `ConflictManager`, `ServiceContext`; restores persisted projects; runs IPC server + background tasks (cleanup, rotation timer, gossip heartbeat, QUIC accept/dispatch, gossip subscriber, gossip fanout, catalog sync, peer-info HTTP, control heartbeat, bootstrap). |
| `Exchange` | `synergos-core/src/exchange/mod.rs` | Transfer controller: Want/Offer ledger, `shared_files` registry, gossip `FileOffer`/`FileWant`/`CatalogUpdate` broadcast, QUIC send (`execute_send`) and receive (`handle_incoming_transfer`), pinned fetches for checkout/restore, history/receive/post-receive hooks, `ActiveTransfer` table + progress events. |
| `history_hooks` | `synergos-core/src/exchange/history_hooks.rs` | Type definitions for the two closures the daemon injects when history is enabled (`archive`, `lookup`). |
| `ProjectManager` | `synergos-core/src/project.rs` | Open/close/update projects, invite tokens, `file_id → relative path` registry, in-memory manifests + per-project mutex, node-local `VersionState` high-water marks, persistence of the project list to `projects.json`. |
| `ProjectManifest` | `synergos-core/src/manifest.rs` | `.synergos/manifest.json` model, version bump logic, path safety (`safe_rel_to_local`, `safe_join_under_root`), atomic replace, CRC32 streaming helper, metadata dir validation. |
| `VersionState` | `synergos-core/src/version_state.rs` | `.synergos/state.json`: per-path highest version ever confirmed on this node (survives git rollback of the manifest). |
| `checkout` | `synergos-core/src/checkout.rs` | `project checkout` (reconcile working tree with a manifest by issuing pinned `FileWant`s) and `project restore` (single file to a version, locally from the history store or via pinned `FileWant`). |
| `restore` | `synergos-core/src/restore.rs` | Startup: rebuild `Exchange.shared_files` from manifests after verifying each file's CRC/size. |
| `ConflictManager` | `synergos-core/src/conflict/mod.rs` | In-memory conflict registry (equal-version/different-content detection record, chain-fork detection function, hot-standby queue). |
| `CoreEventBus` | `synergos-core/src/event_bus.rs` | Type-indexed `tokio::sync::broadcast` channels (capacity 256) for intra-daemon events. |
| `cli` / `main` | `synergos-core/src/cli.rs`, `main.rs` | clap CLI; `start` runs the daemon in-process, every other subcommand is a thin `synergos_ipc::IpcClient` call. Logging init (`RUST_LOG`, `SYNERGOS_LOG_FORMAT=json`). |

### 1.3 Inputs → outputs

**Publish path (local user → network)**

1. CLI `project publish <id> <files...>` → `IpcCommand::PublishUpdate` (`cli.rs:387-404`).
2. `ipc_server.rs:1251-1438`: canonicalize project root, `resolve_publish_path` for every path (containment, regular file, not `.synergos/`), run `pre-publish` hooks, re-resolve, stream CRC32 (`manifest::crc32_of_file`), `ProjectManager::bump_file_version` (per-project mutex, floor from `VersionState`), `register_file(project, FileId(rel), rel)`, `Exchange::archive_to_history(source="published")`, then `Exchange::publish_updates(Vec<PublishNotification>)`, then spawn `post-publish` hooks.
3. `Exchange::publish_updates` (`exchange/mod.rs:1317-1454`): per file → ledger `register_offer`, `clear_fetch_pin`, `shared_files.insert`, gossip `FileOffer{sender,file_id,version,size,crc,content_hash=0}`; then one msgpack `RootCatalog` snapshot `put` into `MemoryContentStore` and one gossip `CatalogUpdate{project_id, root_crc, update_count=len, updated_chunks=[], catalog_cid, publisher}`.
4. Gossip fan-out task (`daemon.rs:847-907`) opens one QUIC `Control` stream per mesh peer per message and writes `GSP1` + signed envelope.

**Auto-pull path (network → local working tree)**

1. Gossip subscriber (`daemon.rs:917-1057`) receives `FileOffer` on topic `project/<id>`; rejects version 0 / `u64::MAX`; `Exchange::handle_file_offer` registers remote offer in the ledger; if local record has same version but different size/CRC → `ConflictManager::record_version_conflict` and stop; else if sender ≠ self, `!has_shared_file(project, file, version)` and project open → spawn `Exchange::fetch_file(FetchRequest{version, source_peer: Some(sender)})`.
2. `fetch_file` (`exchange/mod.rs:1243-1315`): inserts a Queued `ActiveTransfer`, ledger `register_want`, gossip `FileWant{requester, file_id, version}`.
3. On the publisher, subscriber → `Exchange::handle_file_want` (`898-958`): if `shared_files[(project,file)]` exists and version matches (or want version 0) → spawn `share_and_send` (= `share_file` + `spawn_send_after_share` → `hash_file` → `execute_send`); otherwise `serve_from_history` (history node only).
4. `execute_send` (`627-683`): opens QUIC `Data` bidi stream to the requester, writes `TXFR` + header + 64 KiB chunk frames + footer (`synergos-net/src/transfer/mod.rs:71-112, 250-262`), emits `TransferProgressEvent` every 128 KiB read, `complete_transfer` on success.
5. On the requester, QUIC accept loop → `dispatch_peer_streams` (`daemon.rs:766-842`) reads 4-byte magic; `TXFR` → `Exchange::handle_incoming_transfer(recv, sender)` (`689-889`): peek header → resolve final path via `ProjectManager::resolve_file_path` → write to `<root>/.synergos/incoming/incoming-<uuid>.part` with chunk/total BLAKE3 verification → recompute CRC32 → version/pin/stale/conflict checks → re-resolve path → `replace_file_atomically` → `shared_files.insert` → `received_hook` (`ProjectManager::record_received_file`) → `archive_to_history(source="received")` → `post_receive_hook` → synthetic `ActiveTransfer` + `complete_transfer` (`TransferCompletedEvent`, ledger `mark_fulfilled`).

**Checkout / restore path**

- `project checkout <id> [--manifest <path>]` → `IpcCommand::ProjectCheckout` → `checkout::checkout_project` (`checkout.rs:55-116`): reload (or replace) manifest, for each entry compare working-tree size+CRC; equal → `restore_shared_file`; different → `pin_fetch_version(..., Some((size, crc)))` + `fetch_file(version)` (broadcast `FileWant`).
- `project restore <id> <rel> --version N` → `checkout::restore_file` (`119-207`): if local history store has it → copy into `.synergos/incoming/restore-<uuid>.part`, re-resolve, atomic replace, `record_received_file(force=true)`; else `pin_fetch_version(..., None)` + `fetch_file`.

**Startup restore**: `restore::restore_shared_files_from_manifests` (`restore.rs:18-79`) walks every open project's manifest, verifies each file (`safe_rel_to_local`, `safe_join_under_root`, `is_file`, CRC+size) and registers it in `ProjectManager.file_paths` + `Exchange.shared_files`.

### 1.4 Cross-crate calls

- `synergos_net::quic::QuicManager` (`open_stream`, `accept`, `list_connections`, `get_bandwidth_estimate`, `shutdown`), `synergos_net::quic::StreamType::{Data, Control}`.
- `synergos_net::transfer::{peek_transfer_header, receive_over_quic_with_header, send_over_quic, hash_file, TransferHeader, TRANSFER_STREAM_MAGIC}`.
- `synergos_net::chain::{TransferLedger, OfferEntry, WantEntry, LedgerAction, LedgerEntryState}` (exchange) and `{FileChain, ChainBlock}` (conflict `detect_conflict`).
- `synergos_net::gossip::{GossipNode, GossipMessage, GossipWireMessage, send_gossip, handle_gossip_stream, GOSSIP_STREAM_MAGIC}`.
- `synergos_net::content::{MemoryContentStore, Block, ContentStore, handle_bitswap_stream, BITSWAP_STREAM_MAGIC}`.
- `synergos_net::catalog::{RootCatalog, ChunkIndex, FileEntry, FileState, CatalogManager}`.
- `synergos_net::dht::{DhtNode, handle_dht_stream, DHT_STREAM_MAGIC}`, `identity::Identity`, `mesh::Mesh`, `tunnel::TunnelManager`, `conduit::Conduit`, `promotion::{NetCapabilities, PromotionMode}`, `config::NetConfig`, `types::{FileId, PeerId, TopicId, TransferId, Cid, Blake3Hash, ChunkId, now_ms}`.
- `synergos_ipc::{IpcClient, IpcCommand, IpcResponse, response::{ProjectDetail, ProjectInfo}}`.
- Intra-crate: `ipc_server::{IpcServer, ServiceContext}`, `presence::{PresenceService, NodeRegistry}`, `history::{HistoryStore, wiring::build_hooks}`, `hooks::{HookRunner, wiring::build_post_receive_hook}`, `catalog_sync::CatalogSyncService`, `peer_info_server`, `peer_bootstrap`, `control_heartbeat`, `cli_history`, `cli_hooks`.

---

## 2. Key types / APIs / wire formats

### 2.1 Public API surface (this crate)

```rust
// exchange/mod.rs
pub trait FileSharing { share_file, fetch_file, publish_updates, list_transfers, get_transfer, cancel_transfer, pause_transfer, resume_transfer }   // :234-261
pub struct ShareRequest { project_id, file_id, file_path, file_size, crc: u32, priority, target_peer: Option<PeerId>, version: u64 }           // :156-169
pub struct FetchRequest { project_id, file_id, source_peer: Option<PeerId>, priority, version: u64 /* 0 = any latest */ }                    // :173-181
pub struct PublishNotification { project_id, file_id, file_path, file_size, crc, version }                                                   // :204-211
pub struct ActiveTransfer { transfer_id, project_id, file_id, file_name, file_size, bytes_transferred, speed_bps, direction, peer_id, state, priority, version } // :185-200
pub enum TransferState { Queued, Running, Paused, Completed, Failed(String), Cancelled }                                                    // :139-152
pub struct SharedFileRecord { project_id, file_path, file_size, crc, version }                                                              // :322-328
pub type OutPathResolver   = Arc<dyn Fn(&str, &FileId) -> Option<PathBuf>>                                                                  // :34
pub type ReceivedHook      = Arc<dyn Fn(String, FileId, u64, u64, u32, PeerId, bool) -> BoxFuture>   // (project, file, version, size, crc, sender, pinned) :40-45
pub type IncomingDirResolver = Arc<dyn Fn(&str) -> Option<PathBuf>>                                                                        // :54
pub type PostReceiveHook   = Arc<dyn Fn(String, FileId, u64, PeerId)>                                                                       // :60
impl Exchange { new, with_network, attach_history_hooks, pin_fetch_version, unpin_fetch_version, archive_to_history, attach_quic, attach_receive_hooks, attach_post_receive_hook, restore_shared_file, local_peer_id, has_shared_file, shared_file_record, attach_content_store, gc_finished_transfers, ledger, execute_send, handle_incoming_transfer, handle_file_want, handle_file_offer, complete_transfer, update_progress }
pub async fn share_and_send(Arc<Exchange>, ShareRequest) -> Result<TransferId>                                                              // :1104
pub async fn spawn_send_after_share(Arc<Exchange>, ShareRequest, TransferId)                                                                // :1120
const PINNED_FETCH_TTL_MS: u64 = 5 * 60 * 1000;                                                                                             // :26

// exchange/history_hooks.rs
pub struct ArchiveRequest { project_id, file_id, version, size, crc, publisher: String, source: &'static str /* "published"|"received" */, path }
pub struct HistoryHit { path, size, crc }
pub struct HistoryHooks { archive: ArchiveHook, lookup: HistoryLookup }

// project.rs
pub trait ProjectConfiguration { open_project, close_project, get_project, update_project, list_projects, create_invite, join_project, count } // :136-177
impl ProjectManager { new, with_gossip, with_state_path, load_state, save_state, register_file, resolve_file_path, gc_expired_invites, close_all, bump_file_version, record_received_file, reload_manifest, replace_manifest, manifest_entries, note_confirmed_version, highest_confirmed_version, project_root }
pub struct InviteToken { token: String /* uuid v4 */, project_id, expires_at: Option<u64>, uses_remaining: u32 }                            // :93-101
pub struct PersistedState { projects: Vec<PersistedProject{project_id, root_path, display_name, description, sync_mode, max_peers, created_at}> } // :226-239

// manifest.rs
pub const META_DIR = ".synergos"; MANIFEST_FILE = "manifest.json"; INCOMING_DIR = "incoming";                                                // :27-31
pub struct ManifestEntry { version: u64, size: u64, crc: u32, updated_at: u64, #[serde(default)] publisher: String }                       // :35-46
pub struct ProjectManifest { #[serde(default=1)] format: u32, project_id: String, #[serde(default)] files: BTreeMap<String, ManifestEntry> } // :50-56
pub enum BumpOutcome { Bumped(u64), Unchanged(u64) }                                                                                        // :64-69
impl ProjectManifest { new, incoming_dir, load, load_from_file, from_bytes, save, bump, bump_at_least, record_received, record_received_with, get }
pub fn normalize_rel_path(&Path) -> String; safe_rel_to_local(&str) -> Option<PathBuf>; safe_join_under_root(&Path, &str) -> Option<PathBuf>; replace_file_atomically(&Path,&Path); crc32_of_file(&Path) -> (u32, u64)

// version_state.rs
pub const VERSION_STATE_FILE = "state.json";
pub struct VersionState { format: u32, project_id: String, versions: BTreeMap<String, u64> }  // load, save, highest, note

// checkout.rs
pub struct CheckoutReport { requested: Vec<(String,u64)>, up_to_date: usize, extra: Vec<String> }
pub enum RestoreOutcome { RestoredLocally, Requested, AlreadyAtVersion }
pub struct CheckoutContext<'a> { projects, exchange, history }
pub async fn checkout_project(&ctx, project_id, Option<&Path>) -> io::Result<CheckoutReport>
pub async fn restore_file(&ctx, project_id, rel, version) -> io::Result<RestoreOutcome>

// conflict/mod.rs
pub enum ConflictResolution { KeepLocal, AcceptRemote, ManualMerge }
pub enum ConflictState { Active, Resolved{resolution} }
pub struct ConflictInfo { file_id, common_ancestor_hash: Option<Blake3Hash>, common_ancestor_version, local_version, local_author, remote_version, remote_author, involved_peers, detected_at, state, file_path, project_id }
impl ConflictManager { new, record_version_conflict, detect_conflict, resolve_conflict, list_conflicts, get_conflict, cleanup_resolved, queue_notification, drain_pending_notifications, handle_conflict_alert, cleanup_expired_notifications }

// event_bus.rs
pub trait CoreEvent: Clone + Send + Sync + 'static { fn event_name() -> &'static str }
pub struct CoreEventBus { channels: DashMap<TypeId, Box<dyn Any + Send + Sync>> }  // emit<E>, subscribe<E>; CHANNEL_CAPACITY = 256 (:20)
Events: PeerConnectedEvent, PeerDisconnectedEvent, TransferProgressEvent{transfer_id,file_name,bytes_transferred,total_bytes,speed_bps}, TransferCompletedEvent{transfer_id,file_name,file_path}, ConflictDetectedEvent{project_id,file_id,file_path,involved_peers}, CatalogSyncNeededEvent{project_id,remote_root_crc,remote_update_count,changed_chunks,catalog_cid,publisher}, CatalogSyncCompletedEvent, NetworkStatusEvent, PeerStreamReceivedEvent{peer_id,magic:[u8;4],payload:Vec<u8>}

// daemon.rs
pub struct DaemonConfig { config_path: Option<PathBuf> }
pub struct NetworkHandles { local_peer_id, identity, net_config, dht, gossip, quic, tunnel, mesh, conduit }
impl Daemon { new(DaemonConfig), run(self), shutdown(self) }
```

### 2.2 On-disk formats

| File | Writer | Format | Notes |
|---|---|---|---|
| `<root>/.synergos/manifest.json` | `ProjectManifest::save` (`manifest.rs:165-179`) | pretty JSON `{format:1, project_id, files:{rel:{version,size,crc,updated_at,publisher}}}`; keys are `/`-separated relative paths | Written to `manifest-<uuid>.tmp` in the same dir then `replace_file_atomically`. Loader rejects `format != 1`, project id mismatch, unsafe paths, `version ∈ {0, u64::MAX}` (`manifest.rs:132-162`). |
| `<root>/.synergos/state.json` | `VersionState::save` (`version_state.rs:71-83`) | `{format:1, project_id, versions:{rel:u64}}` | Node-local high-water marks; same atomic pattern; same validation (`:41-69`). |
| `<root>/.synergos/incoming/*.part` | `handle_incoming_transfer`, `restore_file` | raw file bytes | `incoming-<uuid>.part` / `restore-<uuid>.part`; must be a real dir under a real `.synergos` (`manifest.rs:89-112`). |
| `<identity dir>/projects.json` | `ProjectManager::save_state` (`project.rs:302-333`) | pretty JSON `PersistedState` | **Non-atomic** `tokio::fs::write`. Location = parent of `Identity::default_path()` (`daemon.rs:153-156`). |

### 2.3 Wire formats touched by this subsystem

**TXFR (file transfer) — `synergos-net/src/transfer/mod.rs`**

```
stream = b"TXFR" ‖ frame*                                         (magic consumed by dispatcher, daemon.rs:788-805)
frame  = u32 LE length ‖ msgpack(FrameKind)                      (:324-349)
FrameKind::Header(TransferHeader{ transfer_id: String, project_id: String, file_id: String,
                                  total_size: u64, chunk_count: u64, total_hash: Blake3Hash(32B),
                                  #[serde(default)] version: u64 })   (:28-46)
FrameKind::Chunk(ChunkFrame{ index: u64, hash: Blake3Hash, data: serde_bytes })   (:49-58)
FrameKind::Footer(TransferFooter{ total_hash })                                    (:61-65)
CHUNK_SIZE = 64 KiB (:25); MAX_FRAME = CHUNK_SIZE + 1024 (:339); chunk_count must equal ceil(total_size/64Ki) (:132-142)
```

Receiver checks: chunk length ∈ (0, 64 KiB], per-chunk BLAKE3, strictly increasing index, running size ≤ `total_size`, footer hash == running hash == header hash, chunk count and size equality (`:171-239`). There is **no CRC in the header**; the receiver's CRC32 is computed from the written temp file (`exchange/mod.rs:743`). The header is **unauthenticated** (no signature; project_id/file_id/version are attacker-controlled strings).

**GSP1 (gossip) — `synergos-net/src/gossip/transport.rs`, `message.rs`**

```
stream = b"GSP1" ‖ u32 BE length ‖ msgpack(GossipWireMessage{ topic: TopicId, signed: SignedGossipMessage{ message, sender_public_key: Vec<u8>(32), signature: Vec<u8>(64) } })
MAX_GOSSIP_FRAME = 256 KiB (transport.rs:30)
GossipMessage::FileOffer { sender, file_id, version, size, crc, content_hash }   // content_hash always zero from Exchange (exchange/mod.rs:572-575)
GossipMessage::FileWant  { requester, file_id, version }
GossipMessage::CatalogUpdate { project_id, root_crc, update_count, updated_chunks, catalog_cid, publisher }
GossipMessage::ConflictAlert { file_id, conflicting_nodes, their_versions }      // no signer-bound field (message.rs:85-86)
GossipMessage::PeerStatus { status, origin, .. }
```

Signature verification binds `FileOffer.sender`, `FileWant.requester`, `CatalogUpdate.publisher`, `PeerStatus.origin` to the ed25519 signer (`message.rs:36-89`). Topic is part of the message id (`node.rs:183-186, 310-317`). The project id is carried only as the topic `project/<id>` (`daemon.rs:931, 937`).

**HLO1 (peer identity)** — first bidi stream after QUIC handshake, ≤ 4096 bytes, 5 s timeout, `SignedHello{peer_id, public_key, ts_ms}`; server derives `peer_id = blake3(pubkey)[:20]` and verifies the signature (`quic/hello.rs:77-103, 134-173`; `quic/mod.rs:382-390`). This is the only authentication of inbound peers — there is **no allowlist, ACL, or project-membership check** anywhere in the accept path.

**Extension magic streams** — any other 4-byte magic: payload read with `read_to_end(1 MiB)` and re-emitted as `PeerStreamReceivedEvent` (`daemon.rs:777, 818-839`).

**RootCatalog snapshot (Bitswap)** — `build_catalog_snapshot` (`exchange/mod.rs:272-317`): msgpack `RootCatalog{project_id, update_count, chunks:[ChunkIndex{chunk_id: random, crc, content_hash: 0, last_updated}], catalog_crc, last_updated}` with `FileEntry{file_id, path: n.file_path.to_string_lossy() /* absolute local path */, crc, content_hash: 0, state: Synced, size}`.

### 2.4 Identifiers & limits

- `FileId` == normalized relative path (`ipc_server.rs:1382`, `checkout.rs:85`, `restore.rs:60`); `TopicId::project(id)` == `"project/<id>"`.
- Project id: non-empty, ≤ 256 chars, no control chars, no `/` or `\` (`project.rs:687-694`).
- Versions: `u64`, must be `1..u64::MAX-1` in manifests/state; transfer header `version` default 0 → coerced to 1 (`exchange/mod.rs:750`).
- Pin TTL 5 min (`:26`). Event bus channel capacity 256 (`event_bus.rs:20`). Periodic cleanup every 60 s (`daemon.rs:580`). Gossip heartbeat clamped to 100 ms..60 s (`daemon.rs:1066`). Conflict hot-standby TTL 24 h (`daemon.rs:586`).
- Progress reporter: every 128 KiB read on the sender (`exchange/mod.rs:85, 110`), channel size 16 with `try_send` (lossy) (`:80, 109`).

---

## 3. Security controls PRESENT

### 3.1 Authentication / identity

| Control | Evidence |
|---|---|
| Inbound QUIC peers must complete a signed `HLO1` hello; the derived `PeerId` is what `dispatch_peer_streams` and `handle_incoming_transfer` use as `sender` | `synergos-net/src/quic/mod.rs:382-407`; `synergos-net/src/quic/hello.rs:77-103`; `daemon.rs:733, 806` |
| Outbound QUIC pins the expected server `PeerId` against the TLS cert (`PeerPinningVerifier`) | `synergos-net/src/quic/mod.rs:205-258, 463-466` |
| All wire gossip is signed with the node's ed25519 identity and verified before delivery; `sender`/`requester`/`publisher`/`origin` fields must equal the signer's derived `PeerId` | `daemon.rs:80-84` (`set_identity`); `synergos-net/src/gossip/transport.rs:99-104`; `node.rs:216-226`; `message.rs:36-89`. The unsigned path `on_message_received` has no wire caller (grep over `*.rs`: only its definition at `node.rs:230`). |
| `handle_file_want` / `handle_file_offer` ignore messages whose `requester`/`sender` is the local peer | `exchange/mod.rs:905-907, 1020-1022` |
| Invite tokens are uuid v4, one-shot by default (`uses_remaining: 1`), optional expiry with overflow-checked arithmetic, decremented and removed atomically under a `DashMap` entry guard; token value is never logged; join path is redacted in logs | `project.rs:861-886, 896-924, 877-882, 931` |
| Expired invites are garbage collected every 60 s | `project.rs:363-371`; `daemon.rs:589-592` |
| IPC socket is `chmod 0600` and peer uid is checked (outside this map, noted for completeness) | `ipc_server.rs:91-96, 114-115, 231-243` |
| Bootstrap URLs are validated (`http`/`https` only, no credentials, no query) and HTTP redirects are disabled — problem-log fix #6 verified present | `peer_bootstrap.rs:85-98, 193-198`; tests `:339-344` |

### 3.2 Integrity of transferred content

| Control | Evidence |
|---|---|
| Per-chunk BLAKE3 + whole-file BLAKE3 (header and footer must both match), strict chunk ordering, length bounds, size/count equality | `synergos-net/src/transfer/mod.rs:171-239` |
| Frame length capped at `CHUNK_SIZE + 1024` before allocation; header `chunk_count` validated against `total_size` before any disk write | `transfer/mod.rs:132-142, 336-344` |
| Receiver recomputes CRC32 of the fully written temp file (never trusts a remote CRC) | `exchange/mod.rs:743-749` |
| `version == u64::MAX` rejected on receive; `FileOffer` with version 0 or `u64::MAX` dropped before ledger/auto-pull | `exchange/mod.rs:751-756`; `daemon.rs:944-947` |
| Pinned (checkout) transfers must match the manifest's expected `(size, crc)` | `exchange/mod.rs:759-766`; `checkout.rs:101-108` |
| Stale (`local.version > version`) and equal-version/different-content pushes are rejected unless explicitly pinned; the temp file is deleted | `exchange/mod.rs:767-784` |
| Gossip path records equal-version/different-content offers as conflicts and keeps local content (problem-log fix #4 verified for the gossip path) | `daemon.rs:956-977`; `conflict/mod.rs:104-143` |
| Startup restore refuses to offer files whose on-disk CRC/size differ from the manifest | `restore.rs:44-59` |
| Checkout/restore compare working tree by size+CRC before requesting | `checkout.rs:86, 135, 214-222` |

### 3.3 Path containment / filesystem safety

| Control | Evidence |
|---|---|
| `safe_rel_to_local`: rejects empty, leading `/`, any `\`, control chars, `.`/`..`/empty segments, `:` in a segment, and first segment `.synergos` | `manifest.rs:301-322`; tests `:466-484` |
| `safe_join_under_root`: canonicalizes root, walks each component with `symlink_metadata`, rejects symlinks/junctions and anything resolving outside root (problem-log fix #2 verified) | `manifest.rs:328-351`; test `:488-509` |
| Receive path re-resolves the destination after `create_dir_all` and aborts if it changed; restore does the same | `exchange/mod.rs:785-797`; `checkout.rs:167-176` |
| Incoming dir must be exactly `<project>/.synergos/incoming`, a real (non-symlink) directory whose canonical path stays under `.synergos` | `exchange/mod.rs:709-727`; `manifest.rs:89-112` |
| `.synergos` itself must be a real directory inside the canonical root | `manifest.rs:371-393, 395-406` |
| `resolve_file_path` always goes through `normalize_rel_path` + `safe_join_under_root`, even for the `file_id`-as-path fallback | `project.rs:347-360` |
| Manifest / state loaders reject unsafe paths | `manifest.rs:150-160`; `version_state.rs:58-67, 90-96` |
| Publish side: canonicalize, `starts_with(project_root)`, regular-file check, `.synergos/` publish refused; whole batch validated before hooks and re-validated after hooks with path-identity check | `ipc_server.rs:771-826, 1276-1322` |
| Project root canonicalized and must be a directory; project id validated | `project.rs:687-706` |
| `replace_manifest` / `from_bytes` refuse a manifest whose `project_id` differs | `project.rs:533-538`; `manifest.rs:144-149` |

### 3.4 Atomicity / durability / locking

| Control | Evidence |
|---|---|
| Atomic same-directory replace (`rename` on Unix, `MoveFileExW(REPLACE_EXISTING|WRITE_THROUGH)` on Windows); old destination preserved on failure (problem-log fix #1 verified) | `manifest.rs:355-369, 408-433`; test `:608-624` |
| Temp names are uuid-based (not PID-based) | `manifest.rs:168`; `version_state.rs:74`; `exchange/mod.rs:733`; `checkout.rs:165` |
| Per-project `tokio::sync::Mutex` serializes bump / record / reload / replace / note (problem-log fix #3 verified) | `project.rs:212, 411-416, 428-429, 476-477, 516-517, 539-540, 578-579`; test `core_services.rs:122-153` |
| High-water mark persisted **before** the manifest so a crash between the two only skips numbers | `project.rs:453-460` |
| Monotonic version floor: `bump_at_least` uses `max(local+1, floor+1)`; `VersionState::note` only moves forward | `manifest.rs:196-233`; `version_state.rs:90-103`; test `tests/checkout.rs:126-158` |
| Received temp file is removed on every failure branch | `exchange/mod.rs:736-740, 745-748, 752, 761, 773, 793, 799` |
| Pins are keyed by generation (`unpin` only removes a matching version); pins are cleared when a newer local state is accepted or republished | `exchange/mod.rs:428-439, 495-499, 818-823, 1368`; unit tests `:1542-1568` |
| `shared_files` keyed by `(project_id, FileId)` (problem-log cross-project fix verified for this map) | `exchange/mod.rs:359, 519-527`; test `auto_transfer_e2e.rs:186-226` |
| Gossip message id includes the topic (project) | `synergos-net/src/gossip/node.rs:183-186, 310-317` |

### 3.5 Resource limits

| Control | Evidence |
|---|---|
| Extension-magic stream payload ≤ 1 MiB | `daemon.rs:777, 821` |
| Gossip frame ≤ 256 KiB; hello ≤ 4 KiB with 5 s timeout | `gossip/transport.rs:30, 88-92`; `quic/hello.rs:31, 155-159, 170-172` |
| Finished transfers reaped every 60 s; conflicts/notifications cleaned every 60 s | `daemon.rs:584-598`; `exchange/mod.rs:536-546` |
| Heartbeat interval clamped | `daemon.rs:1066` |
| Hook execution has timeouts and process-group kill; repository-provided hooks are opt-in (`allow_project_hooks` default `false`) | `hooks/runner.rs:341-435`; `synergos-net/src/config.rs:99-104`; test `config.rs:963` |

### 3.6 Logging / observability

| Control | Evidence |
|---|---|
| `tracing` with `RUST_LOG` filter and optional JSON output | `main.rs:27-49` |
| Publish pipeline logs each step at `info` with file id/version/size | `exchange/mod.rs:1338-1452` |
| History archive failure after a successful receive is surfaced as `warn` (working copy kept) | `exchange/mod.rs:836-857` |
| History archive failure on publish is fatal to the publish | `ipc_server.rs:1388-1406` |

---

## 4. Weaknesses, TODOs, missing controls FOUND

Severity guesses are mine. "Exploitable" = a remote party can trigger it with the code as read; "design gap" = missing capability. Theme tags: **security** / **transfer** / **ai-game** / other.

### W-01 No project authorization on inbound transfers or wants — any reachable peer can write into, and read from, any open project
- Evidence: QUIC accept only verifies a self-generated identity (`synergos-net/src/quic/mod.rs:362-407`, `hello.rs:134-173`); `dispatch_peer_streams` forwards every `TXFR` stream to `handle_incoming_transfer` (`daemon.rs:805-808`); `handle_incoming_transfer` accepts any `project_id`/`file_id` in the header as long as the project is open and the version is not stale (`exchange/mod.rs:703-801`); `handle_file_want` pushes any shared file to any `requester` (`:898-958`); `serve_from_history` serves any stored version (`:962-1005`). No membership list, ACL, invite check, or Cernere identity is consulted anywhere. Acknowledged in `spec/plan/problem_logs/2026-08-16-versioned-transfer-safety.md:44`.
- Theme: security. Severity: critical. Exploitable.
- Experience: anyone who learns a project id (visible in gossip topics `project/<id>`, `daemon.rs:931`) and can reach the QUIC port can (a) exfiltrate every published asset by sending signed `FileWant{version:0}` for guessed paths, and (b) overwrite working-tree files on every member by pushing a `TXFR` with a higher version.
- Redesign needs: per-project membership with a verifiable credential (Cernere-issued, per org rules), enforced at stream dispatch (before reading the header) and in `handle_file_want`/`serve_from_history`; signed offers bound to a membership key; ideally project-scoped QUIC ALPN or per-project session keys.

### W-02 Version-number poisoning permanently breaks publishing of a path on the victim
- Evidence: receive accepts any `version < u64::MAX` (`exchange/mod.rs:750-756`) and records it as the node's high-water mark (`project.rs:492-493`). A later local publish computes `max(local+1, floor+1)` with `saturating_add` → `u64::MAX` (`manifest.rs:208-211, 219`), then `note_confirmed_version_unlocked` fails `is_valid_entry` (`version_state.rs:91-96, 106-108`) → `bump_file_version` returns `Err` before saving (`project.rs:456-458`).
- Theme: security. Severity: high. Exploitable (given W-01).
- Experience: after one malicious push with `version = u64::MAX - 1`, `project publish` of that path fails forever with "manifest update failed"; the inflated number also propagates to every peer that accepts the push.
- Redesign needs: versions must be authenticated (signed by the publisher), or replaced by content-addressed revisions with a vector clock / parent hash rather than a bare monotonic integer trusted from the wire.

### W-03 First-writer-wins for a new version; no binding between the gossip offer and the pushed bytes
- Evidence: `FileOffer.content_hash` is always zero (`exchange/mod.rs:572-575`, `TODO(S5)`); `handle_incoming_transfer` never compares the pushed `total_hash` with any offer; the equal-version conflict check only fires if the receiver already holds that version (`:767-784`). Any peer may answer a `FileWant` (`handle_file_want` has no "am I the publisher" check beyond having a record).
- Theme: security. Severity: high. Exploitable (given W-01).
- Experience: attacker observes `FileOffer v2` from the real publisher and races a `TXFR v2` with malicious bytes; the receiver stores it and then **rejects the genuine v2** as "conflicting content for existing version" (`:769-782`), logged only at `debug` (`daemon.rs:807`).
- Redesign needs: offers must carry a publisher-signed `(project, path, version, blake3, size)` record; the receiver must verify the transfer's `total_hash` against the signed offer and store the signature as provenance in the manifest.

### W-04 Remote peers can write files anywhere in the project except `.synergos/` — including `.git/hooks/*` and IDE/tool config
- Evidence: `safe_rel_to_local` only blocks the first segment `.synergos` (`manifest.rs:315`); the `file_id`-as-path fallback is used whenever `register_file` was not called locally (`project.rs:349-356`), i.e. for every path the receiver has not itself published.
- Theme: security. Severity: high. Exploitable (given W-01).
- Experience: a peer pushes `.git/hooks/pre-commit`, `.cargo/config.toml`, `.vscode/tasks.json`, `Makefile`, etc.; the next `git commit` / IDE open runs attacker code. Received files are created with default mode via `File::create` (`transfer/mod.rs:166`), so on Unix the hook may not be executable, but `.cargo/config.toml` (`[target.*] runner`) and similar do not need the bit.
- Redesign needs: receiver-side allowlist of syncable paths (manifest-declared set, or project `.synergosinclude`), explicit deny of VCS/tool directories, and never materializing paths that were not announced in a signed manifest.

### W-05 Problem-log fix #7 ("Reject peers using the pre-versioned transfer protocol") is NOT implemented
- Evidence: `TransferHeader.version` is `#[serde(default)]` (`transfer/mod.rs:44-45`) and the receiver coerces 0 → 1 (`exchange/mod.rs:750`). No protocol-version negotiation exists in `hello.rs` (grep for `version`/`PROTOCOL_VERSION` only hits test names at `:188, 197`).
- Theme: security / transfer. Severity: medium. Design gap (documented as fixed).
- Experience: an old or hostile peer can push version-less transfers that are treated as v1 and overwrite files on receivers that have no manifest entry.
- Redesign needs: explicit protocol version in `HLO1` and in `TransferHeader` with hard rejection, plus a version-negotiation test.

### W-06 CRC32 (+ size) is the content identity for conflict detection, "unchanged" detection and checkout matching
- Evidence: `manifest.rs:206` (`Unchanged` when size+crc equal), `exchange/mod.rs:769-770`, `daemon.rs:957-960`, `checkout.rs:214-222`, `restore.rs:52`. BLAKE3 is computed for transfer (`exchange/mod.rs:1139`) but never persisted or compared.
- Theme: security. Severity: medium. Exploitable with a crafted CRC32 collision (trivial for CRC32 with 4 free bytes).
- Experience: attacker crafts content with the victim's `(size, crc)` → conflict detection and pinned-fetch verification (`:759-766`) pass; on the publisher a tampered file with equal CRC is treated as "unchanged" and never republished.
- Redesign needs: store BLAKE3 in `ManifestEntry`, use it for all equality checks, and put it into `FileOffer.content_hash` (the field already exists).

### W-07 `FileWant` → full-file push amplification with no rate limit, dedupe or quota
- Evidence: `handle_file_want` spawns `share_and_send` for every want (`exchange/mod.rs:936-957`); the doc comment claims the ledger would skip already-fulfilled peers (`:896-897`) but the code never consults ledger state before sending; `mark_fulfilled` only touches the `wants` map (`ledger.rs:123-132`).
- Theme: security / transfer. Severity: medium. Exploitable.
- Experience: a few-hundred-byte signed gossip message makes the publisher (or a history node) push a multi-GB asset repeatedly to the requester or to the victim named as `requester`... note the signature binds `requester` to the signer (`message.rs:70-75`), so the amplification target is the attacker itself or any node whose key the attacker holds; the victim is the uploader's bandwidth/disk I/O.
- Redesign needs: per-peer token bucket on outbound pushes, in-flight dedupe keyed by `(requester, project, file, version)`, and a want-queue with priorities (which the type `TransferPriority` already implies).

### W-08 Unbounded inbound `TXFR` streams: disk fill and no per-peer concurrency / size cap
- Evidence: each incoming stream is spawned independently (`daemon.rs:779-841`); `receive_body` writes until the stream ends or size exceeds the header (`transfer/mod.rs:171-207`); there is no max `total_size`, no free-space check, no per-peer in-flight limit (only `QuicConfig.max_concurrent_streams`, default 100, `config.rs:707`); the temp file is removed only after the stream fails (`exchange/mod.rs:736-740`). No idle timeout beyond QUIC's 30 s (`config.rs:708`).
- Theme: security / transfer. Severity: medium. Exploitable (given W-01).
- Experience: 100 parallel streams with `total_size = 2^40` fill `.synergos/incoming` until the disk is full; legitimate publishes then fail on `manifest.save`.
- Redesign needs: per-project / per-peer quotas, a maximum asset size, reservation of free space, and admission control before `create_dir_all`/`File::create`.

### W-09 Stale/conflicting pushes are rejected only after the entire file has been downloaded
- Evidence: version, pin, stale and conflict checks run after `receive_over_quic_with_header` and CRC pass (`exchange/mod.rs:734-784`); the header already carries `version`, `project_id`, `file_id`, `total_size`.
- Theme: transfer. Severity: medium. Design gap (also amplifies W-08).
- Experience: a 20 GB stale push is fully written, hashed and CRC'd, then deleted.
- Redesign needs: evaluate version/stale/pin/size rules from the peeked header before accepting the body; reject with a QUIC stream reset.

### W-10 Downloads never complete their own `ActiveTransfer`; Queued receive entries leak and receive progress is never reported
- Evidence: `fetch_file` inserts a Queued entry keyed by a locally generated `TransferId` (`exchange/mod.rs:1244-1272`), but `handle_incoming_transfer` creates a **new** entry keyed by the sender's `transfer_id` (`:868-887`) and completes that one; `gc_finished_transfers` only reaps Completed/Cancelled/Failed (`:536-546`). No `TransferProgressEvent` is emitted on the receive side (`bytes_transferred` jumps to `total_size` at the end, `:878`).
- Theme: transfer. Severity: medium. Design gap.
- Experience: `transfer list` shows ever-growing Queued downloads; GUI progress bars for downloads never move; memory grows with every offer.
- Redesign needs: correlate the pushed `transfer_id` with the local want (carry the requester's transfer id in `FileWant`/header), emit progress from `receive_body`, and expire Queued entries.

### W-11 `pause`/`resume`/`cancel` are state flags only; nothing stops bytes
- Evidence: `pause_transfer`/`resume_transfer` flip `state` (`exchange/mod.rs:1500-1530`); `execute_send` never checks state (`:627-683`); `cancel_transfer` does not abort the spawned task or reset the QUIC stream (`:1473-1498`); `Daemon::shutdown` "cancels" and then relies on `quic.shutdown()` (`daemon.rs:532-548`).
- Theme: transfer. Severity: medium. Design gap (API promises more than it does).
- Redesign needs: hold a `JoinHandle`/abort token per transfer; use QUIC `stop`/`reset` codes; implement resume via range request (header `offset`).

### W-12 No priority queue, bandwidth control, concurrency limit or resume despite the module contract
- Evidence: module doc claims "優先度キュー、帯域制御、チャンク分割・再組立" (`exchange/mod.rs:3-4`); `TransferPriority` is stored but never read for scheduling; `share_and_send` spawns immediately (`:1104-1116`); the transfer protocol is single-stream, sequential, no resume (`transfer/mod.rs:10-12`).
- Theme: transfer / ai-game. Severity: medium. Design gap.
- Experience: a `checkout` of a large manifest fires every `FileWant` at once; publishers open one QUIC stream per file; interactive requests cannot preempt background sync; a dropped connection restarts a multi-GB asset from byte 0.
- Redesign needs: a scheduler with per-peer concurrency, priorities and bandwidth caps; resumable ranged transfers keyed by content hash; multi-source fetching.

### W-13 Lost updates between concurrent editors are undetectable (no parent version / causal history in offers)
- Evidence: `bump_at_least` only ensures a number larger than what this node has seen (`manifest.rs:196-233`); `FileOffer` has no parent hash/version (`message.rs:229-237`); the only conflict detection is equal-version/different-content (`daemon.rs:957-977`, `exchange/mod.rs:769-770`). Problem log acknowledges (`:44`).
- Theme: ai-game. Severity: high. Design gap.
- Experience: A publishes `v2`; B, who received `v2`, edits the file and publishes `v3` — B's bump is `max(2+1, floor+1) = 3`, overwriting A's work on every node with no conflict, even if B edited the file *before* `v2` arrived (B's auto-pull already overwrote B's in-progress edit silently, see W-16). Classic binary-asset lost update; the design's Phase-3 locking (docs/versioning-design.md §4) does not exist.
- Redesign needs: offers carry `parent_hash`; receivers detect fork when `parent_hash ≠ local hash`; optional file locks announced via gossip; conflict copies written as `<name>.conflict-<peer>-v<N>` instead of rejecting.

### W-14 Two nodes that rolled back independently can mint the same new version with different content
- Evidence: the floor is node-local `state.json` (`version_state.rs:1-5`; `project.rs:444`); no global allocation or coordination.
- Theme: ai-game. Severity: medium. Design gap.
- Experience: after `restore --version 1` on nodes X and Y (both having seen v3), both publish v4 with different bytes; third nodes accept whichever arrives first and reject the other (W-03 path).
- Redesign needs: content-addressed revisions + parent links; version numbers become presentation only.

### W-15 `ConflictManager` is keyed by `FileId` only and accepts unauthenticated `ConflictAlert`s with empty project
- Evidence: `conflicts: DashMap<FileId, ConflictInfo>` (`conflict/mod.rs:87`); `record_version_conflict` returns the existing entry regardless of project (`:113-115`); `handle_conflict_alert` creates `Active` entries with `project_id: String::new()` and `file_path: String::new()` (`:325-353`); `ConflictAlert` has no signer-bound field (`message.rs:85-86`); `cleanup_resolved` never removes Active entries and there is no TTL (`:276-279`).
- Theme: security. Severity: medium. Exploitable.
- Experience: any peer floods `ConflictAlert{file_id: "assets/hero.fbx"}` → every project with that path shows a phantom conflict forever; later genuine equal-version conflicts in another project are swallowed (`record_version_conflict` returns the stale entry and emits no event). Memory grows without bound.
- Redesign needs: key by `(project_id, file_id)`, bind alerts to a signer and to a project topic, cap and expire entries, require alerts to reference signed offers.

### W-16 Auto-pull overwrites locally modified, unpublished files without warning (and ignores `sync_mode`)
- Evidence: auto-pull eligibility is only `sender ≠ self && !has_shared_file && project open` (`daemon.rs:978-980`); `handle_incoming_transfer` compares only against `shared_files` (the last published/received record), not the current working-tree bytes (`exchange/mod.rs:767-784`); `SyncMode::Manual/Selective` are stored (`project.rs:27-59`) but never consulted in the pull or receive path.
- Theme: ai-game. Severity: high. Design gap.
- Experience: an artist editing `level.unity` loses hours of work when a teammate publishes; "manual" sync mode does nothing.
- Redesign needs: dirty-check (hash the working copy vs manifest before replacing), divert to a conflict copy, honor sync mode and selective patterns, and provide an explicit "pull" command.

### W-17 Pinned restore over the network accepts arbitrary content for the requested version
- Evidence: `restore_file` network fallback pins with `expected_size_crc: None` (`checkout.rs:205, 224-246`); the pin bypasses stale/conflict checks (`exchange/mod.rs:772`) and forces the manifest entry (`project.rs:474, 492-506`).
- Theme: security. Severity: medium. Exploitable (given W-01/W-03).
- Experience: any peer answering the pinned `FileWant` replaces the file and the manifest entry with whatever it sends for that version.
- Redesign needs: restore must know the expected hash (from a history index or signed manifest) before accepting; without it, refuse network restore.

### W-18 Pin TTL (5 min) is shorter than large-asset transfer times, so checkout of older versions silently fails for big files
- Evidence: `PINNED_FETCH_TTL_MS` (`exchange/mod.rs:26`); expiry evaluated at receive completion (`:451-454, 757`); a post-expiry receive of an older version is then rejected as stale (`:767-784`), logged at `debug` only (`daemon.rs:807`); `CheckoutReport` reports "requested" with no completion tracking (`checkout.rs:27-34`).
- Theme: transfer / ai-game. Severity: medium. Design gap.
- Redesign needs: pins keyed to the in-flight transfer rather than wall clock; checkout should return a job id with completion events and retries.

### W-19 Absolute local file paths of the publisher leak into the advertised `RootCatalog`
- Evidence: `build_catalog_snapshot` sets `FileEntry.path = n.file_path.to_string_lossy()` where `file_path` is the canonical absolute path (`exchange/mod.rs:282-292`, filled from `ipc_server.rs:1412`); the block is `put` to the content store and its CID advertised to all peers (`:1399-1449`).
- Theme: security (privacy; org rule "no personal data"). Severity: medium. Exploitable via Bitswap fetch of the CID.
- Redesign needs: catalog entries carry the normalized relative key only.

### W-20 Manifest write amplification: every received or published file rewrites the entire manifest under the project mutex, inline in the receive path
- Evidence: `save` serializes the whole `BTreeMap` (`manifest.rs:165-179`) on each `bump_file_version`/`record_received_file` (`project.rs:458-459, 503-505`); `received_hook` is awaited inside `handle_incoming_transfer` (`exchange/mod.rs:824-835`); `VersionState::save` adds a second full rewrite (`project.rs:638-642`).
- Theme: transfer / ai-game. Severity: medium (O(n²) bytes for a checkout of n files). Design gap.
- Experience: a checkout that receives 50k assets rewrites a multi-MB JSON 100k times; receives serialize behind the lock.
- Redesign needs: append-only journal + periodic compaction, or a per-file entry store (sqlite/sled), and batch manifest commits per checkout.

### W-21 Every publish reads the file three times on the sender and the receiver reads it twice after writing
- Evidence: sender CRC (`ipc_server.rs:1325`), `hash_file` (`exchange/mod.rs:1139`), then the send loop reads again (`:652`); receiver writes (`transfer/mod.rs:196`), CRC re-read (`exchange/mod.rs:743`), history copy + hash of the copy (`history/store.rs:657, 675`).
- Theme: transfer. Severity: low-medium. Design gap.
- Redesign needs: single-pass hashing (compute BLAKE3 and CRC together while streaming), keep BLAKE3 in the manifest, hardlink/reflink into the history store where safe.

### W-22 `TransferLedger` is not project-scoped (problem-log "scope ... by project" only partially done)
- Evidence: keys are `(FileId, version)` (`synergos-net/src/chain/ledger.rs:56-57`); `Exchange` passes no project (`exchange/mod.rs:1207-1216, 1275-1283, 1068-1069`). `wants` entries are never removed (no GC API).
- Theme: transfer. Severity: low. Design gap.
- Experience: an offer for `a.bin v1` in project X "matches" a want for `a.bin v1` in project Y (`:1285-1296`), setting the wrong `peer_id`/`file_size` on the local transfer; `wants` grows forever.

### W-23 `CatalogUpdate` drift detection always fires and `update_count` is not a counter
- Evidence: `publish_updates` never updates `ctx.catalogs` (`exchange/mod.rs:1317-1454`; `CatalogManager` is only constructed at `daemon.rs:300-309`), so `local.catalog_crc`/`update_count` never match the remote (`daemon.rs:1008-1018`); `update_count = notifications.len()` (`exchange/mod.rs:1403, 1447`); `total_crc` is a CRC-of-CRCs (`:1393-1394`).
- Theme: transfer. Severity: low-medium. Design gap.
- Experience: every publish by any peer triggers a `CatalogSyncNeededEvent` and a Bitswap fetch on every other node (redundant traffic); the catalog can never be used to compute a real diff.

### W-24 Broadcast-channel event loss has no recovery path
- Evidence: `CoreEventBus` uses `broadcast` with capacity 256 (`event_bus.rs:20, 57`); gossip subscriber and fan-out drop on `Lagged` with only a `warn` (`daemon.rs:898-900, 1048-1050`); progress channel uses `try_send` (`exchange/mod.rs:109`).
- Theme: transfer. Severity: medium. Design gap.
- Experience: under an offer burst (publish of 1000 files) the subscriber lags → missed `FileOffer`s → files never auto-pulled until the next publish of that path.
- Redesign needs: durable per-project "wanted set" reconciled from the manifest/catalog rather than fire-and-forget events.

### W-25 Security-relevant rejections are logged at `debug`
- Evidence: all `handle_incoming_transfer` errors (stale, conflict, pinned mismatch, path escape, destination changed) surface via `tracing::debug!` (`daemon.rs:806-808`); FileWant failures at `warn` (`exchange/mod.rs:955`) but the default filter is `synergos_core=info` (`main.rs:29`).
- Theme: security. Severity: medium. Design gap.
- Redesign needs: structured security events (`warn`/`error`) with peer id, project, path, reason; emit an `EventBus` event for GUI surfacing.

### W-26 No provenance / audit trail for who introduced which bytes
- Evidence: `ManifestEntry.publisher` is just the last sender's peer id string (`exchange/mod.rs:831`, `project.rs:473`); nothing is signed; history index stores `publisher` text only (docs/versioning-design.md §3.3).
- Theme: security / ai-game (training-data lineage). Severity: medium. Design gap.
- Redesign needs: publisher-signed revision records (path, version, blake3, parent, timestamp, identity) stored in the manifest and history index; verifiable by Curare submissions.

### W-27 Working-tree receive path trusts the `incoming` directory layout and `.synergos` but not `.git`/tool directories — see W-04; additionally no handling of deletions, renames, or permissions
- Evidence: manifest has no tombstones; `checkout` leaves `extra` files untouched (`checkout.rs:111-114`); `FileId == rel path` so a rename is add+delete with version reset to `max(1, floor+1)` (`manifest.rs:218-231`); received files use default permissions (`transfer/mod.rs:166`).
- Theme: ai-game. Severity: medium. Design gap.
- Experience: deleted assets resurrect on other nodes; renaming a 4 GB texture retransfers it; executable tool scripts lose the exec bit cross-platform.

### W-28 `projects.json` is written non-atomically and save errors are ignored
- Evidence: `tokio::fs::write` (`project.rs:329-331`); `let _ = self.save_state()` (`:751, 779, 829`).
- Theme: other. Severity: low.
- Experience: a crash during save loses the project list; the daemon restarts with no projects and stops serving `FileWant`s.

### W-29 Startup restore CRC-scans every file in every manifest before the daemon serves anything
- Evidence: `restore_shared_files_from_manifests` is awaited in `Daemon::new` (`daemon.rs:265`) and reads every file (`restore.rs:44`).
- Theme: transfer / ai-game. Severity: medium. Design gap.
- Experience: a 500 GB asset project delays daemon start by the time to read 500 GB; IPC and QUIC are not up until then.
- Redesign needs: mtime/size short-circuit cache (or BLAKE3 in manifest + lazy verification on first want).

### W-30 `.synergos/incoming/*.part` leftovers from crashes are never swept
- Evidence: no cleanup in `restore.rs`, `daemon.rs` cleanup task (`:574-646`), or `ProjectManifest::incoming_dir` (`manifest.rs:89-112`).
- Theme: transfer. Severity: low.

### W-31 `max_peers` is never enforced
- Evidence: `ProjectError::MaxPeersReached` marked "現状未使用" (`project.rs:123-125`); `connected_peer_ids` only read in `get_project`/`list_projects`.
- Theme: other. Severity: low. Design gap.

### W-32 Equal-version conflict on the `TXFR` path is not recorded in `ConflictManager`
- Evidence: `handle_incoming_transfer` returns an error (`exchange/mod.rs:772-783`) without calling `record_version_conflict`; only the gossip path records (`daemon.rs:961-977`).
- Theme: ai-game. Severity: low-medium. Design gap.
- Experience: a conflicting push that arrives without a prior `FileOffer` (direct push, history node) is silently dropped.

### W-33 `resolve_conflict` has no effect on files or peers
- Evidence: flips state only (`conflict/mod.rs:231-251`); `IpcCommand::ConflictResolve` just calls it (`ipc_server.rs:2001-2026`); `AcceptRemote` does not fetch; `KeepLocal` does not republish; no `ConflictAlert` is ever published by core (grep: only handled at `daemon.rs:1000`); `detect_conflict` and `drain_pending_notifications` have no callers in `synergos-core/src` (grep).
- Theme: ai-game. Severity: medium. Design gap vs DESIGN §19.

### W-34 Synchronous filesystem calls inside async paths
- Evidence: `safe_join_under_root` uses `std::fs::canonicalize`/`symlink_metadata` per component (`manifest.rs:330-346`), called from the resolver on every receive (twice, `exchange/mod.rs:707, 790`) and from `metadata_file_path` on every manifest/state save (`manifest.rs:371-393`).
- Theme: transfer (perf). Severity: low.

### W-35 `shared_files` / `transfers` / `pinned_fetches` / `ledger.wants` lack bounds; `pinned_fetches` expire only when looked up
- Evidence: `exchange/mod.rs:341, 359, 369, 441-456`; `ledger.rs:56-57`. GC only for finished transfers (`:536-546`).
- Theme: security (DoS) / transfer. Severity: low-medium.

### W-36 Gossip fan-out falls back to every QUIC connection when the mesh is empty
- Evidence: `daemon.rs:864-871`.
- Theme: security (information leak of project ids, paths, versions to non-members). Severity: low.

### W-37 `TransferCompletedEvent.file_path` is always empty; `TransferCompletedEvent` carries no project id
- Evidence: `exchange/mod.rs:1061-1065`; `event_bus.rs:123-127`.
- Theme: other. Severity: low.

### W-38 Clock-skew underflow in notification cleanup
- Evidence: `now - n.created_at` on `u64` (`conflict/mod.rs:359`) panics in debug builds if the clock moves backwards.
- Theme: other. Severity: low.

### W-39 `file_paths` registry is never cleared on `close_project`
- Evidence: `project.rs:755-784` removes manifests/version_states/locks but not `file_paths` (`:204`).
- Theme: other. Severity: low (memory, stale mapping if reopened with another root; result still contained by `safe_join_under_root`).

### W-40 No `project status` command although documented
- Evidence: docs/versioning-design.md §3.4 lists `synergos project status`; `cli.rs:62-149` has no such subcommand; grep for `ProjectStatus` in `synergos-core/src` finds nothing.
- Theme: ai-game (workflow). Severity: low. Design gap.

### W-41 Hooks: repository hooks are opt-in, but daemon hooks run `sh -c` / `cmd /C` with the command string from config
- Evidence: `hooks/runner.rs:440-456`; `config.rs:99-108`. Not exploitable remotely in this map's code path (hooks come from local config or opt-in repo file), noted because `post-receive` is triggered by remote pushes (`exchange/mod.rs:858-865`) and any hook environment leak would be remote-triggerable.
- Theme: security. Severity: low. Design note for the hooks reviewer.

### W-42 `redact_path` is used for publish errors, but receive-side error strings embed `file_id` and raw reasons; `tracing` at `info` logs full file names and peer ids
- Evidence: `exchange/mod.rs:708, 791, 948-953, 1177-1182`.
- Theme: security (log hygiene). Severity: low.

---

## 5. Performance characteristics

- **Transfer framing**: 64 KiB chunks (`transfer/mod.rs:25`), each a msgpack frame `[u32 len][index u64][hash 32B][bin data]`, ≈ 50 B overhead per 64 KiB (<0.1 %). Sender allocates `buf[..filled].to_vec()` per chunk (`:97`) → one extra 64 KiB copy per chunk. Receiver `write_all` per chunk to an unbuffered `tokio::fs::File` (`:166, 196`), `flush` at the end (`:232`); **no `sync_all` before the atomic rename** (`exchange/mod.rs:798`) — durability after rename is left to the OS.
- **Hashing**: BLAKE3 single-threaded `Hasher::update` on 64 KiB reads (`transfer/mod.rs:289-313`); CRC32 via `crc32fast` on 256 KiB reads (`manifest.rs:436-450`). Sender per publish: CRC read + BLAKE3 read + send read = 3 full reads (W-21). Receiver per receive: 1 write + CRC read (+ history copy + BLAKE3 read if history enabled).
- **Round trips per auto-pull**: publisher gossip `FileOffer` (1 QUIC stream per mesh peer) → receiver gossip `FileWant` (1 stream per mesh peer) → publisher opens 1 `Data` bidi stream and pushes. Plus `CatalogUpdate` per publish batch and a Bitswap round trip for the catalog on every other node (W-23). Gossip fan-out = **one new QUIC stream per (message, peer)** (`daemon.rs:872-896`): a publish of n files to m peers opens 2·n·m + m streams before any data flows.
- **Concurrency**: every inbound stream, every FileWant and every auto-pull is its own `tokio::spawn` with no cap (`daemon.rs:800, 990`; `exchange/mod.rs:947, 977, 1112`). Only QUIC `max_concurrent_streams` (default 100 per connection) bounds it. Transfers are **one stream per file, no parallel chunks, no multi-source**.
- **Locking**: one `tokio::sync::Mutex` per project around manifest+state mutations (`project.rs:411-416`); the receive path awaits the manifest write inside the hook (`exchange/mod.rs:824-835`), so receives for the same project are serialized at the JSON write. `DashMap` everywhere else (lock-free reads, shard locks).
- **Disk I/O pattern**: whole-file sequential write to `<root>/.synergos/incoming/*.part` then same-volume rename (`exchange/mod.rs:733, 798`). Manifest and state are rewritten in full (`to_vec_pretty`) on every file event — O(n) per event, O(n²) for an n-file checkout (W-20). `projects.json` rewritten on every open/close/update (`project.rs:302-333`).
- **Startup**: O(total asset bytes) CRC scan (W-29); O(manifest entries) `VersionState::note` loop per open (`project.rs:714-724`).
- **Memory**: `MemoryContentStore` grows by one `RootCatalog` block per publish batch with no eviction (`exchange/mod.rs:1399-1432`; `content/store.rs` has no size limit — grep for `max|limit|evict` hits only `len()`); `transfers`, `ledger.wants`, `conflicts`, `hot_standby`, `pinned_fetches` grow as described in W-10/W-22/W-15/W-35.
- **Periodic work**: 60 s cleanup tick does conflict GC, invite GC, transfer GC, DHT GC, mesh session GC, RTT/bandwidth copy, and a `try_migrate_route` **await per connected peer** inside the tick (`daemon.rs:579-645`) — O(peers) network probes per minute on one task.
- **Progress reporting**: sender emits every 128 KiB via a 16-slot `try_send` channel (`exchange/mod.rs:80-115`); speed = cumulative bytes / elapsed (`:661-662`). Receiver: none.
- **Event bus**: broadcast capacity 256 per type (`event_bus.rs:20`); `emit` clones the event per subscriber.
- **Checkout**: one CRC pass over every manifest file (`checkout.rs:86`) plus one `FileWant` per differing file (no batching).

---

## 6. Config knobs & defaults

From `synergos-net/src/config.rs` (read for defaults) and this crate:

| Knob | Default | Secure/efficient by default? | Where used |
|---|---|---|---|
| `quic.listen_addr` | `None` → `[::]:0` (kernel port, all interfaces) | Listens on all interfaces by default; no bind restriction | `daemon.rs:89-96` |
| `quic.max_concurrent_streams` | 100 | Only bound on inbound stream fan-out | `config.rs:707` |
| `quic.idle_timeout_ms` | 30000 | Only idle bound on receives | `config.rs:708` |
| `gossipsub.heartbeat_interval_ms` | 1000 (clamped 100..60000) | ok | `daemon.rs:1066` |
| `gossipsub.message_cache_size` | 1000 | Replay window; small | `config.rs:725` |
| `force_relay_only` / `auto_promote` | false / true | ok | `daemon.rs:106-128` |
| `bootstrap_urls` | `[]` | ok; validated, no redirects | `daemon.rs:431-489` |
| `peer_info_listen_addr` | `None` | ok (opt-in) | `daemon.rs:393-416` |
| `history.enabled` | false | ok | `daemon.rs:241-254` |
| `history.projects` | `["*"]` | all projects once enabled | `config.rs:275-277` |
| `history.root` | `.synergos/history` (relative) | `..` and symlink escapes rejected per docs §3.3 (store.rs not read) | `config.rs:278-281` |
| `history.max_versions_per_file` / `max_age_days` / `max_bytes` | 0 / 0 / 0 (unlimited) | **Not** bounded by default | `config.rs:282-290` |
| `history.rotation.enabled` | false; `interval_hours` default via `default_rotation_interval_hours` | ok | `daemon.rs:653-708` |
| `hooks.allow_project_hooks` | false | ok (repo hooks opt-in) | `config.rs:99-104` |
| `hooks.hooks` | `[]` | daemon-local shell commands | `config.rs:105-107` |
| `catalog.chunk_max_files` / `chain_max_depth` | 256 / 10 | n/a | `daemon.rs:305-306` |
| Project `sync_mode` | `Full` (unknown strings → `Full`) | **Ignored** by pull/receive (W-16) | `project.rs:49-59` |
| Project `max_peers` | 0 (unlimited) | Not enforced (W-31) | `project.rs:67-68` |
| Invite `expires_in_secs` | `None` (never expires) | Default is non-expiring | `project.rs:862-868` |
| `PINNED_FETCH_TTL_MS` | 5 min (const) | Too short for big assets (W-18) | `exchange/mod.rs:26` |
| `CHANNEL_CAPACITY` | 256 (const) | Lossy under bursts (W-24) | `event_bus.rs:20` |
| `MAX_EXTENSION_PAYLOAD` | 1 MiB (const) | ok | `daemon.rs:777` |
| `RUST_LOG` / `SYNERGOS_LOG_FORMAT` | `synergos_core=info` / text | security rejections hidden at `debug` (W-25) | `main.rs:27-49` |
| Config file | `--config <path>`; missing file → defaults with a warn | A typo'd path silently runs with defaults | `daemon.rs:557-571` |

There is **no** knob for: max asset size, per-peer/per-project quotas, inbound transfer concurrency, allowed path patterns, project ACL / membership, required protocol version, or signature requirement (the `require_signature` flag in `GossipNode` is never set by the daemon, but the wire path verifies unconditionally).

---

## 7. Test coverage observed

**Tested**
- Real two-node QUIC `TXFR` round trip with `execute_send` ↔ `handle_incoming_transfer` (`tests/transfer_e2e.rs`).
- `FileWant` → `share_and_send` push; no-op when nothing is shared; remote offer registration; project-scoped `shared_files` (`tests/auto_transfer_e2e.rs`).
- Full gossip (signed, over QUIC) + ledger + push chain, manual `fetch_file` (`tests/two_node_full_e2e.rs`) and auto-pull without IPC (`tests/auto_pull_e2e.rs`).
- Versioned republish: v1 → v2 re-receive, unchanged content keeps version, both manifests record version/CRC, `.synergos/incoming` is inside the project and left empty, restart restore of `shared_files` from manifest (`tests/versioned_republish_e2e.rs`). Note this test re-implements the daemon's subscriber/fan-out inline rather than exercising `daemon.rs`.
- Checkout decision logic without network: requests only differing files, `extra` reporting, `--manifest` replacement, project-id mismatch rejection; version high-water survives manifest rollback + restart (`tests/checkout.rs`).
- Project persistence round trip and update persistence (`tests/project_persistence.rs`).
- Project open/close/duplicate/invalid id, concurrent manifest writers preserve both entries, equal-version conflict record, transfer state transitions (Queued→Cancelled, pause/resume flags, GC), presence idempotence (`tests/core_services.rs`).
- Unit: path normalization and rejection, symlink containment, bump idempotence and floor, manifest rejects unsafe paths, `record_received` forward-only/force, save/load round trip, failed replace preserves destination (`manifest.rs:452-625`); version state forward-only (`version_state.rs:110-123`); pin generation semantics (`exchange/mod.rs:1533-1569`); event bus delivery/isolation/late subscriber (`event_bus.rs:226-328`); transfer round trip and tampered chunk rejection (`transfer/mod.rs:351-436`).

**Not tested (gaps relevant to this map)**
- Any negative inbound transfer case end-to-end: stale version rejection, equal-version conflict rejection, pinned size/CRC mismatch, `u64::MAX` version, destination-changed, symlinked destination over the wire, unknown project id, oversized `total_size`, truncated stream leaving `.part`.
- Pin TTL expiry behaviour; checkout where the file actually arrives (network restore); `restore_file` local-history path; `RestoreOutcome` variants.
- `daemon.rs` itself (no test constructs `Daemon`; the e2e tests copy its subscriber logic). `dispatch_peer_streams` magic routing, extension-stream cap, gossip subscriber conflict branch, `CatalogUpdate` drift path, periodic cleanup, shutdown cancellation.
- Concurrency: concurrent receives of the same path, receive racing a local publish, two publishers minting the same version.
- Cross-project ledger collision; `has_shared_file` with version 0; `FileWant` version 0 behaviour.
- `ConflictManager::detect_conflict`, `handle_conflict_alert`, hot-standby queue/drain, `resolve_conflict` on an active conflict, `cleanup_expired_notifications` with entries.
- `ProjectManager::join_project` / `create_invite` expiry and one-shot semantics; `gc_expired_invites`; `update_project` sync-mode parsing.
- `restore_shared_files_from_manifests` skip branches (missing file, CRC mismatch, symlinked path).
- `publish_updates` with `content_store` attached (catalog snapshot content, CID advertisement); the absolute-path leak.
- CLI argument handling (`cli.rs` has no tests; it is a thin IPC client).
- Windows `replace_file_windows` and `normalize_canonical_root` (cfg-gated, no Windows test observed).
- Performance/regression tests for large files (>1 GiB), many files (>10k), or slow links.

---

## 8. Documented-vs-implemented deltas

| Doc claim | Reality | Evidence |
|---|---|---|
| Problem log fix #7: "Reject peers using the pre-versioned transfer protocol" (`2026-08-16-versioned-transfer-safety.md:36`) | Not implemented: version-less headers are accepted and coerced to v1; no protocol version negotiation | `transfer/mod.rs:44-45`; `exchange/mod.rs:750`; `quic/hello.rs` |
| Problem log fix "Scope shared-file and gossip deduplication state by project" (`:33`) | Done for `shared_files` and gossip message id; `TransferLedger` still global by `(FileId, version)` | `exchange/mod.rs:359`; `node.rs:183-186`; `ledger.rs:56-57` |
| Problem log fix "Report equal-version/different-content offers as conflicts without overwriting local data" (`:34`) | Done on the gossip path; the `TXFR` path rejects but does not report | `daemon.rs:957-977`; `exchange/mod.rs:769-783` |
| Problem log fixes #1–#3, #5 (atomic replace, symlink revalidation, serialized manifest mutation, bootstrap URL validation/no redirects) | Present | `manifest.rs:355-369, 328-351`; `project.rs:411-416`; `peer_bootstrap.rs:85-98, 193-198` |
| Problem log "Verification": "Static regression tests were added ... No tests were run during review" (`:40`) | Regression tests exist for symlink containment, project-scoped records, URL validation; none for versioned framing rejection or concurrent edits | see §7 |
| docs/versioning-design.md §2: "受信側: 受信完了時に version を記録。手元の版 ≥ 提示版 なら auto-pull しない" | Implemented | `daemon.rs:979`; `exchange/mod.rs:510-515` |
| §2: "転送は ファイル全体 (64 KiB フレーム + blake3 検証)" | Implemented | `transfer/mod.rs` |
| §2 `.gitignore` guidance for `state.json`, `incoming/`, `history/` | Nothing enforces or generates it; `state.json` committed by mistake would alter other nodes' floors | — |
| §3.4 table: `synergos project status <id>` ("git status 相当") | Not implemented | `cli.rs:62-149` |
| §3.4: "通常ノードは手元 manifest の version と一致する FileWant にだけ応答" | Also answers `version == 0` ("any latest") | `exchange/mod.rs:925-927` |
| §3.4: "要求側は最初に返ってきた Offer から受信" | There is no offer selection; the receiver accepts the first `TXFR` push from anyone | `exchange/mod.rs:689-801` |
| §3.4: `restore ... (manifest も N に書き戻す)` | Local-history path forces the manifest; network path forces only on receipt (pinned) | `checkout.rs:146-156, 191-201`; `exchange/mod.rs:832` |
| §3.4: pin "5 分で失効し、その後に publish / 受信した版があれば直ちに無効化" | Implemented | `exchange/mod.rs:26, 451-454, 818-823, 1368` |
| §4 Phase 1: equal-version different-content → ConflictManager + keep local | Implemented (gossip path) | `daemon.rs:957-977` |
| §4 Phase 3: conflict copies `<name>.conflict-<peer>-v<N>`, parent version in offers, `project lock` | Not implemented | — |
| §5-3: "転送要否は version・size・CRC で決める (時刻は使わない)" | Implemented; but CRC32 is the only content identity (W-06) | `manifest.rs:206` |
| DESIGN.md §19 (headings "16.x"): `ConflictInfo` with `ChainBlock` ancestor/heads, `notify_conflict` via gossip, `flush_hot_standby`, `update_during_conflict`, `notify_queue` with retries, resolution flow ④–⑤ (apply resolution, add resolution block, gossip resolution) | `ConflictInfo` holds only hashes/versions; `detect_conflict` exists but has no caller; core never publishes `ConflictAlert`; hot-standby is queued but never drained/sent; `resolve_conflict` is state-only; `notify_queue` absent | `conflict/mod.rs`; grep results |
| `exchange/mod.rs:1-6` module doc: "優先度キュー、帯域制御、チャンク分割・再組立" | No queue, no bandwidth control; chunking is in `synergos-net` | `exchange/mod.rs:1104-1116` |
| `exchange/mod.rs:896-897`: "既に該当 peer へ同じバージョンを送ったことがあれば ledger の mark_fulfilled でスキップされる想定" | Not implemented; sends unconditionally | `exchange/mod.rs:898-958` |
| `exchange/mod.rs:1007-1011` doc: `handle_file_offer` "自分の保留 Want とマッチしたらログを出す" | Matches are logged only; matched state is never set (`LedgerEntryState::Matched` unused) | `exchange/mod.rs:1032-1043` |
| `daemon.rs:113` comment: relay endpoint presence "現状は判定不能、後続 PR で正確化" | Still hard-coded `false` | `daemon.rs:110-115` |
| `ipc_server.rs:1207-1209`: IPC `TransferRequest` always version 0; "呼出側が具体バージョンを指定したい場合は IpcCommand を拡張する" | Still unextended | — |

---

## 9. Open questions for designers

1. **Who is allowed to push into a project?** Today the answer is "anyone with a QUIC route and the project id". Should membership be a Cernere-issued credential presented in `HLO1`, a per-project key, or a signed membership list gossiped by an owner? How does Curare's access policy map onto Synergos projects?
2. **What is the unit of identity for a revision?** Bare `u64` versions minted locally cannot be authenticated or reconciled. Is the intended model content-addressed (BLAKE3) revisions with parent links, with `version` as a display counter? If so, does the git-committed `manifest.json` carry hashes and signatures?
3. **Receiver trust model for pushes.** Should receivers only accept bytes whose hash matches a signed offer they have already seen (pull-verified), rather than accepting unsolicited `TXFR` pushes keyed only by `(project, path, version)`?
4. **Working-tree safety.** Which paths are syncable? Should the receiver refuse anything not declared in a signed manifest, and should `.git/`, `.cargo/`, `.vscode/`, `.idea/`, `*.sh`, etc. be denied by default?
5. **Dirty working copies.** Is overwriting an unpublished local edit ever acceptable? If not, how are conflict copies named, surfaced in the GUI/CLI, and resolved — and does `sync_mode` become a real contract?
6. **Locks for binary assets.** docs §4 defers `project lock` to Phase 3; given W-13, is optimistic versioning viable for game teams at all, or should advisory locks ship with the first conflict UI?
7. **Transfer scheduling.** Is the single-stream, push-on-want model intended to scale to thousands-of-files checkouts and multi-GB assets? Where should resume, ranged chunks, multi-source and bandwidth caps live (Exchange vs synergos-net)?
8. **Manifest storage.** JSON-rewrite-per-event is O(n²) for large projects. Is a journal/sqlite acceptable for a git-committed artifact, or should the committed manifest be a periodic export of a local store?
9. **Observability contract.** What security events must be `warn`-level and surfaced to the GUI/admin (rejected pushes, conflicts, version anomalies, peers without membership)?
10. **History node trust.** History nodes answer any version to anyone; should restore require the hash from a tag/manifest, and should history nodes verify publisher signatures before archiving?
11. **Protocol versioning.** Where is the protocol version declared and how are incompatible peers rejected (the problem log says this was fixed)?
12. **Catalog/Bitswap path.** `CatalogUpdate`/`RootCatalog` is advertised on every publish but never used for diffs and leaks absolute paths. Keep, fix, or remove?
13. **Privacy.** Peer ids, absolute paths and file names are logged at `info` and gossiped to fallback peers. What is the org rule for log/gossip hygiene given "no personal data in service DBs"?
