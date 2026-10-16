# Synergos subsystem map: content layer (blocks, bitswap, chunker, store, session, transfer, chain/ledger, catalog, catalog_sync)

Scope: `synergos-net/src/content/*`, `synergos-net/src/transfer/mod.rs`, `synergos-net/src/chain/*`, `synergos-net/src/catalog/*`, `synergos-core/src/catalog_sync.rs`, plus the wiring sites in `synergos-core/src/daemon.rs`, `synergos-core/src/exchange/mod.rs`, `synergos-core/src/ipc_server.rs`, `synergos-core/src/conflict/mod.rs` that decide what is live vs dormant. All line numbers are from the files as read on 2026-10-15. Paths are relative to `/home/user/Synergos/`.

## 0. Live vs dormant verdict (asked for explicitly)

| Component | Status in the daemon | Evidence |
|---|---|---|
| `transfer` (`TXFR` framing, `send_over_quic` / `receive_over_quic_with_header`, `hash_file`) | **LIVE: this is the only path real file bytes take** | `synergos-core/src/daemon.rs:805-808` dispatches `TRANSFER_STREAM_MAGIC` to `Exchange::handle_incoming_transfer`; `synergos-core/src/exchange/mod.rs:1139-1161` (`hash_file` → `TransferHeader` → `execute_send`), `:670` (`send_over_quic`), `:703`, `:734` (`peek_transfer_header`, `receive_over_quic_with_header`). `hash_file` is also reused by the history store (`synergos-core/src/history/store.rs:675,711`, `history/rotation/mod.rs:487`). |
| `content::bitswap` server (`handle_bitswap_stream`) | **LIVE, but only serves the shared `MemoryContentStore`, which only ever holds `RootCatalog` snapshot blocks** | `daemon.rs:814-817` dispatches `BITSWAP_STREAM_MAGIC`; the only `put` into that store in production code is the catalog snapshot at `exchange/mod.rs:1408-1411` (and the client-side `put` of fetched blocks in `session.rs:186`). |
| `content::session::BitswapSession::fetch_blocks` | **LIVE, single-CID use only** | `synergos-core/src/catalog_sync.rs:155-159` fetches exactly one CID (`catalog_cid`). |
| `content::chunker` (`add_file`, `get_file`, `ChunkDag`) and `BitswapSession::fetch_dag` / `have_query` | **DORMANT** (no callers outside `synergos-net` tests) | grep over all crates: `add_file(`/`get_file(`/`fetch_dag`/`ChunkerOptions` appear only in `synergos-net/src/content/*`, `synergos-net/tests/bitswap_*.rs`. `exchange/mod.rs:302-304` says the child chunk blocks are "future extension". |
| `content::store::FileContentStore` | **DORMANT** (only unit-tested; daemon uses `MemoryContentStore` concretely) | `daemon.rs:237`, `ipc_server.rs:54`, `catalog_sync.rs:36` all name `Arc<MemoryContentStore>`. |
| `chain::FileChain` / `ChainBlock` (signed per-file update chain) | **DORMANT** | `CatalogManager::record_update` has no caller in `synergos-core` (only `synergos-net/tests/net_additional.rs:160`); `ConflictManager::detect_conflict` (`conflict/mod.rs:149`) has no caller; `ChainBlock` has no `Serialize` derive (`file_chain.rs:8-9`) so it cannot be transmitted at all. |
| `chain::TransferLedger` (Want/Offer) | **LIVE** | `exchange/mod.rs:343,387` owns one; `register_offer` at `:1032,1216,1362`, `register_want` at `:1282`, `mark_fulfilled` at `:1068`, `cancel_want` at `:1487`. |
| `catalog::CatalogManager` | **INSTANTIATED BUT EMPTY** | Created per project at `daemon.rs:301-308` and `ipc_server.rs:739-756`; nothing in core ever calls `add_file`/`record_update`/`diff_catalog`/`set_file_state`, so `root_catalog()` is always `{update_count: 0, catalog_crc: 0, chunks: []}`. It is read only as the comparator in the drift check at `daemon.rs:1008-1018`. |
| `catalog_sync::CatalogSyncService` | **LIVE but terminal** | Spawned at `daemon.rs:383-388`; it fetches and decodes a `RootCatalog` then discards it (`catalog_sync.rs:175-178`). `CatalogSyncCompletedEvent` has no non-test subscriber (grep: only `event_bus.rs`, `catalog_sync.rs`, tests). |

Inference: the IPFS-style pipeline in DESIGN 15.1 is scaffolding; the shipping transfer path is a single-stream, whole-file, 64 KiB-chunk `TXFR` protocol, and the content-addressed layer currently only carries small catalog metadata blocks.

## 1. Responsibilities & data flow

### 1.1 `content` (synergos-net/src/content)
- `block.rs`: `cid_for(bytes) -> Cid("blake3-<64 hex>")` (`block.rs:7-10`); `Block { cid, bytes }` with `verify()` recomputing the hash (`:19-28`).
- `store.rs`: `ContentStore` trait (`put/get/has`, `store.rs:17-22`); `MemoryContentStore` = `Arc<RwLock<HashMap<Cid, Vec<u8>>>>` (`:25-27`); `FileContentStore` = `<root>/<2-char prefix>/<rest>` files (`:66-85`).
- `chunker.rs`: `add_file` splits a file into fixed-size chunks, `put`s each as a `Block`, then `put`s a msgpack `ChunkDag { total_size, chunks: Vec<Cid> }` root (`chunker.rs:45-84`); `get_file` walks the root and concatenates (`:88-130`).
- `bitswap.rs`: wire protocol over one QUIC bidi stream: client sends `BSW1 | u32be len | msgpack(BitswapRequest)` and `finish()`; server replies `BSW1 | [u32be len | msgpack(BitswapResponse)]* | EOF` (`bitswap.rs:12-24`). Server entry `handle_bitswap_stream(store, send, recv)` (`:228-288`) is called after the daemon has consumed the 4-byte magic.
- `session.rs`: `BitswapSession { quic, peer, store, max_inflight_streams }` (`session.rs:42-51`); `have_query`, `fetch_blocks` (skips locally-held CIDs, batches of `SESSION_WANTLIST_CHUNK`=128, up to 4 concurrent streams, `put`s verified blocks), `fetch_dag` (root then children) (`:77-238`).
- Enters: QUIC streams opened by peers (server) / `CatalogSyncNeededEvent` (client). Leaves: blocks into `MemoryContentStore`; `CatalogSyncCompletedEvent`. Calls: `crate::quic::QuicManager::open_stream(peer, StreamType::Control)` (`session.rs:83-86,151-154`), `rmp_serde`, `blake3`.

### 1.2 `transfer` (synergos-net/src/transfer/mod.rs)
- Pure I/O protocol: `send_stream(reader, writer, header)` writes `FrameKind::Header`, N `FrameKind::Chunk`, `FrameKind::Footer`, each as `u32le len | msgpack` (`mod.rs:71-112, 324-331`). `receive_body` validates per-chunk hash, order, size, then footer hash vs header hash (`:152-240`). `hash_file` computes whole-file blake3 + size + chunk count (`:289-313`).
- Enters: `TransferHeader` prepared by `Exchange` (`exchange/mod.rs:1149-1157`); QUIC recv half after `TXFR` magic. Leaves: a file at `out_path` (the Exchange passes a `.part` temp path, `exchange/mod.rs:733`).

### 1.3 `chain` (synergos-net/src/chain)
- `file_chain.rs`: `ChainBlock { hash, prev_hash, version, author, author_public_key, timestamp, payload, signature }` with `compute_hash`, `signing_message`, `sign(identity)`, `verify()` (`file_chain.rs:9-129`); `ChainPayload::{TextDiff, BinaryCid, FullSnapshot}` (`:133-140`); `FileChain` = `VecDeque<ChainBlock>` capped at `max_depth`, `append` enforces `prev_hash == head` (`:143-228`).
- `ledger.rs`: `TransferLedger { wants: DashMap<(FileId,u64), Vec<WantEntry>>, offers: DashMap<(FileId,u64), OfferEntry> }` (`ledger.rs:60-63`) with `register_want`, `register_offer`, `mark_fulfilled`, `cancel_want`, `pending_want_count` (`:74-163`).
- Enters: `OfferEntry`/`WantEntry` from `Exchange` (local shares/fetches and remote gossip `FileOffer`). Leaves: `LedgerAction::{Queued, Duplicate, Match{sender,file_size}}`.

### 1.4 `catalog` (synergos-net/src/catalog)
- `types.rs`: `RootCatalog { project_id, update_count, chunks: Vec<ChunkIndex>, catalog_crc, last_updated }` (`types.rs:12-22`), `ChunkIndex { chunk_id, crc, content_hash (serde default), last_updated }` (`:29-40`), `Chunk { chunk_id, files: Vec<FileEntry>, max_files }` with `compute_crc` = crc32 over file CRCs (`:44-73`), `FileEntry { file_id, path, crc, content_hash (default), state, size }` (`:77-91`), `FileState`, `CatalogDiff`, `FileUpdateAction` (`:95-128`).
- `manager.rs`: `CatalogManager { root: RwLock<RootCatalog>, chunks: DashMap, chains: DashMap<FileId, FileChain>, file_to_chunk, chunk_max_files, chain_max_depth }` (`manager.rs:10-21`); `add_file`, `record_update(file_id, ChainBlock)`, `set_file_state`, `diff_catalog(&RootCatalog)`, `root_catalog()`, `get_chain` (`:42-174`).

### 1.5 `catalog_sync` (synergos-core/src/catalog_sync.rs)
- Subscribes to `CatalogSyncNeededEvent` synchronously before spawning (`catalog_sync.rs:42`), spawns one task per event (`:53-55`), `process_sync_event` → `fetch_root_catalog` (`:118-125`) → `BitswapSession::fetch_blocks([catalog_cid])` (`:155-159`) → decode as `RootCatalog` (`:175-176`) → `CatalogSyncCompletedEvent { fetched_blocks, error }` (`:181-194`).
- Upstream producer: `daemon.rs:1003-1047` compares gossip `CatalogUpdate { root_crc, update_count }` with the (always empty) local `CatalogManager` and emits the event. Publisher side: `exchange/mod.rs:1397-1441` builds a `RootCatalog` snapshot with `build_catalog_snapshot` (`:272-317`), `put`s it as a `Block`, and gossips `CatalogUpdate { catalog_cid, publisher }`.

### 1.6 End-to-end live flows
1. Publish: IPC `PublishUpdate` → `Exchange::publish_updates` → ledger Offer + `shared_files` + gossip `FileOffer` per file (`exchange/mod.rs:1346-1395`) → one `RootCatalog` block into `MemoryContentStore` → gossip `CatalogUpdate` (`:1397-1441`).
2. Pull: receiver's daemon sees `FileOffer` → `handle_file_offer` (ledger) → auto `fetch_file` (ledger Want + gossip `FileWant`) (`daemon.rs:936-995`) → publisher's `handle_file_want` → `share_and_send` → `hash_file` → `execute_send` → `TXFR` stream → receiver `handle_incoming_transfer` → `receive_over_quic_with_header` into `<project>/.synergos/incoming/incoming-<uuid>.part` → CRC + pin/stale checks → atomic replace (`exchange/mod.rs:689-800`).
3. Catalog drift: receiver sees `CatalogUpdate` → always "drift" (local root is 0/0) → `CatalogSyncNeededEvent` → `BitswapSession` fetch of the snapshot → decode → event; nothing else happens.

## 2. Key types / APIs / wire formats

### 2.1 CID
- `types.rs:144`: `pub struct Cid(pub String);` no format validation anywhere. Live values are `"blake3-" + 64 hex` (71 bytes) produced by `cid_for` (`block.rs:7-10`). Not a multiformats CID (`content/mod.rs:11-13` admits this). Tests use arbitrary strings such as `"blake3-unknown"` (`tests/bitswap_e2e.rs:125`).

### 2.2 Bitswap (`BSW1`)
- Magic `BITSWAP_STREAM_MAGIC = b"BSW1"` (`bitswap.rs:36`). Frame = `u32 big-endian length` + body (`:84-91`). `MAX_BITSWAP_FRAME = 16 MiB` enforced on read (`:117-121`). `MAX_WANTLIST_LEN = 1024` (`:39`), checked client-side (`:158-163`) and server-side after decode (`:260-265`).
- Request enum (msgpack of a serde enum, external tagging): `Want{cid}`, `WantList{cids, want_have (serde default false)}`, `Cancel{cids}` (`:41-56`). Response: `Block{cid, bytes (serde_bytes)}`, `NotFound{cid}`, `Have{cid}`, `DontHave{cid}` (`:58-76`).
- Semantics: one request per stream; the client `finish()`es its send half immediately (`:183-184`); server writes magic once then one frame per CID in request order and `finish()`es (`:249-287`). `Cancel` → server writes nothing and finishes (`:241-246`). Single-CID non-have requests are encoded as v1 `Want` for backward compatibility (`:169-179`).
- Client helpers: `request_block` (`:133-142`), `request_many` (`:149-204`), `send_cancel` (`:208-221`).

### 2.3 Chunker DAG
- `ChunkDag { total_size: u64, chunks: Vec<Cid> }` msgpack-encoded as the root block (`chunker.rs:22-27, 77-83`). Flat, one level; no links beyond the list. Default `chunk_size = 256 KiB` (`:36-41`). Last chunk may be short; empty file produces a root with zero chunks.

### 2.4 Transfer (`TXFR`)
- Magic `TRANSFER_STREAM_MAGIC = b"TXFR"` (`transfer/mod.rs:245`). Frame = `u32 little-endian length` + msgpack(`FrameKind::{Header,Chunk,Footer}`) (`:317-349`). `MAX_FRAME = CHUNK_SIZE + 1024 = 66560` bytes (`:339`). `CHUNK_SIZE = 64 KiB` (`:25`).
- `TransferHeader { transfer_id, project_id, file_id: String, total_size, chunk_count, total_hash: Blake3Hash, version (serde default 0) }` (`:29-46`). `ChunkFrame { index, hash, data (serde_bytes) }` (`:50-58`). `TransferFooter { total_hash }` (`:62-65`).
- `Blake3Hash([u8;32])` (`types.rs:134`), serialized by serde as a 32-element array (not `serde_bytes`), so each hash costs ~34-66 bytes on the wire depending on msgpack int packing (inference from derive; no `serde_bytes` attribute at `types.rs:133-134`).

### 2.5 Chain
- `ChainBlock` hash preimage: `prev_hash? | version le | author str | author_public_key | timestamp le | payload tag+fields` (`file_chain.rs:32-69`). Signing message: `hash | prev flag+hash | version | pubkey | timestamp` (`:74-87`) — ed25519 via `identity::sign/verify` (`identity.rs:154-156, 346-354`). `author` must equal `peer_id_from_public_bytes(pubkey)` (`:103-108`), which is blake3(pubkey) truncated to `PEER_ID_PREFIX_LEN = 20` bytes hex (`identity.rs:37, 339-343`).
- No serde derives on `ChainBlock`/`ChainPayload`/`FileChain` (`file_chain.rs:8, 132, 142`).

### 2.6 Ledger
- `WantEntry { requester, file_id, version, requested_at, state }`, `OfferEntry { sender, file_id, version, file_size, crc: u32, offered_at, state }`, `LedgerEntryState::{Pending, Matched, Fulfilled, Cancelled}` (`ledger.rs:12-43`).

### 2.7 Catalog
- Serialized with `rmp_serde` as the bitswap root block (`exchange/mod.rs:1406-1408`; decode at `catalog_sync.rs:175-176`). `content_hash` fields are `#[serde(default)]` for old peers (`types.rs:36-37, 85-86`).
- Gossip `CatalogUpdate { project_id, root_crc, update_count, updated_chunks: Vec<ChunkId>, catalog_cid: Option<Cid> (default), publisher: PeerId (default) }` (`gossip/message.rs:203-217`).

### 2.8 Events
- `CatalogSyncNeededEvent { project_id, remote_root_crc, remote_update_count, changed_chunks: Vec<String>, catalog_cid: Option<Cid>, publisher: Option<String> }` (`event_bus.rs:154-165`); `CatalogSyncCompletedEvent { project_id, remote_root_crc, remote_update_count, fetched_blocks: u32, error: Option<String> }` (`:176-184`).

## 3. Security controls PRESENT

Authn / authz
- C1. Bitswap and TXFR streams are only reachable over an accepted QUIC connection whose `peer_id` was established by the QUIC layer (`daemon.rs:726-747` passes `acc.peer_id` as `sender`). Inference: identity comes from the certificate/hello handshake in the transport subsystem; not re-verified here.
- C2. Gossip envelopes are signed and the `publisher` field of `CatalogUpdate` must equal the signer's derived peer id when non-empty (`gossip/message.rs:77-84`); `FileOffer.sender` / `FileWant.requester` likewise (`:63-76`). So the `publisher` that `catalog_sync` dials cannot be spoofed to a third party.
- C3. `handle_file_offer` ignores offers from self (`exchange/mod.rs:1020-1022`); daemon rejects `FileOffer` with `version == 0 || u64::MAX` (`daemon.rs:944-947`).

Crypto / integrity
- C4. Every `Block` is verified against its CID on `put` for both stores (`store.rs:47-49, 91-93`), on `get` from disk (`store.rs:107-115`), on bitswap receipt (`session.rs:180-185`) and on reassembly (`chunker.rs:113-117`). Hash is blake3-256.
- C5. Bitswap client checks the returned CID equals the requested CID per frame (`session.rs:170-175, 190-195`) and response count equals request count (`:88-94, 156-162`).
- C6. TXFR: per-chunk blake3 (`transfer/mod.rs:182-188`), strict in-order index (`:189-194`), non-empty and `<= CHUNK_SIZE` chunk (`:175-181`), running total vs header on every chunk with `checked_add` (`:198-206`), footer hash == running hash == header hash (`:209-219`), chunk count and byte count equality (`:220-231`), duplicate header rejected (`:235-237`), header `chunk_count` must equal `ceil(total_size / CHUNK_SIZE)` (`:132-142`).
- C7. `ChainBlock::sign/verify`: ed25519 signature over hash+prev+version+pubkey+timestamp; `author` bound to pubkey (`file_chain.rs:91-123`). `FileChain::append` rejects non-linear `prev_hash` with `SynergosNetError::ChainFork` (`:164-171`, `error.rs:19-23`).
- C8. `catalog_sync` verifies the fetched block decodes as `RootCatalog` before reporting success (`catalog_sync.rs:175-176`).

Validation / limits
- C9. Bitswap frame cap 16 MiB (`bitswap.rs:117-121`); wantlist cap 1024 server and client (`:158-163, 260-265`); truncated length prefix is an error, clean EOF is `None` (`:94-127`).
- C10. TXFR frame cap `CHUNK_SIZE + 1024` (`transfer/mod.rs:339-344`).
- C11. Daemon extension streams capped at 1 MiB (`daemon.rs:775-777, 821`).
- C12. `catalog_sync` skips events with no `catalog_cid` or empty `publisher` (`catalog_sync.rs:77-109`) and short-circuits if the CID is already local (`:151-153`).
- C13. `BitswapSession::fetch_blocks` filters out CIDs already in the local store before any network I/O (`session.rs:106-118`).

Atomicity / locking
- C14. `MemoryContentStore` uses `tokio::sync::RwLock` (`store.rs:25-27`); `CatalogManager.root` is a `tokio::sync::RwLock`, other maps are `DashMap` (`manager.rs:10-21`); `TransferLedger` is `DashMap` (`ledger.rs:60-63`).
- C15. The TXFR *caller* (Exchange) receives into a `.part` temp file inside `<project>/.synergos/incoming`, validates the directory shape, removes the temp on any error and does an atomic replace at the end (`exchange/mod.rs:698-741, 785-800`). (The `transfer` module itself writes straight to `out_path`; see W18/W29.)
- C16. `catalog_sync` subscribes before `tokio::spawn` to avoid losing the first event (`catalog_sync.rs:39-42`).

Logging
- C17. Peer ids are truncated in logs via `PeerId::short()` (`catalog_sync.rs:114`, `daemon.rs:783,790`); project ids and CIDs are logged in full (`catalog_sync.rs:111-131`, `daemon.rs:1020-1024`).
- C18. The S20 path-redaction helper exists (`types.rs:231-239`) — but is not used by the catalog snapshot (see W45).

## 4. Weaknesses, TODOs, missing controls FOUND

Tags: [security] [transfer] [ai-game]. "Exploitable" means reachable by a peer that has completed the QUIC handshake and is on a project gossip topic; "design gap" means not reachable today because the code is dormant or an upstream check prevents it.

### Bitswap / content store
- W1 [security] **No authorization on the bitswap server; the content store is global, not project-scoped.** `daemon.rs:814-817` hands every `BSW1` stream from any connected peer to `handle_bitswap_stream(content_store, ...)`, which answers any CID (`bitswap.rs:267-283`). The store is a single `MemoryContentStore` shared across all projects (`daemon.rs:237`, `ipc_server.rs:54`). Exploitable: any connected peer that learns or guesses a CID (CIDs are broadcast in gossip `CatalogUpdate`, `exchange/mod.rs:1438-1441`) can read blocks for projects it is not a member of. Redesign: per-project stores or an ACL check `(peer, project) -> may_read` before `get`, with CIDs bound to a project namespace.
- W2 [security][transfer] **`MemoryContentStore` is unbounded, never evicted, never persisted.** Every `publish_updates` adds a new snapshot block forever (`exchange/mod.rs:1408-1411`); every catalog sync adds the fetched block (`session.rs:186`). No GC, no pinning (DESIGN 15.1 promises pinning). Exploitable as memory DoS: a peer can cause up to 16 MiB per `CatalogUpdate` to be fetched and retained (see W44). Also everything is lost on restart, so advertised `catalog_cid`s go `NotFound` after the publisher restarts (`catalog_sync.rs:165-167`). Redesign: persistent, size-capped, project-scoped store with pin/GC.
- W3 [security] **`Cid` is an unvalidated string; `FileContentStore::path_for` splices it into a filesystem path.** `store.rs:78-86` does `root.join(prefix).join(rest)` on raw `cid.0`. A CID like `"../../etc/passwd"` makes `has()` (`:124-126`) answer existence of arbitrary paths and `get()` (`:103-121`) read arbitrary files (verification then fails, but the read and an error string containing the path happen). `put` is safe only because `verify()` forces the CID to be `blake3-<hex>` of the bytes. Design gap today (store is dormant) but a trap for the persistent-store redesign. Redesign: parse CID into a fixed binary form (multihash) and derive paths from the digest only.
- W4 [transfer][ai-game] **CID format is not a real CID and carries no codec/hash-agility.** `content/mod.rs:11-13`; `block.rs:7-10`. Raw chunks and `ChunkDag` roots are indistinguishable by CID; no multicodec, no versioning. Hex string CIDs are 71 bytes vs 36 for binary CIDv1 (≈2x overhead in every wantlist, DAG and gossip message). Design gap vs DESIGN 15.1 "IPFS integration".
- W5 [security][transfer] **`fetch_dag` trusts the decoded `ChunkDag` with no bounds.** `session.rs:218-222` decodes `chunks: Vec<Cid>` (anything up to the 16 MiB frame) and immediately `fetch_blocks` all of them; no cap on chunk count, no byte budget, no check that `chunks.len()` is consistent with `total_size`. A malicious root (or any peer answering a wanted CID — CIDs are self-certifying so the *root* must match, but the publisher chooses the root) can make a client open hundreds of thousands of streams and hold every block in RAM. Design gap (dormant) but will be the file path if DESIGN 15.1 is pursued.
- W6 [security] **Wantlist length is checked only after a full msgpack decode** (`bitswap.rs:237-238` then `:260-265`): an attacker sends a 16 MiB `WantList` body and the server allocates and parses it first. Also `SESSION_WANTLIST_CHUNK = 128` (`session.rs:31`) and `MAX_WANTLIST_LEN = 1024` (`bitswap.rs:39`) disagree although the comment says they are kept equal. Low; fix by streaming decode or a cheap pre-check on body size.
- W7 [security][transfer] **Response amplification with no per-peer accounting, rate limit or fairness.** One request of ≤1024 CIDs (~75 KB) can make the server read and send up to 1024 × 16 MiB (`bitswap.rs:267-283`, blocks fully buffered by `store.get`, `store.rs:56-61`). No Bitswap-style debt ratio/ledger, no concurrency cap per peer (daemon spawns one task per stream, `daemon.rs:800`). Exploitable for bandwidth/CPU exhaustion by any connected peer. Redesign: per-peer token bucket, max outstanding bytes, and streamed block reads.
- W8 [security] **No timeouts on bitswap stream I/O (either side).** `read_frame_body` and `read_exact` await indefinitely (`bitswap.rs:94-127, 187-190`); the server task per stream (`daemon.rs:800-817`) stays alive until the QUIC connection idle-times-out at connection level. Slowloris: a peer opens many streams, writes only the magic. Redesign: `tokio::time::timeout` per frame and per request; cap streams per peer.
- W9 [transfer] **No error frame in the protocol.** If `store.get` fails mid-list the server returns `Err` and the stream is cut (`bitswap.rs:275, 285-287`); the client reports "expected N responses, got M" (`session.rs:156-162`) and the whole session errors (`:137-140`). Partial results are discarded even though already-verified blocks were `put`. Design gap.
- W10 [transfer] **`Cancel` is a no-op** (`bitswap.rs:240-246`): the server keeps no wantlist state, so cancellation cannot stop an in-flight `WantList` on another stream. The module doc oversells it (`:8-9`). Design gap.
- W11 [transfer] **Single-peer, no retry, no multi-source, no want-have-driven peer selection.** `BitswapSession` is bound to one `peer` (`session.rs:46-47`); `have_query` exists but the fetch path never uses it; a single batch failure aborts everything (`:137-140`). DESIGN 15.1 promised parallel multi-source retrieval. Design gap.
- W12 [security] **Bitswap session requires a pre-existing QUIC connection and never dials.** `open_stream` fails with `PeerNotFound` if the publisher is not already connected (`quic/mod.rs:284-292`); `catalog_sync` does not call `connect` (`catalog_sync.rs:144-159`). Result: silent sync failure in the common case where the gossip reached us via a relay/mesh hop. Design gap.
- W13 [transfer] **Double hashing of every received block.** `session.rs:180` verifies, then `store.put` verifies again (`store.rs:47, 91`). Also `MemoryContentStore::get` clones the full `Vec<u8>` under the read lock (`store.rs:56-61`), so a 16 MiB block costs a 16 MiB copy per request.
- W14 [transfer] **`FileContentStore::put` is not atomic and does not fsync** (`store.rs:99`, `tokio::fs::write`). A crash leaves a partial block; afterwards `has()` says true (no verification, `:124-126`) while `get()` errors (`:107-115`) — so `want_have` answers `Have` for a block the server then fails to serve (W9). Design gap (dormant).
- W15 [security] **No audit log of who fetched what.** `daemon.rs:814-817` logs only errors; `handle_bitswap_stream` never receives the peer id. Combined with W1, data access is untraceable. Redesign: pass `sender` into the handler and emit structured access events.

### Chunker
- W16 [transfer][ai-game] **Fixed-size chunking, not content-defined.** `chunker.rs:36-41, 55-75` always cuts at `chunk_size` boundaries. Inserting or deleting bytes in a large game/AI asset shifts every subsequent chunk boundary and defeats dedup; DESIGN 15.1 specifies Rabin-fingerprint variable chunks. Design gap.
- W17 [transfer] **Flat single-level DAG with a 16 MiB root cap bounds the maximum file size.** The root block is one msgpack list of 71-byte CIDs (`chunker.rs:77-83`); it must fit a bitswap frame (`bitswap.rs:37`). ≈ 16 MiB / ~73 B ≈ 230k chunks ≈ 56 GiB at 256 KiB chunks; larger files cannot be fetched via bitswap. No intermediate nodes, so no partial verification of sub-ranges (DESIGN 15.1 claims partial verification). Design gap.
- W18 [transfer] **`get_file` writes directly to `out_path` and truncates before validating.** `chunker.rs:105` `File::create(out_path)` then streams; on a missing/bad chunk the half-written file remains (`:109-117`) and the previous content is already gone. No fsync (`:121`). Design gap (dormant); the live TXFR caller avoids this with a temp path, but a future bitswap file path would not.
- W19 [transfer] **`add_file` copies every chunk (`buf[..filled].to_vec()`, `chunker.rs:67`) and hashes on one thread; `ChunkDag.total_size` is only checked at the very end of `get_file`** (`:123-128`), after all bytes were written.
- W20 [ai-game] **No file-type awareness.** The chunker has no hooks for format-aware chunk boundaries (e.g., texture mip levels, safetensors tensors, glTF buffers) or for compression; every asset is treated as opaque bytes. Design gap relative to the "AI/game asset" goal.

### TXFR transfer
- W21 [security][transfer] **No bound on `TransferHeader.total_size`; any connected peer can push an arbitrarily large file.** `read_header` only checks `chunk_count` arithmetic (`transfer/mod.rs:132-142`); `receive_body` writes until the header size is reached (`:171-207`). The daemon accepts `TXFR` from any connected peer (`daemon.rs:805-808`) and only pinned fetches compare against an expected size (`exchange/mod.rs:757-766`). grep found no `max_file_size`/quota anywhere. Exploitable: disk exhaustion of `<project>/.synergos/incoming` (temp removed only after failure, `exchange/mod.rs:736-740`, so the attacker just never sends the footer, or sends many in parallel). Redesign: per-project/per-peer byte quotas, free-space check, and require an expected `(size, hash)` from a trusted offer/manifest before accepting a stream.
- W22 [security] **The TXFR protocol proves consistency, not authenticity.** Per-chunk hashes and the header/footer total hash are all sender-supplied (`transfer/mod.rs:93, 103-108`); a malicious sender just hashes its own bytes. Authenticity exists only when the receiver already holds a trusted expected hash (pins, `exchange/mod.rs:757-766`); the `content_hash` that `FileOffer` carries is discarded at `daemon.rs:936` (`content_hash: _`). The header is unsigned and `project_id`/`file_id` are attacker-chosen (`transfer/mod.rs:29-46`). Redesign: bind every transfer to a signed offer/manifest entry (file_id, version, size, blake3) and verify the received total hash against *that*, not against the header.
- W23 [transfer] **Sender marks the transfer Completed and the ledger Fulfilled without any receiver acknowledgment.** `send_stream` ends with `writer.shutdown().await.ok()` (`transfer/mod.rs:110`), ignoring errors, and `execute_send` calls `complete_transfer` on `Ok` (`exchange/mod.rs:673-676`) → `mark_fulfilled` (`:1068-1069`). If the receiver rejected the file (hash mismatch, stale version, conflict, `exchange/mod.rs:736-783`) the sender still believes it succeeded. The bidi stream's recv half (`_recv`, `:641`) is never read. Redesign: receiver sends a signed ack/nack frame; sender completes only on ack.
- W24 [transfer] **Sequential single stream, fixed 64 KiB chunks, no resume, no parallelism.** `transfer/mod.rs:11-12, 25, 81-102`. DESIGN 15.2 promises 1 MiB chunks for ≥100 MB files, multi-stream allocation and a bandwidth scheduler; `FileSizeClass` exists (`types.rs:197-217`) but nothing in `transfer`/`chunker` consults it. Each chunk is copied twice on send (`buf[..filled].to_vec()` at `:97`, then `rmp_serde::to_vec` at `:325`) and once on receive. The source file is read twice end-to-end (`hash_file` at `exchange/mod.rs:1139`, then `send_stream`). Design gap.
- W25 [transfer] **Hash mismatch is only detected after the entire file is on disk.** `receive_body` writes every chunk before the footer comparison (`transfer/mod.rs:195-196, 208-219`). A corrupted or malicious multi-GB transfer costs full disk and network before rejection. Redesign: when an expected hash is known, verify a Merkle/chunk-level hash list up front (ties to W22).
- W26 [transfer] **Framing conventions differ between protocols on the same connection.** TXFR uses `u32::to_le_bytes` (`transfer/mod.rs:327, 336`), bitswap uses `u32::to_be_bytes` (`bitswap.rs:84, 116`). Both are magic-prefixed and have no version byte. Low, but a redesign should define one framing with a version.
- W27 [security] **`TransferHeader.version` defaults to 0 for old peers and the receiver maps 0 → 1.** `transfer/mod.rs:44-45`; `exchange/mod.rs:750` `header.version.max(1)`. A legacy or malicious sender omitting `version` is treated as v1, which the stale/conflict checks (`:767-783`) then compare against local v1. `PEER_INFO_PROTOCOL_VERSION = 2` (`peer_info_server.rs:51-53`) is only enforced at HTTP bootstrap, not on the stream. Redesign: make `version` mandatory on the wire and reject 0.
- W28 [transfer] **`receive_body` never fsyncs** (`transfer/mod.rs:232` `flush` only). Combined with the caller's atomic rename this can produce a zero-length/partial file after a power loss on some filesystems. Low.

### Chain / ledger
- W29 [security] **`ChainBlock` cannot be transmitted and no code calls `verify()` outside tests.** No serde derives (`file_chain.rs:8-9, 132`); `FileChain::append` (`:164-179`) and `CatalogManager::record_update` (`manager.rs:77-108`) accept blocks without `verify()`; `is_signed()` is a non-zero heuristic (`:126-128`). If/when blocks are exchanged, unsigned or forged blocks are accepted. Design gap (dormant).
- W30 [security] **Chain hash and signature do not cover `file_id` or `project_id`.** `compute_hash` (`file_chain.rs:32-69`) and `signing_message` (`:74-87`). A validly signed block for file A can be appended to file B's chain if the `prev_hash` happens to match (e.g., both at genesis, `prev_hash: None`). Version monotonicity (`version > head.version`) and timestamp sanity are not enforced in `append`. Design gap (dormant).
- W31 [other] **Pruning to `max_depth` (default 10) discards the genesis, so a chain can never be re-verified from its root** (`file_chain.rs:175-177`); `find_common_ancestor` (`:221-227`) only works inside the window. DESIGN 17.3 says "old blocks GC'd" but does not address verifiability.
- W32 [security][ai-game] **CRC32 is the effective content identity throughout catalog and ledger; blake3 `content_hash` is always zero in live code.** `OfferEntry.crc` (`ledger.rs:27`), `FileEntry.crc`/`ChunkIndex.crc` (`types.rs:32, 81`), `catalog_crc` (`manager.rs:217-224`), `diff_catalog` compares only `crc` (`:144`), `build_catalog_snapshot` sets `content_hash: Blake3Hash::default()` (`exchange/mod.rs:288, 311`), `CatalogManager::add_file` likewise (`manager.rs:50`), and the daemon's conflict detection compares `(size, crc)` (`daemon.rs:957-960`). CRC32 collisions are trivial to craft, so a peer can publish a different file with the same size+CRC and it is "not a conflict". The "S5 対策" comments (`types.rs:26-27`) are aspirational. Redesign: carry and compare blake3 everywhere; drop CRC or keep it only as a cache hint.
- W33 [security] **`TransferLedger` grows without bound and any peer can overwrite the offer for any `(file_id, version)`.** Remote `FileOffer` → `register_offer` → `self.offers.insert(key, offer)` replaces whatever was there (`ledger.rs:118`; `exchange/mod.rs:1023-1032`). Wants/offers are never removed (no eviction API). Exploitable: (a) memory DoS with unique `(file_id, version)` floods on a project topic; (b) offer hijack — after an attacker's offer replaces the legitimate one, a local `fetch_file` gets `Match { sender: attacker, file_size: attacker's }` and sets `transfer.peer_id`/`file_size` to the attacker (`exchange/mod.rs:1285-1296`). The data path itself (TXFR from whoever pushes) does not consult the ledger, so the practical effect is state confusion plus the DoS. Redesign: scope keys by project, keep one offer per `(file, version, sender)`, TTL/eviction, and bound entries per peer.
- W34 [other] **Ledger state machine is incomplete vs DESIGN 17.4.** `Matched` is never set (`ledger.rs:88-95, 107-116`), offers never leave `Pending`, `register_offer` returns one identical `Match` per pending want without saying which requester (`:107-116`), `register_want` treats `Fulfilled` entries as duplicates (`:78-85`) so re-fetching an already-fetched version is reported `Duplicate` (the Exchange ignores this and continues, `exchange/mod.rs:1298-1300`). `get`-then-`entry().push()` is not atomic (`:78-98`).

### Catalog / catalog_sync
- W35 [other][transfer] **Two divergent catalog implementations; the drift check always fires.** `CatalogManager` per project is never populated (see §0), so `root_catalog()` is `{0, 0}`; the publisher's snapshot uses `update_count = notifications.len()` (not monotonic, `exchange/mod.rs:1403`) and `catalog_crc` = rolling CRC of this batch's file CRCs (`:1393-1394`), representing only the current publish batch, not the project. `daemon.rs:1008-1019` therefore emits `CatalogSyncNeededEvent` for every `CatalogUpdate` from every peer. DESIGN 18 describes a single monotonic root per project. Design/correctness gap.
- W36 [other] **`catalog_sync` fetches and discards.** `catalog_sync.rs:175-178` only type-checks the `RootCatalog`; no `diff_catalog`, no `CatalogManager` update, no `FetchNew`; `CatalogSyncCompletedEvent` has no consumer. DESIGN 18.3 steps 3-5 are unimplemented. The feature is observability only.
- W37 [security] **Unbounded fan-out of sync tasks driven by untrusted gossip.** One `tokio::spawn` per `CatalogSyncNeededEvent` (`catalog_sync.rs:53-55`), no dedup per `(project, catalog_cid)`, no concurrency cap, no backoff. Because of W35 every `CatalogUpdate` triggers one, and `catalog_cid` is attacker-chosen. Exploitable: a signed project member floods `CatalogUpdate` with fresh random CIDs → N concurrent bitswap streams to itself, each able to return 16 MiB that the victim stores forever (W2). Redesign: per-project single-flight, bounded queue, and verify `update_count` monotonicity before fetching.
- W38 [security] **`RootCatalog` snapshot leaks absolute filesystem paths to any connected peer.** `build_catalog_snapshot` sets `FileEntry.path = n.file_path.to_string_lossy()` (`exchange/mod.rs:286`) where `PublishNotification.file_path` is an absolute `PathBuf` (`tests/catalog_sync.rs:205` shows `/tmp/a.txt`); the block is then served by the unauthenticated-for-authz bitswap server (W1). This contradicts the S20 policy implemented by `redact_path` (`types.rs:225-239`). Exploitable information disclosure (usernames, project layout). Redesign: store project-relative paths only.
- W39 [security] **Untrusted `RootCatalog` is decoded with no structural limits and the legacy zero-`content_hash` fallback is a downgrade path.** `types.rs:34-36, 84-86` explicitly allow zero hashes; `update_count`/`catalog_crc` are not recomputed from contents on receipt (`catalog_sync.rs:175-176`). Design gap now (nothing consumes the catalog) but a redesign must verify `catalog_crc`/hashes against the chunk list.
- W40 [transfer] **`diff_catalog` is O(local_chunks × remote_chunks)** via linear `find` (`manager.rs:137-142`), `diff_chunk_files` linear per chunk (`:226-245`), `update_chunk_index` linear (`:206-215`), `recompute_catalog_crc` iterates every chunk on every update (`:217-224`). With `chunk_max_files = 256`, a 100k-file project has ~400 chunks → 160k comparisons per diff; manageable but quadratic. Deletions are never produced (`Remove` unreachable, `:137-158`), `FetchNew` only for files whose chain is missing locally (`:239-243`) and remote file lists are not in the root (`exchange/mod.rs:302-304`), so the diff cannot actually drive a fetch.
- W41 [other] **DashMap guards held across `.await` on a tokio `RwLock`.** `update_chunk_index` holds `self.chunks.get(chunk_id)` while awaiting `self.root.write()` (`manager.rs:206-215`); `record_update` holds `self.file_to_chunk.get(file_id)` across `update_chunk_index(...).await` (`:88-97`). DashMap documents that holding a `Ref` across an await can deadlock with writers on the same shard. `find_or_create_chunk` reads, drops, then creates — two concurrent `add_file`s can create two chunks or push past `max_files` (`:178-204, 58-60`). Design gap (no concurrent callers today).
- W42 [other] **Default mismatch:** `CatalogConfig::default()` is `{256, 10}` (`config.rs:520-526`) but `ipc_server::ensure_catalog_manager` falls back to `(128, 32)` when `net_config` is `None` (`ipc_server.rs:743-747`).
- W43 [transfer] **Lagged broadcast receivers silently drop sync events with no retry or periodic reconciliation** (`catalog_sync.rs:57-59`; `daemon.rs:1048-1050`). The channel capacity is 256 (`gossip/node.rs:121`). Under a burst, catalogs diverge with no recovery path. Design gap.
- W44 [other] **`CatalogSyncService::spawn` and `ServiceContext.content_store` are typed to the concrete `MemoryContentStore`** (`catalog_sync.rs:36, 75, 147`; `ipc_server.rs:54`; `exchange/mod.rs:364, 531`), so a persistent or ACL-aware store cannot be injected without touching every call site. Design rigidity.

### Cross-cutting
- W45 [ai-game][transfer] **No dedup on the live path.** The content-addressed machinery is dormant (§0); `publish_updates` → `FileOffer` → full-file `TXFR` every time a version changes, regardless of how much of a multi-GB asset actually changed. The "same content transferred once" claim in DESIGN 15.1 is not met for file bytes.
- W46 [security] **No per-peer or per-project resource accounting anywhere in this layer** (streams, bytes, blocks, ledger entries, sync tasks). Every limit present is per-frame or per-request, never per-principal.

## 5. Performance characteristics

Bitswap
- One request → N response frames on one QUIC bidi stream; client half-closes immediately (`bitswap.rs:183-184`); zero extra round trips beyond stream open. Max 1024 CIDs/request (`:39`); session uses 128/stream (`session.rs:31`) with 4 streams in flight by default (`:62`), driven by `FuturesUnordered` (`:126-146`).
- Server loop is sequential per stream: `store.get` (full `Vec` clone for memory store, `store.rs:56-61`; full file read for disk store, `:105`) → msgpack encode (another copy, `bitswap.rs:280-281`) → two `write_all`s (`:85-90`). Blocks ≤ 16 MiB are fully buffered on both ends (`:122-126`).
- Hashing: blake3-256 per block on receipt (`session.rs:180`) and again on `put` (`store.rs:47`): 2× hash cost. Hex-string CIDs (71 B) in every message.
- `have_query` is strictly sequential per batch (`session.rs:82-99`).

Chunker
- 256 KiB default chunks (`chunker.rs:36-41`), sequential read with `to_vec` copy per chunk (`:55-71`), blake3 per chunk, one msgpack root. Reassembly: sequential `store.get` + verify + `write_all`, `flush` only (`:108-121`).

TXFR
- 64 KiB chunks (`transfer/mod.rs:25`), msgpack frame per chunk with `u32le` length (`:324-331`), per-chunk blake3 + running blake3 (`:93, 182, 195`), strictly sequential on one stream, no pipelining knobs. Send side: 2 copies/chunk; receive side: 1 decode copy + `write_all` (`:196`). Whole file read twice by the sender (`hash_file` then `send_stream`). Receiver pays CRC32 pass after receipt (`exchange/mod.rs:743`) → the file is read three times in total across sender+receiver before the hash-of-interest is confirmed. Progress events every 128 KiB (`exchange/mod.rs:625`).
- `MAX_FRAME` 66560 B limits header string sizes implicitly.

Chain / ledger
- `FileChain` ops are O(depth) linear scans (`file_chain.rs:182-227`), depth ≤ 10 by default. `TransferLedger` DashMap with `Vec<WantEntry>` per key, linear scans per key (`ledger.rs:78-85, 107-116, 128-149`). No eviction → memory grows with every distinct `(file_id, version)` seen.

Catalog
- crc32fast for chunk and root CRCs (`types.rs:66-72`; `manager.rs:217-224`); `recompute_catalog_crc` O(chunks) per update; `diff_catalog` O(n×m) (W40). `root_catalog()` clones the whole `RootCatalog` on every gossip `CatalogUpdate` (`daemon.rs:1010`, `manager.rs:164-166`).

catalog_sync
- One task per event, one QUIC stream per event (W37); 16 MiB max block retained in memory.

## 6. Config knobs & defaults

| Knob | Where | Default | Configurable? | Secure-by-default? |
|---|---|---|---|---|
| `ChunkerOptions.chunk_size` | `chunker.rs:36-41` | 256 KiB | code only (no `NetConfig` field) | n/a |
| `MAX_BITSWAP_FRAME` | `bitswap.rs:37` | 16 MiB | const | partial (caps one frame, not a request's total) |
| `MAX_WANTLIST_LEN` | `bitswap.rs:39` | 1024 | const | partial (W6, W7) |
| `SESSION_WANTLIST_CHUNK` | `session.rs:31` | 128 | const | — |
| `BitswapSession::with_max_inflight` | `session.rs:62, 66-69` | 4 (min 1) | builder | — |
| `CHUNK_SIZE` (TXFR) | `transfer/mod.rs:25` | 64 KiB | const | — |
| `MAX_FRAME` (TXFR) | `transfer/mod.rs:339` | 66560 B | const | yes for frame size; no cap on `total_size` (W21) |
| `CatalogConfig.chunk_max_files` | `config.rs:513-526` | 256 (ipc fallback 128) | TOML `[catalog]` | — |
| `CatalogConfig.chain_max_depth` | `config.rs:513-526` | 10 (ipc fallback 32) | TOML `[catalog]` | — |
| Content store backend / size cap / TTL | none | in-memory, unbounded | no | **no** (W2) |
| Bitswap authz / per-peer quota / timeouts | none | open to all connected peers | no | **no** (W1, W7, W8) |
| Gossip `require_signature` | `gossip/node.rs:132, 148-150` | `false` in `GossipNode::new`; grep found no `set_require_signature` caller in `synergos-core/src` | code | **no** by default (envelope verification happens in `on_signed_message_received`, `:216-226`, but the unsigned path `on_message_received` exists, `:228-230`; whether the daemon ever feeds unsigned messages is outside this map's scope) |

Assessment: frame-level limits are sane, but there is no principal-level limit, no authorization, no timeout, and no persistence configuration, so the layer is not secure-by-default against a hostile but authenticated peer.

## 7. Test coverage observed

Covered
- `block.rs` unit: CID determinism/length, verify success/tamper (`block.rs:30-60`).
- `store.rs` unit: memory roundtrip, CID mismatch rejection, file roundtrip, on-disk tamper detection (`store.rs:135-186`).
- `chunker.rs` unit: 1,000,000-byte roundtrip with 64 KiB chunks; missing root errors (`chunker.rs:137-172`).
- `bitswap.rs` unit: serde roundtrips for all request/response variants (`bitswap.rs:294-371`).
- `session.rs` unit: empty `have_query` no-op; local short-circuit (`session.rs:247-298`).
- `tests/bitswap_e2e.rs`: single block over real QUIC; `NotFound`; chunker + per-chunk `request_block` + `get_file` reassembly (200 KB, 32 KiB chunks).
- `tests/bitswap_v2.rs`: `WantList` of 3; mixed Block/NotFound order; `have_query` Have/DontHave; `Cancel` closes cleanly; `fetch_dag` pulls root + all chunks (120 KB, 16 KiB chunks) and a second `fetch_blocks` is all local.
- `transfer/mod.rs` unit: 10 KiB roundtrip through `tokio::io::duplex`; tampered chunk hash fails (`transfer/mod.rs:356-435`).
- `file_chain.rs` unit: signed block verifies; tampered payload fails; append/head; fork detection; max depth pruning (`file_chain.rs:253-335`). `ledger.rs` unit: want→offer, offer→want, duplicate want (`ledger.rs:180-248`). `tests/net_additional.rs`: `mark_fulfilled`, `cancel_want`, order independence, `record_update` on unknown file errors, per-project CRC isolation.
- `manager.rs` unit: add + record_update bumps `update_count`; chunk overflow creates a second chunk (`manager.rs:250-300`).
- `synergos-core/tests/catalog_sync.rs`: drift detection emits/doesn't emit; unknown project ignored; `publish_updates` emits `FileOffer` + `CatalogUpdate` locally. Note it re-implements the daemon's dispatch logic by hand (`catalog_sync.rs:56-88`) rather than exercising `daemon.rs:1003-1047`.
- `synergos-core/tests/catalog_sync_e2e.rs`: two real QUIC nodes; `CatalogSyncNeededEvent` → bitswap → `CatalogSyncCompletedEvent { fetched_blocks: 1, error: None }`.
- `catalog_sync.rs` unit: missing `catalog_cid` → completed-with-error without network.

Not covered
- Oversized bitswap frame (> 16 MiB) rejection; wantlist > 1024; truncated length prefix; response-count mismatch; CID mismatch in response; unexpected variant; slow/hanging peer; concurrent stream limits; server error mid-list.
- `FileContentStore` with hostile CID strings (W3); `has()` on corrupt block (W14).
- `fetch_dag` with hostile root (huge `chunks`, `total_size` mismatch); `get_file` size-mismatch branch; partial output cleanup.
- TXFR: out-of-order chunk, oversized chunk, `received_bytes > total_size`, header `chunk_count` mismatch, duplicate header, huge `total_size` (W21), version 0 handling, sender behaviour when receiver rejects (W23).
- Chain: `verify()` never called from `append` (so no test can catch W29); cross-file replay (W30); version monotonicity.
- Ledger: offer overwrite by another sender (W33); growth/eviction; `Fulfilled`-as-duplicate semantics.
- Catalog: `diff_catalog` has **no test at all**; `set_file_state`; concurrent `add_file`; defaults mismatch.
- catalog_sync: `NotFound`, decode failure, publisher not connected (`PeerNotFound`), lagged receiver, flood/dedup; anything consuming `CatalogSyncCompletedEvent`.

## 8. Documented-vs-implemented deltas (DESIGN.md)

- 15.1 (lines 1403-1466): describes `IpfsBlock { links }`, `IpfsFileStream { block_sizes }` with **Rabin variable-length chunks**, a `BlockExchange` trait with `get_wantlist`/`send_block` (push) and "already-held blocks skipped", "partial verification / parallel retrieval", "streaming before completion", "pinning". Implemented: flat `ChunkDag` (`chunker.rs:22-27`), fixed-size chunks, pull-only `Want/WantList` (`bitswap.rs:41-56`), skip-local only inside `BitswapSession` (`session.rs:106-118`), no pinning/GC, no streaming consumer, and none of it is on the file path (§0).
- 15.2 (lines 1468-1569): size-class chunk sizes 1 MiB / 256 KiB / 64 KiB, `BandwidthScheduler`, per-class stream limits. Implemented: TXFR fixed 64 KiB (`transfer/mod.rs:25`), chunker fixed 256 KiB, no scheduler in this layer; `FileSizeClass::classify` exists in `types.rs:207-217` but is unused here.
- 17.1 (lines 2421-2451): "binary → IPFS chunk → chain write", "FileWant written to chain". Implemented: binary files go whole-file over TXFR; wants/offers go to the in-memory `TransferLedger`, nothing is written to any chain.
- 17.2 (lines 2453-2487): text diff (`TextDiff`, `FileContentType::detect`) — only the `ChainPayload::TextDiff` variant exists (`file_chain.rs:135`); no diff generation or application code in this layer.
- 17.3 (lines 2489-2610): `ChainBlock` implemented and *extended* with `author_public_key` + ed25519 `signature` (`file_chain.rs:9-28`), `ChainError::Fork` became `SynergosNetError::ChainFork` (`error.rs:19-23`). But blocks are never serialized/exchanged and `record_update` is never called (§0).
- 17.4 (lines 2612-2741): `TransferLedger` matches the sketch almost verbatim; deltas: impl excludes `Cancelled` entries from the duplicate check (`ledger.rs:78-85` vs design 2692-2697); impl returns `vec![Queued]` when no match (`:120-124` vs design returning an empty vec); "matched entries transition to Fulfilled" is only done for wants on transfer completion.
- 18.1-18.2 (lines 2745-2879): structures implemented plus `content_hash` fields; `CatalogDiff.conflicts` is `Vec<FileId>` not `Vec<ConflictInfo>` (`types.rs:114` vs 2921); `FileUpdateAction::ApplyRemote`/`FetchNew` carry only `file_id`, not `blocks`/`cid` (`types.rs:121-128` vs 2924-2939). `record_update` steps 1-5 are implemented (`manager.rs:77-108`) but unreachable in the daemon.
- 18.3 (lines 2881-2940): steps 1-2 (compare `update_count`/crc) implemented at `daemon.rs:1008-1018`, but against an empty local catalog (W35); steps 3-5 (chunk CRC compare, per-file compare, chain-based conflict) not implemented — `catalog_sync` discards the fetched root (W36). The gossiped `updated_chunks` is always empty (`exchange/mod.rs:613`).

## 9. Open questions for designers

1. Is the content-addressed bitswap/DAG path meant to *replace* TXFR for file bytes (DESIGN 15.1), or stay a metadata channel? The answer decides whether W5/W16/W17/W18 are urgent.
2. What is the trust model for a project topic member: is "any connected peer may read any CID" acceptable, or must the content store be project-scoped with an ACL checked per request (W1)? Where would the project-membership oracle live (Cernere per org rules)?
3. Should the content store be persistent (disk, size-capped, pinned) and shared with the history store, or remain a volatile cache? What is the eviction policy and who owns quotas (W2, W21, W46)?
4. Are chain blocks ever supposed to leave the process? If yes, define the wire form, who validates version monotonicity, and bind `file_id`/`project_id` into the signed message (W29, W30).
5. Real CIDv1 (multihash blake3 `0x1e`, raw/dag-cbor codecs) for interop, or a private binary CID? Hex strings should not survive a redesign (W4).
6. Content-defined chunking (FastCDC/Rabin) and a multi-level DAG for multi-GB game/AI assets; do you want format-aware chunk hints (W16, W17, W20)?
7. Who consumes `CatalogSyncCompletedEvent` and the fetched `RootCatalog`? Should the publisher gossip a full-project root (monotonic `update_count`) or keep per-batch snapshots (W35, W36)?
8. Can CRC32 be removed entirely in favour of blake3 (W32)? If a cheap hint is still wanted, should it be the first 8 bytes of blake3 rather than CRC?
9. Should TXFR gain a signed header + receiver ack, or be folded into the bitswap path with a per-file Merkle root from a signed manifest (W22, W23, W25)?
10. Unify framing (endianness, version byte, error frame) across `BSW1`/`TXFR`/`DHT`/`GOSSIP` streams (W9, W26)?
11. Per-peer resource budgets (streams, in-flight bytes, ledger entries, sync tasks) — a single policy object or per-protocol constants (W7, W8, W33, W37)?
