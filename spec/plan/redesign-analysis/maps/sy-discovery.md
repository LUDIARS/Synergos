# Synergos subsystem map: peer discovery & membership

Scope: DHT (`synergos-net/src/dht/*`), gossip (`synergos-net/src/gossip/*`), relay client (`synergos-net/src/relay/mod.rs`), relay server (`synergos-relay/*`), peer-info servlet (`synergos-core/src/peer_info_server.rs`), URL bootstrap (`peer_bootstrap.rs`), join helper (`peer_join.rs`), self-contained invite tokens (`invite_token.rs`), control-plane heartbeat (`control_heartbeat.rs`), presence (`presence/mod.rs`). All paths below are relative to `Synergos/` unless absolute. Every file in scope was read in full. Where I looked at adjacent code to confirm wiring (daemon.rs, ipc_server.rs, quic/mod.rs, quic/hello.rs, conduit/mod.rs, config.rs, types.rs, identity.rs, synergos-control heartbeat API), I cite it the same way.

Convention: "exploitable" = an attacker that can reach the surface today can trigger it; "design gap" = missing control that a redesign has to add. Claims marked **(inference)** are deductions from code reading, not observed at runtime.

---

## 1. Responsibilities & data flow

### 1.1 What this subsystem is supposed to do
Discover other daemons, decide who is a member of which project, keep a presence table per project, and get bytes to peers that are not directly reachable. In the current tree, the functional surface actually exercised in production is much narrower than the module list suggests (see §8).

### 1.2 Production data flow (as wired in `synergos-core/src/daemon.rs`)

```
                         invite token (syn1.<json>.<sig>)  ── Slack / chat ──►  joiner
                                     ▲                                             │
   host daemon                       │ ipc ProjectInvite (ipc_server.rs:979-1008)  │ ipc ProjectJoin (ipc_server.rs:1032-1124)
   ┌──────────────────────┐          │                                             ▼
   │ peer_info_server     │◄── HTTP GET /peer-info ──────────── peer_bootstrap::bootstrap_from_url_expected
   │ (axum, no auth)      │   JSON {peer_id, quic_endpoint,       (peer_bootstrap.rs:78-189)
   │ peer_info_server.rs  │          protocol_version, ...}              │ parse, size cap 64 KiB, version check
   └──────────────────────┘                                              │ resolve candidates (DNS + advertised)
             ▲                                                           ▼
             │ /peer-info may call 3rd-party IP echo       QuicManager::connect(expected_peer_id, addr)  (quic/mod.rs:205-260)
             │ services on every request                      TLS cert pinned to blake3(pubkey)[:20]; then HLO1 Hello
             │ (peer_info_server.rs:109-134)                             │
                                                                          ▼
                                                   PresenceService::register_node + update_node_state(Connected)
                                                   (peer_join.rs:19-45; presence/mod.rs:352-407, 448-461)
                                                                          │
   ┌──────────── per-connection stream dispatcher (daemon.rs:766-840) ────┴──────────────────────────────┐
   │  magic DHT1 → dht::handle_dht_stream(dht, send, recv)        (dht/transport.rs:81-110)            │
   │  magic GSP1 → gossip::handle_gossip_stream(gossip, recv, sender) (gossip/transport.rs:78-106)      │
   │  magic TXFR / BSW1 → exchange / bitswap (out of scope)                                              │
   └────────────────────────────────────────────────────────────────────────────────────────────────────┘

   GossipNode::publish(topic, msg)  (gossip/node.rs:182-213)
        ├─ local broadcast `tx` → spawn_gossip_subscriber (daemon.rs:917-1057): FileWant/FileOffer → Exchange,
        │                          PeerStatus → presence.handle_peer_status, ConflictAlert → ConflictManager,
        │                          CatalogUpdate → CatalogSyncNeededEvent
        └─ `outbound_tx` (signed envelope + mesh peers) → spawn_gossip_fanout (daemon.rs:847-907)
                 peers empty (always, see §4) → fallback: EVERY peer in quic.list_connections()
                 → open_stream(Control) → send_gossip (GSP1 + u32 len + msgpack GossipWireMessage)

   HeartbeatReporter (control_heartbeat.rs) ── POST {node_id, peer_id, mesh_ip, synergos_version} + Bearer node_key ──► synergos-control /v1/heartbeat
```

### 1.3 What enters / leaves each module

| Module | Inputs | Outputs | Calls into |
|---|---|---|---|
| `dht::DhtNode` | `DhtRequest` from any authenticated QUIC peer via `handle_dht_stream`; local `announce`/`add_peer` from `PresenceService` | `DhtResponse`; in-memory `store` + `routing_table` | `types::NodeId` (blake3 of peer_id string), `DhtTransport` (only `QuicDhtTransport`, test-only in practice) |
| `gossip::GossipNode` | `GossipWireMessage` from any authenticated QUIC peer; local `publish` from Exchange/Presence/Project | `tx` broadcast `(TopicId, GossipMessage)`; `outbound_tx` broadcast `OutboundGossip` | `identity::Identity` (ed25519 sign/verify), `MessageId::from_content` (blake3) |
| `relay::RelaySession` | ws(s) URL, room_id, local PeerId; `RelayData` from queue | `RelayData` from `recv` | `tokio_tungstenite`, `rmp_serde`, `serde_json` |
| `synergos-relay` server | TCP → WS; text `ClientControl::Join/Leave`; binary frames | binary frames to other room members | `dashmap`, `tokio_tungstenite` |
| `peer_info_server` | HTTP GET `/peer-info`, `/health` (unauthenticated) | `PeerInfoResponse` JSON | `QuicManager::local_addr`, `promotion::probe_ipv6_global`, `reqwest` to third-party echo services |
| `peer_bootstrap` | base URL (from CLI `peer add-url`, config `bootstrap_urls`, or invite token) | `BootstrapResult {peer_id, synergos_version}`; side effect: entry in `QuicManager.connections` | `reqwest`, `tokio::net::lookup_host`, `QuicManager::connect` |
| `peer_join` | `ServiceContext`, project_id, url, optional expected peer | `BootstrapResult`; side effect: presence row | `peer_bootstrap`, `PresenceService` |
| `invite_token` | `Identity`, project metadata (issue); token string + now (decode) | token string / `InvitePayload` | `identity::{verify, peer_id_from_public_bytes}`, `base64`, `serde_json`, `uuid` |
| `control_heartbeat` | `ControlReportConfig`, env var node key, local PeerId | HTTP POST every `interval_secs` | `mesh_ip::detect_mesh_ipv4`, `reqwest` |
| `presence::PresenceService` | `NodeRegistration` (from join/bootstrap/IPC), `PeerActivityStatus` (from gossip), RTT/bandwidth from QUIC snapshot | `RegisteredNode` list for `peer list`; `PeerConnectedEvent`/`PeerDisconnectedEvent` on event bus; DHT announce + gossip PeerStatus (only via `register_self`, never called in production) | `DhtNode`, `GossipNode`, `event_bus` |

### 1.4 Who actually calls what in production (verified by grep over `**/src/**/*.rs`)
- `DhtNode::find_peer_iterative` and `QuicDhtTransport`: **no production callers**; only `synergos-net/tests/dht_find_node.rs` and `dht_over_quic.rs`.
- `PresenceService::register_self`: **no production callers** (only trait def + impl in `presence/mod.rs:85,289`). Consequently `dht_announce` (presence/mod.rs:211-224) and `broadcast_status(Active)` (presence/mod.rs:184-208, 335) never run. `unregister_self` is called at shutdown (daemon.rs:530) but `local_node` is always `None`, so it is a no-op.
- `PresenceService::discover_project_peers`: **no production callers**.
- `GossipNode::graft` / `prune`: **no production callers** (only tests). `subscribe` is called from `ProjectManager::open_project` (project.rs:742-744) which creates an empty mesh `Vec`.
- `RelaySession::connect`: **no production callers**. `Conduit::try_connect_routes` returns `Ok(RouteKind::Relay)` without opening anything (conduit/mod.rs:421-427, comment "WebSocket Relay は今後実装"; relay-only branch conduit/mod.rs:371-385 same).
- `DhtNode::handle_request` IS reachable from the network (daemon.rs:801-804) for `Ping`, `FindNode`, `FindProjectPeers`, `Announce`.
- `DhtNode::gc_expired` runs every 60 s (daemon.rs:580-601). `GossipNode::heartbeat` runs every `heartbeat_interval_ms` clamped to 100..60000 ms (daemon.rs:1060-1080).
- `HeartbeatReporter`: built at daemon start, fail-fast on bad config (daemon.rs:328-331), spawned at daemon.rs:420-421.
- `peer_info_server::run` spawned only when `peer_info_listen_addr` is set (daemon.rs:393-416), `server_name` hardcoded `"synergos-core"`.
- `bootstrap_urls` loop (daemon.rs:431-489): uses `bootstrap_from_url` (no expected peer), registers the peer into **all** open projects.

---

## 2. Key types / APIs / wire formats

### 2.1 Identifiers
- `PeerId(pub String)` (types.rs:8). Canonical value is `hex(blake3(ed25519_pubkey)[..20])` = 40 hex chars (identity.rs:37, 339-343). Nothing in this subsystem enforces that shape; `PeerId::new` accepts any string.
- `NodeId([u8;32]) = blake3(peer_id_string_bytes)` (types.rs:42-45). XOR metric over 256 bits (types.rs:48-54); bucket index = leading zeros of XOR distance, clamped to 255 (routing.rs:128-132).
- `TopicId = "project/<project_id>"` (types.rs:117-119).
- `MessageId = blake3(topic || 0x00 || canonical_bytes(msg))` (gossip/node.rs:310-317).

### 2.2 DHT RPC (`dht/rpc.rs`)
- Framing: `[u32 BE length][msgpack body]`, preceded on QUIC by magic `DHT1` (4 bytes) in both directions (dht/transport.rs:21, 41-45, 104-108). `MAX_FRAME_SIZE = 1 MiB` (rpc.rs:178), enforced on encode (rpc.rs:146-152), decode (rpc.rs:166), and before allocation on both client (transport.rs:59-63) and server (transport.rs:89-93).
- One request/response per bidi stream; stream opened with `StreamType::Control` (transport.rs:38).
- `DhtRequest::{FindNode{target}, FindProjectPeers{project_id}, Announce{record}, Ping}` (rpc.rs:103-112).
- `DhtResponse::{Nodes{exact, closest}, ProjectPeers{peers}, AnnounceAck, Pong, Error{message}}` (rpc.rs:116-132).
- `PeerRecordDto {peer_id, display_name, endpoints: Vec<RouteDto>, active_projects: Vec<String>, ttl_secs: u64}` (rpc.rs:25-31). No signature, no issue timestamp, no sequence number. `RouteDto::{Direct{addr: SocketAddrV6, fqdn}, Tunnel{tunnel_id, hostname}, Relay{server_url, room_id}}` (rpc.rs:36-49).
- Serialization: `rmp_serde` externally-tagged enums; no protocol version field.

### 2.3 DHT node state (`dht/node.rs`, `dht/routing.rs`)
- `store: DashMap<NodeId, PeerRecord>` capped at `DHT_STORE_MAX = 10_000` (node.rs:82); `PeerRecord.published_at` is set to receiver's `Instant::now()` on every `from_dto` (node.rs:43-52).
- `RoutingTable`: 256 `KBucket`s of `k = DhtConfig.k_bucket_size` (routing.rs:98-101); `KBucket::upsert` LRU with stale replacement after `STALE_THRESHOLD = 15 min` (routing.rs:7, 47-74).
- `FindNodeOptions` default: timeout 3 s, max_hops 4, alpha 3, k 20 (node.rs:68-77).

### 2.4 Gossip (`gossip/message.rs`, `gossip/transport.rs`)
- Wire: magic `GSP1` + `[u32 BE len]` + msgpack `GossipWireMessage {topic, signed: SignedGossipMessage}` (transport.rs:25, 34-37, 50-74). `MAX_GOSSIP_FRAME = 256 KiB` (transport.rs:30), enforced before allocation (transport.rs:88-93). Unidirectional: receiver drops its send half (daemon.rs:810).
- `SignedGossipMessage {message, sender_public_key: Vec<u8> (32), signature: Vec<u8> (64)}` (message.rs:14-20). Signature is ed25519 over `signing_bytes(message)`, a hand-rolled tag+concatenation (message.rs:101-188) with **no length prefixes** on variable-length strings (only some lists are NUL-terminated).
- `GossipMessage::{PeerStatus{peer_id, status, origin, hops}, CatalogUpdate{project_id, root_crc, update_count, updated_chunks, catalog_cid: Option<Cid> (serde default), publisher: PeerId (serde default = empty)}, FileWant{requester, file_id, version}, FileOffer{sender, file_id, version, size, crc, content_hash (serde default)}, ConflictAlert{file_id, conflicting_nodes, their_versions}}` (message.rs:192-244).
- `GossipControl::{Graft, Prune, IHave, IWant}` enum exists (message.rs:273-282) but has **no serde derive, no wire format, no handler**.
- `GossipNode` channels: `tx` and `outbound_tx` are `tokio::sync::broadcast` with capacity 256 (node.rs:121-122).

### 2.5 Relay (`relay/mod.rs`, `synergos-relay/src/server.rs`)
- Client→server first message: text frame JSON `{"type":"join","room_id":..,"peer_id":..}` (relay/mod.rs:37-40, 73-82). Server deserializes `ClientControl` where `peer_id` is `Option<serde_json::Value>` and ignored (server.rs:19-27).
- Data: binary WS frames carrying msgpack `RelayData {from_peer_id, payload: serde_bytes}` (relay/mod.rs:45-49). Server forwards the raw bytes without parsing (server.rs:209-215, 80-89). The module doc at relay/mod.rs:17-18 claims a `u16 be length` prefix; the code writes none (relay/mod.rs:90-97). Doc/impl delta.
- No message size limit set on either side (tungstenite defaults apply). No TLS in the server (lib.rs:13-15); the client supports `wss://` via `rustls-tls-webpki-roots` (synergos-net/Cargo.toml:41) with public-CA validation only.

### 2.6 Peer-info (`peer_info_server.rs`, `peer_bootstrap.rs`)
- `GET /peer-info` → `PeerInfoResponse {peer_id, quic_endpoint: Option<String>, protocol_version: u32 (=2), server_name, synergos_version}` (peer_info_server.rs:31-48, 53, 56). `GET /health` → `"ok"`.
- Client side caps body at `MAX_PEER_INFO_RESPONSE_BYTES = 64 KiB` (peer_bootstrap.rs:25, 112-124), rejects `protocol_version != 2` (peer_bootstrap.rs:129-134), parses `quic_endpoint` as `SocketAddr` (peer_bootstrap.rs:140-142).

### 2.7 Invite token (`invite_token.rs`)
- Format `syn1.<base64url(JSON payload)>.<base64url(ed25519 sig over the JSON bytes)>` (invite_token.rs:28, 69-73). `InvitePayload {project_id, display_name?, host_peer_id, host_pubkey (b64url 32 B), peer_info_url, expires_at?: epoch secs, nonce: uuid v4}` (invite_token.rs:35-48).
- Limits: token ≤ 16 KiB, project_id ≤ 256 and no control/`/`/`\`, display_name ≤ 256 no control, peer_info_url ≤ 2048 (invite_token.rs:29-32, 107-126).
- Legacy UUID tokens (no `syn1.` prefix) are daemon-local (`ProjectManager::invites`, ipc_server.rs:1036-1050).

### 2.8 Control heartbeat
- POST JSON `{node_id, peer_id, mesh_ip: Option<String>, synergos_version}` with `Authorization: Bearer <node_key>` (control_heartbeat.rs:101-114). Server side (synergos-control/src/api/heartbeat.rs:41-108) hashes the key with blake3 and compares in constant time inside the store mutation.

### 2.9 Presence
- `PeerState::{Discovered, Connecting, Connected, Idle, Away, Disconnected}` (presence/mod.rs:23-36); `RegisteredNode {peer_id, display_name, endpoints, state, rtt_ms, project_ids, bandwidth_bps, last_seen: Instant, synergos_version}` (presence/mod.rs:40-52). Storage: `DashMap<PeerId, RegisteredNode>` (presence/mod.rs:119).

---

## 3. Security controls PRESENT

| # | Control | Evidence |
|---|---|---|
| C1 | Inbound DHT/gossip streams are only accepted on QUIC connections whose peer identity was established: outbound via TLS cert pinning to `blake3(pubkey)[:20]` (`PeerPinningVerifier`), inbound via signed `HLO1` Hello verified before the connection is registered. | quic/mod.rs:205-236, 362-407; quic/verifier.rs:22-70; quic/hello.rs:77-103, 134-173; dispatch uses `acc.peer_id` (daemon.rs:733, 766-811) |
| C2 | Gossip envelopes are ed25519-signed and verified on receipt; signer key is bound to the self-declared sender field for `PeerStatus.origin`, `FileOffer.sender`, `FileWant.requester`, and `CatalogUpdate.publisher` (when non-empty). Key/sig length checked before use. | gossip/message.rs:24-30, 36-90; gossip/node.rs:162-168, 216-226; transport.rs:104 |
| C3 | MessageId derived from canonical bytes (not `{:?}`), topic-scoped, so identical content in different projects is not deduped away. | gossip/node.rs:182-190, 310-317; message.rs:93-99; test node.rs:433-456 |
| C4 | Gossip frame size cap 256 KiB checked before `vec![0; len]`. | gossip/transport.rs:30, 53-59, 88-93 |
| C5 | DHT frame size cap 1 MiB on encode, decode, and before allocation on both sides; length/actual mismatch rejected. | dht/rpc.rs:143-178; dht/transport.rs:56-63, 86-93 |
| C6 | DHT store bounded at 10 000 records with GC-then-evict-oldest; expired records filtered on read. | dht/node.rs:79-82, 293-310, 109-117, 281-290, 342-344 |
| C7 | k-bucket eviction resists flooding: full bucket only replaces an entry that has been silent ≥ 15 min; otherwise the newcomer is dropped (S22). | dht/routing.rs:5-7, 47-74 |
| C8 | Iterative lookup bounded by `max_hops`, `alpha`, per-request timeout, `k` truncation, and a `queried` set. | dht/node.rs:134-241; tests dht_find_node.rs:194-260 |
| C9 | Invite token: ed25519 signature over exact JSON bytes; signer pubkey must hash to `host_peer_id`; expiry enforced; size and charset limits on project_id/display_name/url; redirect-free URL validation (scheme http/https only, no creds, no query/fragment) before any local side effect. | invite_token.rs:81-143; peer_bootstrap.rs:193-215; ipc_server.rs:1061-1070 |
| C10 | Bootstrap HTTP client: timeout, `redirect::Policy::none()`, response size cap 64 KiB, protocol version check, `quic_endpoint` parsed as `SocketAddr`, and when `expected_peer` is given the identity is compared before DNS/QUIC. | peer_bootstrap.rs:93-134, 136-142, 153-160 |
| C11 | QUIC connect always pins `expected_peer_id`, so a lying `/peer-info` can at worst misdirect to an endpoint that then fails TLS pinning. | peer_bootstrap.rs:170-173; quic/mod.rs:205-236 |
| C12 | Peer-info servlet documented to bind loopback and be published via a tunnel; `advertised_addr` literal path avoids outbound probes. | peer_info_server.rs:199-210, 72-74 |
| C13 | Control heartbeat: node key read from env var, never from config; fail-fast on missing node_id/key; URL scheme/credential checks; plaintext `http` only to loopback or Cloudflare Mesh `192.0.2.10/12`; `no_proxy`; no redirects; 10 s timeout; non-2xx logged at `warn`. Server side: constant-time hash compare, unknown node ⇒ same 401 as bad key, input length/control-char validation, reported vs. expected mesh IP kept separate. | control_heartbeat.rs:26-66, 69-83, 119-122, 130-144; synergos-control/src/api/heartbeat.rs:46-105 |
| C14 | Relay server: `max_rooms` and `max_clients_per_room` caps; sender excluded from its own broadcast; rooms isolated by id; second `join` ignored; per-client outbound queue of 64 with `try_send` (no unbounded buffering); empty rooms pruned. | synergos-relay/src/server.rs:59-69, 80-89, 187-201, 160, 71-77 |
| C15 | Relay-only mode strips `Route::Direct` from self-advertisement (home IP privacy). | presence/mod.rs:171-181, 289-311 |
| C16 | Logging uses `PeerId::short()` in most hot paths (S24). | dht/node.rs:161, 187, 192; daemon.rs:783, 884 |
| C17 | DashMap same-shard deadlock in `heartbeat` fixed and regression-tested. | gossip/node.rs:271-283, 325-368 |
| C18 | Gossip heartbeat interval clamped to 100..60000 ms regardless of config. | daemon.rs:1065-1066 |
| C19 | Extension-magic streams capped at 1 MiB via `read_to_end(MAX_EXTENSION_PAYLOAD)`. | daemon.rs:775-777, 821 |

---

## 4. Weaknesses, TODOs, missing controls FOUND

Severity is my guess for the *redesign* (what it would cost the product), tagged by theme.

### Membership / authorization (security)

**W1. There is no project membership ACL anywhere in the peer path.** Any peer that can complete a QUIC handshake with *any* ed25519 key is a full member of every project the receiving daemon has open: its gossip is accepted for any topic (`handle_gossip_stream` delivers to `wire.topic` without checking subscription or sender membership, gossip/transport.rs:99-105; `deliver` has no topic check, node.rs:244-251), its DHT requests are answered (node.rs:245-278), and a `FileOffer` it sends for an open project triggers `fetch_file` from the attacker (daemon.rs:936-995, eligibility = project open + not already held). The invite-token module says so explicitly: "現状 Synergos にはプロジェクト単位の参加 ACL が無い ... このトークンは「参加手順の同梱」であって認可境界ではない。認可は Cloudflare Mesh 等のネットワーク層で行う" (invite_token.rs:19-21). The accept path has no allowlist (quic/mod.rs:362-407). **Exploitable** by anyone with network reach to the QUIC port (the mesh checklist opens 4433/udp to the whole `192.0.2.10/12` range, docs/mesh-two-node-checklist.md:40). Theme: security, critical. A redesign needs a host-side membership registry keyed by PeerId (ideally rooted in Cernere identity), enforced at Hello time and per-topic on every inbound stream, with an explicit join handshake that consumes the invite.

**W2. Invite tokens are bearer, unbounded, multi-use, non-revocable, and unlimited by default.** `expires_at: None` = no expiry (invite_token.rs:44-45, 137-141); the CLI default is "省略すると無期限" (docs/projects-and-peers.md:35). The `nonce` is never checked for reuse anywhere (invite_token.rs:47, 160; no consumer). The issuing host never learns a token was used (the module header admits "発行側での使用回数管理はできない", invite_token.rs:17). Tokens are meant to be pasted into Slack (docs/projects-and-peers.md:45). Design gap, high. A redesign needs short default TTL, single-use nonces tracked by the host, host-side validation at join time, and invitee binding (token issued to a specific invitee pubkey or exchanged for a session credential).

**W3. `PeerStatus` lets any authenticated peer change any other peer's presence state.** Verification binds only `origin` to the signer (message.rs:56-61); `PeerStatus.peer_id` and `status.peer_id` are unchecked, and `handle_peer_status` ignores `_origin` and mutates `nodes[activity.peer_id]` (presence/mod.rs:267-284). A peer can mark everyone `Disconnected`/`Away` in every receiver's `peer list`. Exploitable, medium (integrity of presence; `peer list` is what operators use to check connectivity).

**W4. `ConflictAlert` has no signer binding at all** (message.rs:85-86) and is applied directly to `ConflictManager::handle_conflict_alert` (daemon.rs:1000-1002). Any authenticated peer can create phantom conflicts naming arbitrary files and peers. Exploitable, medium (ai-game: conflict UI noise and blocked publishes).

**W5. `CatalogUpdate` with empty `publisher` skips the signer check** (message.rs:77-84, "旧ピア互換"), and the subscriber emits `CatalogSyncNeededEvent` with `publisher: None` (daemon.rs:1003-1041). A legacy-compat hole that lets any peer announce catalog drift for any project. Exploitable, medium.

**W6. Signed `HLO1` Hello is replayable (adjacent, but it is the root of membership identity).** `SignedHello.ts_ms` is signed but never checked for freshness (hello.rs:77-103 has no time check), there is no server-supplied nonce or TLS channel binding, and the server uses `no_client_auth` (hello.rs:3-6). Any party that ever *received* a Hello from victim V (every node V connects to, including a malicious `peer add-url` target) can replay it to another server S; S then inserts the attacker's connection under V's PeerId (quic/mod.rs:384-400), overwriting V's genuine entry and hijacking S's outbound gossip/transfer streams to V. Exploitable, high. Redesign: server nonce + TLS exporter in the signed bytes, or mutual TLS with ed25519 client certs.

**W7. DHT records are unauthenticated and unbound to the sender.** `handle_dht_stream` does not receive the authenticated `sender` (dht/transport.rs:81-85; daemon.rs:802), so `Announce{record}` for any `peer_id` with any endpoints/projects is stored (node.rs:270-276) and `add_peer` overwrites the routing-table endpoints for that NodeId (routing.rs:48-54). `FindNode`/`FindProjectPeers` then serve the poisoned records to third parties (node.rs:248-269). `ttl_secs` is attacker-chosen with no cap (`u64`, rpc.rs:30; node.rs:50) so records never expire; `published_at` is reset to the receiver's `now` on every hop (node.rs:49). Exploitable today for eclipse/poisoning of whatever consumes the DHT (nothing in production yet, §1.4), design gap for the intended discovery. Theme security, high. Redesign: signed, timestamped, sequence-numbered records (like libp2p signed peer records), TTL cap, and sender == record.peer_id for Announce.

**W8. Sybil/eclipse cost is zero.** NodeId is `blake3(peer_id)` and peer_id is a free ed25519 key hash (types.rs:42-45; identity.rs:339-343); there is no admission, proof-of-work, or key registration. Bucket placement is attacker-controlled by key grinding. Design gap, high for any public DHT; moot if the DHT stays private/mesh-only.

**W9. Gossip fan-out leaks every project's traffic to every connected peer.** `mesh` entries are only ever created empty (`subscribe`, node.rs:171-173; `graft` has no production caller), so `publish` returns `vec![]` and the fan-out task falls back to `quic.list_connections()` (daemon.rs:860-871). `FileOffer`/`FileWant`/`CatalogUpdate` for project A are sent to peers that are only members of project B, carrying file paths (`FileId`), sizes, CRCs, catalog CIDs and project ids. Exploitable (passive) / design gap, high for a studio with multiple clients or NDAs. Theme security + ai-game.

**W10. `FindProjectPeers` and `PeerRecord.active_projects` disclose membership to anyone.** No authorization on `FindProjectPeers` (node.rs:261-269); the DHT record itself carries the full `active_projects` list and `display_name` (rpc.rs:25-31). Design gap, medium (privacy/NDA; `display_name` is person-identifying data in a service DB, which the org rules forbid).

**W11. `/peer-info` is unauthenticated and discloses peer_id, public IP:port, and exact build version.** peer_info_server.rs:70-88; the docs instruct binding it on `0.0.0.0:7780` for the mesh (docs/mesh-two-node-checklist.md:47-48) while the code comment says never to (peer_info_server.rs:201-203). `synergos_version` enables version-targeted attacks. Design gap, medium.

**W12. `/peer-info` auto-advertise performs up to six outbound HTTPS requests to third-party IP echo services on *every* request, uncached, each with a 5 s timeout.** peer_info_server.rs:72-79, 109-134, 150-185 (`None`/`"auto"` is the default, config.rs:45-49). An unauthenticated client can make the node hammer icanhazip/ipify/ident.me (amplification, ~30 s worst-case latency per GET, a new `reqwest::Client` per probe family), and the node's existence/IP is disclosed to those third parties on every bootstrap. Exploitable, medium. Redesign: resolve once at startup / on interface change, cache with TTL, and do not depend on third parties for a security-relevant value.

**W13. `peer add-url` / `bootstrap_urls` are trust-on-first-use over optionally plaintext HTTP.** `validate_bootstrap_url` allows `http` (peer_bootstrap.rs:196-200); with `expected_peer = None` (ipc_server.rs:1190; daemon.rs:438) the peer_id learned from the response is what gets pinned (peer_bootstrap.rs:152-161). An on-path attacker (or a hijacked DNS name) substitutes its own peer_id and endpoint and becomes a "member" of the project (and, with W1, every open project for `bootstrap_urls`, daemon.rs:446-467). Exploitable off-mesh, high. Redesign: require https or embed the expected peer_id in the URL/token for every bootstrap path.

**W14. `bootstrap_urls` registers the remote into every open project** regardless of whether the remote is meant to be in them (daemon.rs:426-430, 446-467). Combined with W9 the remote receives all projects' gossip. Design gap, medium.

**W15. `peer connect <project> <peer-id>` fabricates a `Connected` presence row for any string without a connection** (ipc_server.rs:1144-1169). Presence is not tied to transport state; `peer list` can show peers that do not exist. Design gap, low (local IPC only).

### Relay (security + transfer)

**W16. The relay server has no authentication, no TLS, and `room_id` is the only secret.** server.rs:151-228, lib.rs:13-15; default bind `0.0.0.0:8080` (config.rs:34). `room_id` is advertised in cleartext DHT records (`RouteDto::Relay{server_url, room_id}`, rpc.rs:45-48), so anyone who can read a record can join the room and receive (and inject into) all traffic. The relayed payload is opaque; nothing in the client encrypts it (relay/mod.rs:140-145). Exploitable if deployed, critical for the relay path. Redesign: authenticate joins with the node identity (signed challenge), bind rooms to project membership, terminate TLS or require wss, and carry QUIC (or an AEAD-wrapped channel) inside the relay so the server never sees plaintext.

**W17. `RelayData.from_peer_id` is client-asserted and forwarded verbatim.** The server does not parse binary frames (server.rs:209-215); the client trusts `from_peer_id` on receive (relay/mod.rs:111-118; tests rely on it, relay_integration.rs:41-42). Exploitable spoofing inside a room. Theme security, high.

**W18. Relay server has no frame-size, rate, pre-join, or idle limits.** `idle_timeout_secs` is parsed but never used (config.rs:14-18; no reference in server.rs). Connections that never `join` are unbounded and not subject to `max_rooms`/`max_clients_per_room` (server.rs:172-219). `room_id` length is unbounded (server.rs:20-23). Each broadcast clones the payload per recipient (`payload.to_vec()`, server.rs:85) so a 64-client room turns 1 MiB into 64 MiB of allocations per frame; tungstenite's default message cap (~64 MiB) applies. Exploitable DoS, medium. The comment "落ちたらそのクライアントを切り捨てる" (server.rs:84) is not what the code does; it silently drops the frame (server.rs:85).

**W19. `max_rooms` check is racy** (`rooms.len() >= max_rooms && !contains_key` then `entry().or_default()`, server.rs:59-67) and `leave` runs `rooms.retain` over every room (server.rs:76). Low.

**W20. `RelaySession::shutdown`/`Drop` leak the WebSocket.** `task.abort()` aborts only the supervisor that is `try_join!`-ing the reader/writer `JoinHandle`s (relay/mod.rs:126-128, 157-166); dropping a `JoinHandle` detaches rather than cancels the child task, so the writer/reader tasks and the socket stay alive until the server closes. Bug, low today (no callers), medium for a redesign that uses it. Also the supervisor does not cancel the writer when the reader ends (`try_join!` waits for both).

**W21. Relay client `recv` is a single `Mutex<Receiver>`** (relay/mod.rs:57, 148-155): one consumer at a time, 64-deep queues each way, no backpressure signal to the server (the server `try_send`s and drops). Transfer theme, low.

### DHT / gossip protocol robustness (security + transfer)

**W22. Hand-rolled canonical signing bytes are ambiguous.** Variable-length strings are concatenated without length prefixes (message.rs:101-188); e.g. `ConflictAlert{file_id:"doca", conflicting_nodes:[""]}` and `{file_id:"doc", conflicting_nodes:["a"]}` serialize to identical bytes and therefore identical `MessageId` and signature. The author flags it as "暫定" (message.rs:94-96). Consequence: targeted dedup suppression of a legitimate message and signature transplant between semantically different messages of the same signer. Design gap, medium; use a length-prefixed/CBOR canonical encoding and include a message version.

**W23. No replay protection on gossip beyond the bounded `MessageCache`.** Cache is 1000 entries by default (config.rs:725), GC drops the oldest 25 % (node.rs:41-88), and messages carry no sequence number or timestamp except `PeerStatus.last_active` (unchecked). Any captured signed envelope (`FileOffer`, `PeerStatus Offline`, `CatalogUpdate`) can be re-injected later by any connected peer; the receiver re-runs auto-pull eligibility (daemon.rs:978-995) and presence updates. Exploitable, medium.

**W24. Gossip is one hop; `hops` is never incremented and nothing re-forwards.** `deliver` only pushes to the local broadcast channel (node.rs:244-251); the DESIGN propagation model (DESIGN.md:2391-2417, "hops+1", "hops が小さい方を採用") does not exist. Membership information therefore cannot cross a non-fully-connected mesh. Design gap, high for the intended topology (transfer/ai-game: FileOffers from peers two hops away are never seen).

**W25. Gossipsub mesh management is absent.** `heartbeat` only trims to `mesh_n_high` by popping the tail (node.rs:278-283, 300-307); no GRAFT below `mesh_n_low`, no PRUNE selection by score, no IHAVE/IWANT, no fanout handling (`fanout` is `#[allow(dead_code)]`, node.rs:99-101). `mesh_n`/`mesh_n_low` are dead config. Design gap, medium (transfer reliability).

**W26. `DhtConfig.peer_ttl_secs` and `routing_refresh_secs` are never read**; `dht_announce` hardcodes a 120 s TTL (presence/mod.rs:219) and there is no routing refresh task (DESIGN.md:2260-2261 `refresh_routing_table` unimplemented). Config knobs are decorative. Low.

**W27. Local gossip delivery uses a lossy `broadcast` channel of 256.** A burst of >256 messages (e.g. a large publish producing many `FileOffer`s) drops messages for lagging subscribers; the daemon only logs (daemon.rs:1048-1050, 898-900). Dropped `FileOffer`/`FileWant` = silently missed auto-pull. Transfer theme, medium. Redesign: bounded mpsc with backpressure or persistent offer ledger reconciliation.

**W28. Per-peer resource limits are missing on the DHT/gossip stream handlers.** Each stream spawns a task (daemon.rs:800) and allocates up to 1 MiB (DHT) / 256 KiB (gossip) before deserialization (dht/transport.rs:94; gossip/transport.rs:94); `max_concurrent_streams` default 100 per connection (config.rs:707) with no per-peer or global rate limit. `FindNode` does an O(N log N) sort over the whole routing table per request (routing.rs:110-121) and `Announce` when the store is full runs two O(10k) scans (node.rs:296-307). Exploitable CPU/memory pressure from any authenticated peer, medium.

**W29. Accept loop is serialized on the Hello (adjacent).** `QuicManager::accept` awaits `recv_hello` (up to 5 s) inline (quic/mod.rs:384-390; hello.rs:31, 170-173) and the daemon loop only spawns after `accept()` returns (daemon.rs:723-760). One client that completes the QUIC handshake and never sends `HLO1` delays every other peer's admission by 5 s each; a slow-loris of such connections blocks discovery. Exploitable DoS, medium.

**W30. Stale `connections` entries after inbound disconnect (inference).** `connections.remove` only happens in `disconnect`/`shutdown` (quic/mod.rs:263-274, 409-419); `dispatch_peer_streams` returns on close without touching `connections` or presence (daemon.rs:780-786). `list_connections()` (used for gossip fan-out fallback) and `peer list` can keep showing dead peers as `Connected`; presence `last_seen` only moves on RTT snapshot or gossip. Transfer/ai-game, medium. Redesign: connection-close hook → presence `Disconnected` + `PeerDisconnectedEvent`.

**W31. Advertised PeerId from `/peer-info` is unvalidated free text.** `PeerId::new(&info.peer_id)` (peer_bootstrap.rs:152) accepts up to 64 KiB with control characters; it becomes `display_name` and is logged in full (`register_node`, presence/mod.rs:363-367; `peer_join.rs:31`). Log injection / UI garbage; harmless for auth because pinning fails for a non-40-hex id. Low.

**W32. Control-plane heartbeat does not prove key possession for `peer_id`.** The daemon sends `peer_id` as a plain string under the node key (control_heartbeat.rs:103-114); the server overwrites `synergos_peer_id` on every beat (heartbeat.rs:88-91). A stolen node key rebinds a node to an attacker identity, and the control plane cannot detect a daemon whose identity file was swapped. Also the control plane is observational only: nothing in the daemon consumes a verdict from it, so it cannot act as a brake ("歯止め") on a compromised or revoked node. Design gap, high for the security theme. Redesign: ed25519-sign the heartbeat body (peer_id proven by signature), include a monotonic counter, and have the daemon poll/receive an allow/revoke decision (membership list, key revocation) that gates `accept`.

**W33. `is_trusted_plaintext_host` accepts the hostname `localhost`** (control_heartbeat.rs:134-136), which resolves via the hosts file / resolver rather than being a literal loopback; low.

**W34. Heartbeat and discovery failures are log-only.** `warn!` on 401/404 (control_heartbeat.rs:119-122), `warn!` on bootstrap failures (daemon.rs:481-483); no metrics/event-bus signal, so the GUI cannot show "this node is unregistered". Low (ops).

**W35. No version negotiation on QUIC sub-protocols.** `protocol_version` is only checked at `/peer-info` (peer_bootstrap.rs:129-134); DHT1/GSP1 frames carry no version, and enum additions in msgpack break old peers (rpc.rs:103-132; message.rs:192-244 rely on `serde(default)` for new fields only). Design gap, low-medium (transfer: mixed-version meshes fail opaquely).

**W36. Invite `display_name` and `project_id` are attacker-chosen inputs to `open_project`** (ipc_server.rs:1086-1094). Charset is restricted (invite_token.rs:107-126) but `project_id` becomes a gossip topic and a DHT key; collisions with an existing local project silently reuse it (ipc_server.rs:1071-1076). A crafted token can join a victim into an attacker-controlled "project" that shares an id with the victim's real project (W1 then applies). Design gap, medium.

---

## 5. Performance characteristics

- **Hashing:** blake3 for NodeId/MessageId/PeerId derivation (types.rs:43, 128; identity.rs:340); ed25519 sign per publish and verify per receive (message.rs:24-30, 88). No batching.
- **DHT frames:** `[u32][msgpack]`, 1 MiB cap; one QUIC bidi stream per round trip; `FindNode` reply holds ≤ 20 records (node.rs:251). `find_peer_iterative`: α = 3 parallel requests per hop, ≤ 4 hops, 3 s per request ⇒ worst case ~12 s; candidate dedup is `Vec::contains` O(n) with n ≤ k = 20 (node.rs:216-218); per-hop sort O(k log k) (node.rs:231-236).
- **Routing table:** `closest` collects all entries from 256 buckets and sorts (O(N log N), N ≤ 256·k = 5120 at k = 20) on every `FindNode` request and every lookup round (routing.rs:110-121). `upsert` is O(k) per bucket (routing.rs:48, 57-62).
- **Store:** `DashMap` sharded; `find_project_peers` is a full scan O(N) with a `String` allocation per compare (`project_id.to_string()` inside the filter, node.rs:286); `announce` when full: `gc_expired` O(N) then `min_by_key` O(N) (node.rs:296-307); `gc_expired` every 60 s O(N) (daemon.rs:601).
- **Gossip:** publish = 1 signature + 1 broadcast send + 1 `OutboundGossip` clone; fan-out spawns one task and one QUIC stream per peer per message (daemon.rs:872-896), i.e. O(peers) streams per publish with no coalescing; frame ≤ 256 KiB; broadcast channel depth 256 (node.rs:121-122). `MessageCache::gc` is O(n log k) with k = n/4 via a max-heap (node.rs:41-88).
- **Presence:** `list_nodes` O(n) scan with `pid.to_string()` per row (presence/mod.rs:433-442); RTT/bandwidth snapshot copied every 60 s per connection (daemon.rs:610-616).
- **Relay:** per-connection mpsc(64) writer; broadcast copies the payload once per recipient (server.rs:80-89); `leave` scans all rooms (server.rs:76). Client allocates per frame via `rmp_serde::to_vec` (relay/mod.rs:90-96).
- **Bootstrap:** 1 HTTPS GET (≤ 64 KiB) + DNS + sequential QUIC connects over candidates (peer_bootstrap.rs:163-185), 10 s client timeout (peer_join.rs:26).
- **Peer-info:** potentially 6 sequential HTTPS probes × 5 s per request when auto-advertise is on (peer_info_server.rs:150-185), plus NIC enumeration; blocking latency for the joiner.
- **Disk I/O:** none in this subsystem (all state is in memory; identity load is elsewhere). Nothing persists DHT/routing/presence across restarts, so every restart starts with an empty routing table.
- **Heartbeats:** gossip `heartbeat` iterates topics each tick (default 1 s) (node.rs:278-283); control heartbeat 1 HTTP POST / 60 s.

---

## 6. Config knobs & defaults (secure-by-default assessment)

| Knob (file:line) | Default | Used? | Secure by default? |
|---|---|---|---|
| `dht.k_bucket_size` (config.rs:716) | 20 | yes (node.rs:99) | n/a |
| `dht.routing_refresh_secs` (config.rs:717) | 60 | **no** | n/a |
| `dht.peer_ttl_secs` (config.rs:718) | 120 | **no** (hardcoded 120 s, presence/mod.rs:219) | n/a |
| `gossipsub.mesh_n` / `mesh_n_low` (config.rs:721-722) | 6 / 4 | **no** | n/a |
| `gossipsub.mesh_n_high` (config.rs:723) | 12 | yes (node.rs:303) | n/a |
| `gossipsub.heartbeat_interval_ms` (config.rs:724) | 1000 | yes, clamped 100..60000 (daemon.rs:1066) | yes |
| `gossipsub.message_cache_size` (config.rs:725) | 1000 | yes | weak replay window (W23) |
| `GossipNode.require_signature` (node.rs:116, 132) | `false` | only affects the unsigned API `on_message_received`, which the wire path never uses (transport.rs:104) | moot; wire path always verifies |
| `peer_info_listen_addr` (config.rs:30, 748) | `None` (servlet off) | yes | yes (off); docs recommend `0.0.0.0:7780` (W11) |
| `peer_info_advertised_url` (config.rs:37, 749) | `None` | yes (peer_join.rs:50-70) | n/a |
| `quic_advertised_addr` (config.rs:54, 750) | `None` = auto (third-party echo) | yes | **no** (W12) |
| `bootstrap_urls` (config.rs:61, 751) | `[]` | yes | TOFU over http allowed (W13) |
| `force_relay_only` / `auto_promote` (config.rs:70, 77, 752-753) | false / true | yes (daemon.rs:106-128) | relay path is a no-op (W16, §8) |
| `control.heartbeat_url` / `node_id` / `node_key_env` / `interval_secs` (config.rs:476-509) | "" / "" / `SYNERGOS_NODE_KEY` / 60 | yes | yes (off by default, fail-fast, env-sourced key) |
| Invite `--expires` (ipc_server.rs:982-993) | `None` = never expires | yes | **no** (W2) |
| `FindNodeOptions` (node.rs:68-77) | 3 s / 4 hops / α 3 / k 20 | test-only | n/a |
| `DHT_STORE_MAX` (node.rs:82) | 10 000 (const) | yes | ok |
| `STALE_THRESHOLD` (routing.rs:7) | 15 min (const) | yes | ok |
| Relay `listen_addr` (synergos-relay config.rs:34) | `0.0.0.0:8080` plaintext | yes | **no** |
| Relay `max_rooms` / `max_clients_per_room` (config.rs:21-26) | 4096 / 64 | yes | ok |
| Relay `idle_timeout_secs` (config.rs:27-29) | 300 | **no** | n/a |
| `HELLO_TIMEOUT` (hello.rs:31) | 5 s | yes | serialized accept (W29) |

---

## 7. Test coverage observed

Tested:
- DHT iterative lookup with a mock transport: local hit short-circuit, exact hit via neighbour, two-hop chain, hop-limit, per-request timeout, server-side `FindNode` exact, `Ping` (dht_find_node.rs:115-287). Note `handle_request_ping_pong` uses `matches!` without `assert!` (dht_find_node.rs:286) so it cannot fail.
- DHT over real QUIC: one client, one server, `FindNode` resolves a record the server announced (dht_over_quic.rs:40-112).
- DHT framing round-trips and malformed length rejection (rpc.rs:180-224). k-bucket upsert and `closest` ordering (routing.rs:135-181).
- Gossip: heartbeat deadlock regression, publish returns grafted peers, duplicate suppression, topic-scoped dedup (node.rs:319-457); end-to-end publish → QUIC GSP1 → remote subscriber with a real signature (gossip_mesh_e2e.rs:48-141); other core e2e tests (two_node_full_e2e, auto_pull_e2e, versioned_republish_e2e, history_node_e2e) drive gossip similarly but hand-graft the mesh.
- Presence handlers: `PeerStatus` Idle/Offline mapping, unknown-peer no-op; conflict-alert registration/merge (gossip_handlers.rs:16-114).
- Relay: client↔in-process echo server round-trip (relay_session.rs); client↔`synergos-relay` server: bidirectional exchange, no self-echo, room isolation (relay_integration.rs). Note `two_clients_exchange_binary_frames_via_relay_server` uses `sleep(150 ms)` to wait for join (relay_integration.rs:30), a race.
- Peer-info: `None` endpoint before bind, literal advertised addr, `/health`, auto path keeps the bind port (peer_info_server.rs:245-468; the auto tests will hit the public internet if reachable).
- Bootstrap: URL building, unsafe URL rejection, candidate resolution for unspecified/specific/unresolvable hosts (peer_bootstrap.rs:305-388). Advertised-URL precedence (peer_join.rs:72-103).
- Invite token: round-trip, expiry boundary, tampered payload, foreign key, legacy prefix (invite_token.rs:166-223).
- Control heartbeat: plaintext host allow-list only (control_heartbeat.rs:146-167). Server-side heartbeat auth and validation (synergos-control heartbeat.rs:110-295).

Not tested:
- Any negative/adversarial case on the DHT server path: unsigned `Announce` from a non-owner, oversized `active_projects`, `ttl_secs = u64::MAX`, store eviction at 10 000, stale-bucket replacement timing, concurrent lookups.
- Gossip signature failures over the wire (bad key length, wrong signer for `FileOffer`, empty-publisher `CatalogUpdate`), `PeerStatus` with `peer_id != origin`, replay after cache eviction, frame exactly at/above 256 KiB, topic not subscribed, `require_signature` path.
- Fan-out fallback to all connections, lagging broadcast subscribers, `enforce_mesh_bounds` choice.
- Relay: frame size limits, `max_rooms`/`max_clients_per_room` enforcement, pre-join flood, `idle_timeout_secs`, `shutdown` actually closing the socket, spoofed `from_peer_id`.
- Peer-info: non-JSON/oversized responses, protocol version mismatch end-to-end, http vs https behaviour, DNS rebinding.
- Invite: `expires_at` absent, nonce reuse, `project_id` collision with an existing local project, display-name limits at the IPC boundary, token > 16 KiB.
- Presence: `register_node` duplicate semantics vs re-join, stale `Connected` after transport loss, relay-only route stripping (`sanitize_local_routes` has no test).
- Control heartbeat client: no test of the request body or of the 401 handling.
- No fuzzing of msgpack decoders (`DhtRequest`, `GossipWireMessage`, `RelayData`, `SignedHello`).

---

## 8. Documented-vs-implemented deltas

| Doc claim | Reality |
|---|---|
| DESIGN.md:2160-2166, 2201-2263: Kademlia DHT is the "global peer registry"; `find_peer` does network FIND_NODE; `refresh_routing_table` exists. | `find_peer` is local-store only (node.rs:119-123); `find_peer_iterative` exists but has no production caller; no refresh task; nobody announces (`register_self` unused). The DHT is a server-side handler with no client in the daemon. |
| DESIGN.md:2216-2222: 160-bit space, `k` default 20. | 256-bit blake3 space, 256 buckets (routing.rs:93-101). |
| DESIGN.md:2265-2293, 2367-2388: Gossipsub with mesh/fanout, GRAFT/PRUNE/IHAVE/IWANT, heartbeat grafts below `mesh_n_low`, cache GC in heartbeat. | Only `mesh_n_high` trimming (node.rs:300-307); no control messages on the wire; no fanout logic; cache GC only on insert overflow (node.rs:34-36). |
| DESIGN.md:2391-2417: multi-hop PeerStatus propagation, "hops が小さい方を採用". | One hop, `hops` constant 0 (presence/mod.rs:203), no forwarding (W24). |
| DESIGN.md:2336-2345: `last_active: Instant`, `working_on: Option<Vec<FileId>>`. | `last_active: u64` ms, `working_on: Vec<FileId>` (message.rs:248-256). |
| DESIGN.md:2307-2313, 2320-2327: `CatalogUpdate`/`FileOffer` fields. | Added `catalog_cid`, `publisher`, `content_hash` with serde defaults (message.rs:211-216, 235-236); signatures and `SignedGossipMessage` are not in DESIGN at all. |
| docs/projects-and-peers.md:73-74: "Gossipsub + DHT + Cloudflare Tunnel で自動ピアディスカバリ ... peer は勝手に見つかります". | No automatic discovery exists; peers are learned only via invite token, `peer add-url`, or `bootstrap_urls`. |
| docs/projects-and-peers.md:88-89 and config.rs:62-70: route column may be `relay`; relay-only forces traffic through `synergos-relay`. | `Conduit` returns `RouteKind::Relay` without connecting (conduit/mod.rs:371-385, 421-427); `RelaySession` is never constructed; relay-only mode has no data path. |
| relay/mod.rs:17-18: binary frames carry `u16 be length` + msgpack. | No length prefix is written or read (relay/mod.rs:90-97, 111). |
| synergos-relay server.rs:84 comment: backpressure drops the *client*. | Drops the *frame* (server.rs:85). |
| synergos-relay config.rs:14-18: `idle_timeout_secs` disconnects idle clients. | Never implemented. |
| peer_info_server.rs:199-203: never bind `0.0.0.0`. | docs/mesh-two-node-checklist.md:47-48 instructs `peer_info_listen_addr = "0.0.0.0:7780"`. |
| peer_info_server.rs:10-12: "Tower middleware で auth layer を後付けする想定". | No auth layer; router is two unauthenticated routes (peer_info_server.rs:192-197). |
| config.rs:613-616: `routing_refresh_secs`, `peer_ttl_secs` tune the DHT. | Unused (§6). |
| config.rs:621-624: `mesh_n`, `mesh_n_low` tune the mesh. | Unused (§6). |
| gossip/node.rs:93-96 comment: `local_peer_id` for "自分向けメッセージを除外". | `#[allow(dead_code)]`; no self-origin filtering in `deliver`; `handle_peer_status` doc says "origin が自分の場合は無視" (presence/mod.rs:265) but the code ignores `_origin` entirely (presence/mod.rs:270-284). |
| invite_token.rs:21: "認可は Cloudflare Mesh 等のネットワーク層で行う". | Nothing in the daemon verifies that a connection came over the mesh; the QUIC listener is `[::]:port` (daemon.rs:89-96). |

---

## 9. Open questions for designers

1. Is a public/global DHT actually wanted, or is discovery meant to be mesh-private (Cloudflare Mesh / Tailscale) plus a control-plane registry? The answer decides whether W7/W8/W10 are "delete the DHT" or "sign records + admission control".
2. Where does project membership live authoritatively: Cernere (org rule: auth centralizes in Cernere), synergos-control, or the host daemon? Who can revoke, and how does a revoked peer get disconnected in real time (W1, W32)?
3. Should invite tokens be exchanged for a membership credential at join (host-validated, single-use), with the invitee's PeerId bound in, rather than being bearer secrets?
4. Is the relay meant to carry QUIC (end-to-end encrypted, server blind) or application frames? If the former, the relay only needs to authenticate joins and meter bandwidth; if the latter, every W16-W21 item is on the critical path.
5. Should gossip be project-scoped at the transport (per-topic peer sets derived from the membership registry) so W9 disappears structurally, instead of relying on gossipsub mesh formation?
6. What is the presence liveness model: QUIC connection state (authoritative, local), signed heartbeat gossip with sequence numbers, or control-plane heartbeats? Today it is none of these.
7. Does display_name belong on the wire at all given the "no personal data in service DBs" rule? It is currently in DHT records, gossip, and presence tables.
8. Should `/peer-info` be replaced by a signed, cacheable "node card" (peer_id, pubkey, endpoints, version, expiry, signature) so bootstrap over plain http is still integrity-protected (W13), and so the node does not depend on third-party IP echo services at request time (W12)?
9. What is the intended mixed-version policy for DHT1/GSP1/HLO1? Add a version byte after the magic now, before any external deployment.
10. Which of DHT lookup, gossip multi-hop, and the relay data path are in scope for the first product release? Each is currently dead or stubbed; shipping docs that describe them as working creates a false sense of reachability and security.
