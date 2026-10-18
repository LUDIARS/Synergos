# Subsystem map: synergos-net transport (QUIC, identity, hello/verifier, tunnel, mesh/STUN, promotion, conduit)

Repository: `Synergos` (Rust workspace). Crate: `synergos-net` (`synergos-net/Cargo.toml`, version 0.1.0).
Files covered completely: `src/lib.rs`, `src/config.rs`, `src/types.rs`, `src/error.rs`, `src/identity.rs`, `src/mesh_ip.rs`, `src/promotion.rs`, `src/quic/{mod,verifier,hello}.rs`, `src/tunnel/mod.rs`, `src/mesh/{mod,stun}.rs`, `src/conduit/mod.rs`, tests `tests/quic_peer_auth.rs`, `tests/quic_control_stream.rs`, `tests/net_additional.rs`; docs `DESIGN.md` §3.2-3.6 and §15.3-15.5, `docs/platforms.md`. Call sites in `synergos-core/src/daemon.rs` and `synergos-core/src/peer_bootstrap.rs` were read only to establish how the daemon drives this subsystem (marked as such).

Locked dependency versions (from `Synergos/Cargo.lock`): quinn 0.11.11, quinn-proto 0.11.17, rustls 0.23.44 (rustls 0.21.12 also present transitively), ring 0.17.14, aws-lc-rs 1.18.1 (present in graph; code pins `ring`), rcgen 0.13.2, x509-parser 0.16.0, ed25519-dalek 2.2.0, argon2 0.5.3, aes-gcm 0.10.3, rmp-serde 1.3.1, hickory-resolver 0.24.4, igd-next 0.16.2, socket2 0.6.5.

---

## 1. Responsibilities and data flow

### 1.1 What this subsystem is
`synergos-net` is described as the "P2P network foundation" (`src/lib.rs:1-4`). The transport slice consists of:

| Module | Responsibility (as implemented) | Production status (daemon wiring) |
|---|---|---|
| `identity.rs` | ed25519 node keypair, `PeerId = hex(blake3(pubkey)[..20])`, key file persistence (plaintext or AES-256-GCM+Argon2id) | Live: `daemon.rs:72-75` loads via `Identity::load_or_generate(&Identity::default_path())` |
| `quic/mod.rs` | quinn `Endpoint` lifecycle, outbound `connect(expected_peer_id, addr, server_name)`, inbound `accept()`, stream open, connection registry (`DashMap<PeerId, QuicConnection>`), RTT/bandwidth bookkeeping, `ConnectionCalibrator` | Live: `daemon.rs:85-97` (bind `[::]:0` or `quic.listen_addr`), `daemon.rs:723-760` serial accept loop, `peer_bootstrap.rs:169-172` outbound connect |
| `quic/verifier.rs` | rustls `ServerCertVerifier` that pins the server's ed25519 SPKI to `expected_peer_id` | Live (used on every `connect`) |
| `quic/hello.rs` | `HLO1` application-layer signed Hello so the server learns the client's PeerId (server does no TLS client auth) | Live (sent in `connect`, required in `accept`) |
| `tunnel/mod.rs` | Spawns and supervises a `cloudflared tunnel run` child process; exposes state/health | Effectively dead: no `tunnel.start()` call in `synergos-core/src` (grep); only `tunnel.stop()` at shutdown `daemon.rs:549` |
| `mesh/mod.rs` | FQDN -> AAAA resolution (DoH w/ DNSSEC then system DNS), UDP "reachability" probe, TURN session stubs, STUN wrapper | Effectively dead: only `cleanup_expired_sessions()` is called (`daemon.rs:550,604`); no resolve/probe/TURN/STUN call sites in `synergos-core/src` |
| `mesh/stun.rs` | Minimal RFC 5389 Binding Request client (XOR-MAPPED-ADDRESS only) | Dead in production (no callers outside crate) |
| `mesh_ip.rs` | Detect Cloudflare WARP/Mesh CGNAT `192.0.2.10/12` IPv4 on local NICs | Helper (used by heartbeat/peer-info in core, out of scope) |
| `promotion.rs` | Startup probe: global IPv6 on NIC, UPnP IGD discover, `cloudflared` on PATH + token configured -> `FullNode` vs `RelayOnly` | Live: `daemon.rs:106-128` decides `effective_relay_only` |
| `conduit/mod.rs` | Per-peer route registry, route detection (IPv6 probe + tunnel health), priority connect Direct->Tunnel->Relay, route migration | Effectively dead: `register_peer`/`connect_peer` never called by core; `try_migrate_route` is called every 60 s (`daemon.rs:621-638`) but the `peers` map is never populated, so it always returns `PeerNotFound` (inference from code: `conduit/mod.rs:223-226`) |
| `config.rs` | `NetConfig` and all sub-configs, validation | Live |
| `types.rs`, `error.rs` | IDs, `Route`/`RouteKind`/`ConnectionState`, error enum | Live |
| `lib.rs` | module exports, `SynergosNet` aggregate (documented as not instantiated by the daemon, `lib.rs:56-58`), `NetEventHandler` trait (`lib.rs:44-54`, stored as `#[allow(dead_code)]` at `lib.rs:70-71`) | Aggregate unused |

### 1.2 Data flow (what enters, what leaves)

Outbound connection (client role), as driven by `peer_bootstrap.rs`:
1. Caller obtains `expected_peer_id` and a `SocketAddr` from the peer-info HTTP servlet (`peer_bootstrap.rs:151-152`; the servlet URL may be plain `http://`, `peer_bootstrap.rs:196`).
2. `QuicManager::connect` (`quic/mod.rs:205-260`): builds a one-off `quinn::ClientConfig` whose rustls verifier is `PeerPinningVerifier::new(expected_peer_id)` (`quic/mod.rs:465-486`), calls `endpoint.connect_with(client_config, addr, server_name)` (`quic/mod.rs:224-226`).
3. TLS 1.3 handshake: `PeerPinningVerifier::verify_server_cert` extracts the ed25519 SPKI from the server's self-signed cert and requires `blake3(pubkey)[..20] == expected_peer_id` (`quic/verifier.rs:38-66,70-80`); the handshake `CertificateVerify` signature is checked by ring (`quic/verifier.rs:96-108`), restricted to `SignatureScheme::ED25519` (`quic/verifier.rs:110-114`).
4. After handshake, client opens a bidi stream and sends `HLO1 || u32be len || msgpack(SignedHello)` (`quic/hello.rs:108-130`), then inserts `QuicConnection` keyed by `expected_peer_id` (`quic/mod.rs:249-258`). If hello send fails the connection is closed (`quic/mod.rs:242-247`).

Inbound connection (server role), as driven by `daemon.rs:723-760`:
1. `QuicManager::accept` (`quic/mod.rs:362-407`): `endpoint.accept().await` -> `incoming.await` (handshake, no client cert: `quic/mod.rs:492`) -> `hello::recv_hello` with a 5 s timeout (`quic/hello.rs:134-173`, `HELLO_TIMEOUT` at `quic/hello.rs:31`).
2. `SignedHello::verify` (`quic/hello.rs:77-103`) checks lengths, `peer_id == blake3(public_key)[..20]`, and the ed25519 signature over `signing_bytes()`.
3. On success the connection is inserted into `connections` keyed by the Hello's `peer_id` (`quic/mod.rs:392-400`) and returned as `AcceptedConnection { peer_id, remote_addr, connection }` (`quic/mod.rs:402-406`). The daemon then spawns `dispatch_peer_streams`, which attributes every subsequent bidi stream on that connection to `sender = acc.peer_id` and routes by a 4-byte magic (`daemon.rs:733-747,766-800`).

Other flows:
- Streams: `open_stream` clones the `quinn::Connection` out of the DashMap and calls `open_bi()` (`quic/mod.rs:277-308`). `send_control` opens a fresh bidi stream per call and writes raw bytes with no framing (`quic/mod.rs:311-322`).
- Promotion: `NetCapabilities::detect` runs three probes in parallel under a per-probe timeout (`promotion.rs:60-85`); `recommended_mode()` (`promotion.rs:88-94`) -> daemon sets `effective_relay_only` (`daemon.rs:106-128`).
- Shutdown: `SynergosNet::shutdown` order is conduit -> quic -> tunnel -> mesh GC (`lib.rs:117-134`); the daemon mirrors this itself.

### 1.3 Crates/modules this slice calls
quinn (`Endpoint`, `Connection`, `TransportConfig`), rustls 0.23 (`ClientConfig::builder().dangerous().with_custom_certificate_verifier`, `ServerConfig::builder().with_no_client_auth().with_single_cert`), rustls `ring` provider, rcgen (self-signed cert from PKCS#8), x509-parser (SPKI extraction), ed25519-dalek (keys, pkcs8, sign/verify), blake3, aes-gcm, argon2, rand (`OsRng`, `thread_rng`), rmp-serde (Hello encoding), hickory-resolver (DoH + system DNS), tokio (process, net, time, sync), socket2 (dual-stack bind), if-addrs, igd-next, which, dashmap, uuid, hex, tracing. Internal: `crate::identity`, `crate::types`, `crate::error`, `crate::config`.

Dead dependencies observed: `prost`/`prost-types` are declared (`Cargo.toml:19-20`) and `prost-build` is a build-dependency (`Cargo.toml:60-61`) but there is no `synergos-net/build.rs` and no `prost` usage in `synergos-net/src` (grep). `tokio-tungstenite` (`Cargo.toml:41`) is for the relay module (outside this slice).

---

## 2. Key types, APIs, wire formats

### 2.1 Identity
- `Identity { signing: SigningKey, verifying: VerifyingKey, peer_id: PeerId }` (`identity.rs:71-75`).
- `PeerId` is a `String` newtype (`types.rs:8`); for key-derived peers it is `hex(blake3(pubkey)[..20])` = 40 hex chars (`identity.rs:37,339-343`). `PeerId::generate()` (uuid v4, `types.rs:15-17`) and `PeerId::new(any string)` also exist, so a `PeerId` is not guaranteed to be key-bound by type.
- Key file formats (`identity.rs:11-13`): (a) 32 raw bytes plaintext; (b) `b"SYNERGOS-ID-V1\0"` (15) || salt (16) || nonce (12) || ciphertext (32) || GCM tag (16) = 91 bytes. Format is auto-detected by magic (`identity.rs:227-243`).
- KDF: `Argon2::default()` (Argon2id, library default params) `hash_password_into(passphrase, salt, 32)` (`identity.rs:320-329`). Cipher: AES-256-GCM with random 12-byte nonce and 16-byte salt from `OsRng` (`identity.rs:249-256`).
- Passphrase source: env var `SYNERGOS_IDENTITY_PASSPHRASE` (`identity.rs:40,222-224`); encryption is opt-in only when it is set and non-empty.
- Default path per OS (`identity.rs:189-219`): Linux `$XDG_STATE_HOME/synergos/identity.key` -> `~/.local/state/synergos/identity.key` -> fallback `/tmp/synergos-identity.key`; macOS `~/Library/Application Support/Synergos/identity.key`; Windows `%APPDATA%/Synergos/identity.key` -> fallback `./Synergos/identity.key`.
- API: `load_or_generate`, `generate`, `load_from_file`, `save_to_file`, `sign(&[u8]) -> [u8;64]`, `to_pkcs8_der`, `from_pkcs8_der`, free fns `peer_id_from_public_bytes`, `verify(pk, msg, sig)` (`identity.rs:83-183,335-355`).

### 2.2 QUIC / TLS
- `QuicConfig { max_concurrent_streams: u32, idle_timeout_ms: u64, max_udp_payload_size: u16, enable_0rtt: bool, listen_addr: Option<SocketAddr> }` (`config.rs:591-607`).
- Server cert: rcgen self-signed X.509 over the identity's ed25519 key, SANs `["synergos", <peer_id>]`, CN = `<peer_id>` (`quic/mod.rs:536-565`). Built once per `bind`.
- Client TLS: custom verifier, no client auth, no ALPN set (`quic/mod.rs:468-471`). Server TLS: `with_no_client_auth`, single cert, no ALPN set (`quic/mod.rs:491-494`).
- Transport config (both roles): `max_concurrent_bidi_streams = max_concurrent_streams`, `max_idle_timeout = idle_timeout_ms` with fallback 30 000 ms on conversion failure (`quic/mod.rs:477-484,496-502`). Nothing else is tuned (no keepalive, no receive windows, no uni-stream limit, no datagram config, no `Incoming::retry`, no `max_incoming`/buffer limits).
- Endpoint: `quinn::EndpointConfig::default()`, UDP socket bound via `bind_udp_dual_stack` (IPV6_V6ONLY=false for v6 addresses, `quic/mod.rs:583-594`).
- Registry: `connections: DashMap<PeerId, QuicConnection>`; `QuicConnection { peer_id, remote_addr, active_streams, state, rtt_ms, connection: Option<quinn::Connection> }` (`quic/mod.rs:49-62,136-148`). Public view `QuicConnectionInfo` (`quic/mod.rs:568-575`).
- `StreamType::{Control, Data{transfer_id}}` (`quic/mod.rs:40-45`) is only used for a debug log line (`quic/mod.rs:301`); streams are returned as a raw `(SendStream, RecvStream)` tuple, not the `QuicStream` wrapper of DESIGN §3.5.
- `SpeedTestResult`, `CalibratedParams`, `ConnectionCalibrator::calibrate` (`quic/mod.rs:77-130`) are pure functions; no speed test is executed anywhere in this crate.

### 2.3 HLO1 Hello wire format (`quic/hello.rs`)
```
bidi stream (client-initiated, first stream after handshake expected by server)
  b"HLO1"                       4 bytes magic            hello.rs:28,117-119,145-149
  u32 big-endian length         <= 4096                  hello.rs:120-123,155-158
  rmp-serde (MessagePack) of SignedHello { hello: Hello { peer_id: String, public_key: Vec<u8>(32), ts_ms: u64 }, signature: Vec<u8>(64) }
signing_bytes = peer_id_utf8 || 0x00 || public_key(32) || ts_ms (u64 little-endian)   hello.rs:42-49
```
Server reads exactly one such stream, verifies, and never replies (no ack; `hello.rs:109` comment). `ts_ms` is carried but never checked (`hello.rs:77-103`). There is no server nonce and no binding to the TLS session.

### 2.4 Control stream
`send_control(peer, data)` = `open_bi` + `write_all(data)` + `finish()` with no length prefix or magic (`quic/mod.rs:311-322`). `probe_rtt` sends `b"ping"` `count` times on fresh streams and records only the local elapsed time of open+write+finish (`quic/mod.rs:442-457`).

### 2.5 STUN (`mesh/stun.rs`)
20-byte header (type 0x0001 Binding Request / 0x0101 Success, length, magic cookie 0x2112A442, 96-bit tx id from `rand::thread_rng`) (`mesh/stun.rs:31-36,73-88`). Only XOR-MAPPED-ADDRESS (0x0020) is parsed; families 0x01/0x02 (`mesh/stun.rs:124-125,138-174`). Single request, single 1500-byte recv under timeout, no retransmission (`mesh/stun.rs:60-70`).

### 2.6 Routes / peers (`types.rs`)
`Route::{Direct{addr: SocketAddrV6, fqdn}, Tunnel{tunnel_id, hostname}, Relay{server_url, room_id}}` (`types.rs:148-158`), `RouteKind` (`types.rs:162-166`), `ConnectionState::{Discovered, Connecting, Connected{rtt_ms, route}, Disconnected{reason}}` (`types.rs:180-185`), `PeerEndpoint { peer_id, display_name, routes, state }` (`types.rs:189-194`). `PeerConnectionStatus` (`conduit/mod.rs:447-454`).

### 2.7 Errors (`error.rs:6-45`)
`SynergosNetError::{Quic, Tunnel, DnsResolution, Turn, ChainFork, PeerNotFound, Transfer, Config, Io, Serialization, Gossip, Identity}` all `String`-payload except `Io`/`ChainFork`. Peer-auth failures on connect are classified by substring matching on the quinn error text (`"PeerPinning"`, `"UnknownIssuer"`) (`quic/mod.rs:228-236`).

### 2.8 Tunnel process contract (`tunnel/mod.rs`)
Command: `cloudflared tunnel --no-autoupdate run [--hostname <hostname>]`, stdout/stderr piped (`tunnel/mod.rs:336-344`). `tunnel_id` is a locally generated `cf-<uuid4>` (or `sim-<uuid4>`), not the Cloudflare tunnel id (`tunnel/mod.rs:325,350`). `TunnelState::{Idle, Starting, Active{tunnel_id, uptime_secs}, Error{message}}` (`tunnel/mod.rs:25-34`).

---

## 3. Security controls PRESENT

Authentication / identity
- C1. Server-identity pinning at TLS handshake: `PeerPinningVerifier::verify_server_cert` requires the ed25519 SPKI (OID 1.3.101.112) of the end-entity cert to hash to `expected_peer_id` (`quic/verifier.rs:19,38-66,70-80,120-130`). Key possession is proven because `verify_tls13_signature` delegates to ring with the cert's SPKI (`quic/verifier.rs:96-108`), and only `ED25519` is an accepted scheme (`quic/verifier.rs:110-114`). Legacy "skip verify" path is documented as removed (`quic/mod.rs:19`).
- C2. Client identity via signed Hello: server derives the client's PeerId only from a `SignedHello` whose `peer_id == blake3(public_key)[..20]` and whose ed25519 signature verifies (`quic/hello.rs:77-103`); on failure or 5 s timeout the connection is closed with `b"hello rejected"` (`quic/mod.rs:384-390`, `quic/hello.rs:31,170-172`).
- C3. Client always sends Hello right after handshake and aborts the connection if it cannot (`quic/mod.rs:240-247`).
- C4. `connect` requires an explicit `expected_peer_id`; there is no default `ClientConfig` on the endpoint, so a connect without pinning is impossible through this API (`quic/mod.rs:163-167,205-210`).
- C5. PeerId derivation is deterministic from the public key with a 160-bit truncated BLAKE3 (`identity.rs:335-343`), and `peer_id_from_cert` (test-only) shows the same derivation from certs (`quic/verifier.rs:135-144`).

Crypto
- C6. Keys generated with `OsRng` (`identity.rs:97-100`); salt/nonce from `OsRng` (`identity.rs:249-252`); STUN tx id from `thread_rng` (`mesh/stun.rs:73-79`).
- C7. At-rest key encryption (opt-in): AES-256-GCM with Argon2id-derived key, random salt and nonce, length checks on both encode and decode (`identity.rs:245-318`). Wrong passphrase yields `DecryptFailed` (`identity.rs:301-306`, test `identity.rs:412-418`).
- C8. rustls crypto provider is explicitly installed once (`quic/mod.rs:519-525`); TLS 1.3 via quinn/rustls (platforms.md:66).
- C9. 0-RTT is off by default with a replay-risk comment (`config.rs:710-712`) (note: the flag is never consumed, see W-19).

Validation / limits
- C10. Hello body limited to 4096 bytes and exact-length reads (`quic/hello.rs:155-163`); public key must be 32 bytes and signature 64 bytes (`quic/hello.rs:78-87`); first-stream magic must be `HLO1` (`quic/hello.rs:145-149`).
- C11. Hostname passed to `cloudflared --hostname` is validated against `[a-zA-Z0-9.-]`, length <= 253, no leading/trailing `.`/`-`, no `..` (`tunnel/mod.rs:309-314,360-376`; tests `tunnel/mod.rs:408-431`), preventing argument injection such as `--flag`.
- C12. Arguments to `cloudflared` are passed as separate argv items (no shell) (`tunnel/mod.rs:336-341`).
- C13. STUN response parsing checks minimum length, message type, magic cookie, transaction id, body truncation, and attribute bounds (`mesh/stun.rs:90-136,138-174`).
- C14. IPv6 probe refuses unspecified/loopback/multicast/link-local targets (S12 note) (`mesh/mod.rs:184-194`).
- C15. `HistoryConfig.root` relative-path traversal rejection and `HookDef` validation (event whitelist, command length <= 32 KiB, timeout 1..=86400 s, patterns <= 1024 bytes without control chars) (`config.rs:130-169,433-463`); `NetConfig::validate` is called at daemon boot (`daemon.rs:65-68`).
- C16. Config secure defaults: `allow_simulation=false` (`config.rs:694`), `hooks.allow_project_hooks=false` (`config.rs:101-104`), `force_relay_only=false`/`auto_promote=true` (`config.rs:752-753`), QUIC listens on an ephemeral port unless configured (`config.rs:713`, `daemon.rs:88-93`).
- C17. DoH with `opts.validate = true` (DNSSEC) across two providers before falling back (`mesh/mod.rs:329-369`).

File-system / process
- C18. Key file chmod 0600 on Unix after write (`identity.rs:132-137`); parent dirs created (`identity.rs:87-89`).
- C19. Tunnel supervisor: exponential backoff `restart_base_ms * 2^min(failures,20)` capped at `restart_max_ms` with saturating arithmetic (`tunnel/mod.rs:243-246`); supervisor aborted and child killed on `stop()` (`tunnel/mod.rs:151-161`).
- C20. `cloudflared` is located on PATH with a std-only resolver honoring Windows PATHEXT (`tunnel/mod.rs:382-406`).

Concurrency / atomicity
- C21. DNS cache uses DashMap entry API to avoid get/drop/remove TOCTOU (`mesh/mod.rs:110-127`).
- C22. `open_stream` releases the DashMap read guard before re-acquiring `get_mut` to avoid same-shard deadlock (`quic/mod.rs:282-294`); `accept` clones the endpoint and drops the `RwLock` guard before awaiting (`quic/mod.rs:363-368`); Conduit `connect_peer`/`try_migrate_route` copy data out of guards before awaiting (`conduit/mod.rs:136-148,228-243`).
- C23. One-time crypto provider install via `Once` (`quic/mod.rs:519-525`).

Logging hygiene
- C24. `PeerId::short()` (12 chars) is used in most info logs to avoid full-ID leakage (S24) (`types.rs:19-28`; e.g. `quic/mod.rs:216-220,273`, `conduit/mod.rs:160,185`); `redact_path` helper exists for path redaction (`types.rs:225-239`). cloudflared stdout is logged at `debug`, stderr at `warn` under target `cloudflared` (`tunnel/mod.rs:184-196`).

---

## 4. Weaknesses, TODOs, missing controls FOUND

Severity guesses are mine. "Exploitable" means a concrete attacker path exists in this code; "design gap" means the control is absent or the feature is stubbed.

### Security theme

- W-1 [critical, exploitable] HLO1 is replayable: Hello has no server nonce, no TLS channel binding, and `ts_ms` is never checked (`quic/hello.rs:42-49,77-103`). A node that receives a victim's `SignedHello` as a server (every node a victim connects to) can connect to any third node and present that same blob; `accept()` will register the attacker's connection under the victim's PeerId (`quic/mod.rs:384-400`) and `dispatch_peer_streams` attributes all streams to the victim (`daemon.rs:733-747`). Redesign: bind Hello to the session (TLS exporter / server-sent nonce challenge-response) or use mTLS with client certs; reject stale `ts_ms`.
- W-2 [high, exploitable] Any Internet host with a freshly generated key can hold an authenticated inbound connection: there is no project membership / allowlist / token check at accept time (`quic/mod.rs:362-407`); `Handshake.project_token` from DESIGN §3.6 (`DESIGN.md:522`) does not exist. Combined with `max_concurrent_streams=100` per connection (`config.rs:707`), a stranger gets 100 concurrent streams into DHT/gossip/transfer dispatch. Redesign: authorization gate at accept (project allowlist, signed invite/capability, Cernere-issued credential), plus per-peer and per-IP limits.
- W-3 [high, exploitable DoS] `accept()` serializes handshake + Hello wait (up to 5 s) on the single accept loop (`quic/mod.rs:370-390`, `daemon.rs:723-760`). A client that completes QUIC handshake and never sends HLO1 stalls all other inbound accepts for `HELLO_TIMEOUT`; repeated, it blocks the node from ever accepting. Redesign: spawn per-incoming task for handshake+hello, bound concurrent pending handshakes, use `Incoming::retry()` for address validation.
- W-4 [high, exploitable] Connection-registry hijack: `accept()` inserts under the Hello PeerId with plain `insert`, overwriting any existing entry (`quic/mod.rs:400`); together with W-1 (or simply a peer reconnecting) this swaps the `quinn::Connection` that `open_stream`/`raw_connection` hand out for that PeerId (`quic/mod.rs:277-308,339-343`), redirecting the node's outbound streams for the victim to the attacker. `connect()` has the same overwrite (`quic/mod.rs:258`). Redesign: multi-connection per peer with explicit selection, or reject/close duplicates with a policy.
- W-5 [high, design gap] No inbound rate limiting, no per-IP connection cap, no bound on `connections` size, no `ServerConfig` tuning (`max_incoming`, incoming buffer sizes, retry tokens) (`quic/mod.rs:488-511`). Unlimited distinct PeerIds (cheap to generate) grow the DashMap and quinn state unboundedly. Redesign: connection admission control, retry/address validation, memory budgets.
- W-6 [high, exploitable, cross-subsystem] Trust anchor for `expected_peer_id` is whatever the caller passes; the bootstrap path accepts plain `http://` peer-info URLs (`peer_bootstrap.rs:196`) and uses the advertised peer_id when none was expected (`peer_bootstrap.rs:151-152`). An on-path attacker on the bootstrap fetch substitutes their own peer_id+address and the pinning "succeeds" against the attacker. Redesign: pin expected PeerId from an out-of-band invite / Cernere-signed record; require https or signed peer-info.
- W-7 [medium, exploitable locally] Secrets leak into child process environments: `tokio::process::Command::new("cloudflared")` does not call `env_clear()` or `env_remove` (`tunnel/mod.rs:336-347`), so `SYNERGOS_IDENTITY_PASSPHRASE` (`identity.rs:40`) and the control node key env (`config.rs:484-486,503-505`) are inherited by the third-party binary (and visible in `/proc/<pid>/environ` of that process). Redesign: pass secrets via Infisical/agent, strip env for children, read passphrase from a file descriptor or keyring.
- W-8 [medium, design gap] Identity key at rest is plaintext by default; encryption only when an env var is set (`identity.rs:15-17,125-130`). A plaintext file is accepted even when a passphrase is configured and is never migrated (`identity.rs:227-243`). Not secure-by-default. Redesign: default to OS keyring / encrypted store; refuse plaintext when passphrase configured; provide rotation tooling.
- W-9 [medium, exploitable locally] Key file write is non-atomic and permissions are applied after the write (`identity.rs:131-137`): a window exists where the key is created with umask permissions; no `O_EXCL`; `load_or_generate` has an `exists()`-then-write TOCTOU (`identity.rs:84-93`). On Windows no ACL is set at all (`identity.rs:132` is `#[cfg(unix)]`). Linux fallback path is world-shared `/tmp/synergos-identity.key` when `HOME` is unset (`identity.rs:198`), Windows fallback is `./Synergos/identity.key` (`identity.rs:216`). Redesign: `OpenOptions::mode(0o600)` + temp file + rename; Windows DACL; fail instead of falling back to /tmp.
- W-10 [medium, design gap] Hello signature has no domain separation tag (`quic/hello.rs:42-49`); the same identity key signs gossip/chain payloads elsewhere. Cross-protocol signature confusion is possible if another message type can be shaped as `peer_id||0||pk||u64`. Redesign: prefix a context string (e.g. `b"synergos/hello/v1"`) and version byte to every signed structure.
- W-11 [medium, design gap] No ALPN is configured on either side (`quic/mod.rs:468-471,491-494`), violating RFC 9001 §8.1's requirement and leaving no protocol/version negotiation. Redesign: set `alpn_protocols = [b"synergos/1"]` and reject mismatches.
- W-12 [medium, design gap] No TLS client certificate (`quic/mod.rs:492`), so client authenticity rests entirely on HLO1 (see W-1). `docs/operations-readiness.md:10` claims "QUIC 相互認証 (S1) ✅". Redesign: mTLS with ed25519 client certs pinned the same way, HLO1 then only carries metadata.
- W-13 [medium, exploitable] Route data from peers is used unvalidated: `Route::Direct.addr` is dialed directly and `fqdn` becomes the TLS `server_name` (`conduit/mod.rs:388-397`). A malicious DHT/gossip record can make every node send QUIC Initials (>=1200 B each) to a victim address (reflection/scan), and the attacker-chosen SNI appears in plaintext ClientHello. (Conduit is currently dead, but `peer_bootstrap` has the same shape with servlet-provided addresses.) Redesign: validate/deny private/reserved targets, rate-limit dials per advertised route, fixed SNI.
- W-14 [low, design gap] `probe_ipv6` SSRF guard omits unique-local (fc00::/7), IPv4-mapped (`::ffff:a.b.c.d`, which can reach 127.0.0.1 or 192.0.2.10), site-local and documentation ranges (`mesh/mod.rs:186-194`), inconsistent with `promotion.rs:144-163`. Impact is one UDP byte, so low. Redesign: shared address-classification helper used by both.
- W-15 [low, design gap] DNS downgrade: when DoH (DNSSEC) fails for any reason, resolution silently falls back to system DNS without DNSSEC (`mesh/mod.rs:154-175,372-388`), so an attacker who blocks DoH forces unauthenticated answers. Mitigated by PeerId pinning (wrong address -> failed connect, i.e. DoS only). Redesign: make fallback a policy knob; log at warn.
- W-16 [low] Full PeerIds are logged in several places contrary to the S24 policy: `conduit/mod.rs:125-129,251-256,278`, `quic/mod.rs:301`. Redesign: lint/grep in CI for `{}` with `peer_id` in logs.
- W-17 [low, design gap] Peer-auth failure detection relies on substring matching of error text (`quic/mod.rs:228-236`); a quinn/rustls message change silently reclassifies auth failures as generic `Quic` errors (the test accepts either, `tests/quic_peer_auth.rs:164-166`). Redesign: match on typed `ConnectionError::TransportError`/rustls error variants.
- W-18 [low, design gap] `PeerPinningVerifier` ignores validity period, intermediates, OCSP and `server_name` (`quic/verifier.rs:70-80`) and does not verify the cert's own self-signature. Acceptable under pure pinning but means there is no notion of revocation or expiry anywhere in the transport. Redesign: add a revocation list / key-rotation record (signed by old key) checked at accept and connect.
- W-19 [low, design gap] `QuicConfig.enable_0rtt` and `max_udp_payload_size` are parsed but never applied (no references in `synergos-net/src` outside `config.rs`; `quic/mod.rs:465-511` sets neither). Setting `enable_0rtt=true` is a silent no-op (safe today), but the knob misleads operators. Redesign: remove or implement with explicit replay-safe stream rules.
- W-20 [low, design gap] `TunnelConfig.api_token_ref` and `TurnServerConfig.credential_ref` are never resolved against any secret store (only `is_empty()` is checked at `daemon.rs:112`); `cloudflared` is run with ambient credentials (`tunnel/mod.rs:336-341`). Redesign: wire to Infisical per org rules, pass `--token`/credentials file explicitly, never ambient.
- W-21 [low, design gap] cloudflared stdout/stderr are read with `BufReader::lines()` without a line-length cap (`tunnel/mod.rs:184-196`), contradicting the module doc "最大バッファサイズで切る" (`tunnel/mod.rs:9`). A runaway child can grow memory. Redesign: `take()`/bounded reader.
- W-22 [low, design gap] `Identity::generate`/`PeerId::new` allow non-key-bound PeerIds (`types.rs:11-17`); nothing at the type level distinguishes a verified PeerId from a user-typed string. Redesign: newtype `VerifiedPeerId` produced only by `hello::verify`/verifier.

### Transfer theme

- W-23 [high, design gap] Conduit is dead in production: `register_peer`/`connect_peer` have no call sites in `synergos-core/src`; `try_migrate_route` runs every 60 s (`daemon.rs:621-638`) on an empty `peers` map and returns `PeerNotFound` (`conduit/mod.rs:223-226`). DESIGN §3.2's route discovery, monitoring, migration and keepalive are therefore not operating. Redesign: either delete Conduit or make it the single owner of peer connections used by bootstrap/DHT.
- W-24 [high, design gap] No keepalive anywhere: `_keepalive_interval` is unused (`conduit/mod.rs:73,84`), `TransportConfig::keep_alive_interval` is never set (`quic/mod.rs:477-484,496-502`; grep `keep_alive` empty), so idle connections die after `idle_timeout_ms=30000` (`config.rs:708`). Users see peers "disconnect" after 30 s of silence and incur a full reconnect + Hello. Redesign: set `keep_alive_interval < idle_timeout/2` on both configs.
- W-25 [high, design gap] Fake success paths: `try_connect_routes` returns `Ok(RouteKind::Tunnel)` if the tunnel process is merely alive (`conduit/mod.rs:409-419`) and `Ok(RouteKind::Relay)` without doing anything (`conduit/mod.rs:422-428`, relay-only branch `conduit/mod.rs:371-385`). `try_migrate_route` disconnects the live QUIC connection before attempting the "better" route (`conduit/mod.rs:259-263`), so a false Tunnel/Relay success destroys the only real data path. Redesign: a route is "connected" only when a verified QUIC session exists over it.
- W-26 [medium, design gap] `probe_ipv6` declares `reachable=true` as soon as one UDP byte is sent (`mesh/mod.rs:203-223`); ICMP unreachable arrives later and is ignored, so every routable address is "reachable" with ~0 ms RTT. DESIGN §15.5 specifies ICMPv6 + QUIC handshake (`DESIGN.md:1750-1754`). Redesign: probe = real QUIC handshake with pinning under a short timeout.
- W-27 [medium, design gap] `probe_rtt` measures local stream-open+write time, not a round trip (`quic/mod.rs:442-457`); `rtt_ms` in Conduit is hard-coded 0 (`conduit/mod.rs:155,270`); `TunnelHealth.rtt_ms` is `Some(0)` (`tunnel/mod.rs:279`). Calibration/peer selection built on these numbers would be meaningless. Only `connection.rtt()` at connect/accept time is real (`quic/mod.rs:238,380`). Redesign: use quinn's `Connection::rtt()`/`stats()` continuously; implement Ping/Pong.
- W-28 [medium, design gap] `send_control` has no framing/magic, and the daemon dispatcher expects a 4-byte magic (`quic/mod.rs:311-322` vs `daemon.rs:786-792`); `b"ping"` from `probe_rtt` would be dropped as unknown magic. `active_streams` is incremented but never decremented (`quic/mod.rs:303-305`). Redesign: a typed control protocol (Ping/Pong/Bye) with length-prefixed frames and stream accounting.
- W-29 [medium, design gap] Promotion misclassifies reachability: UPnP "available" means an IGD answered discovery, no port mapping is created (`promotion.rs:166-196`), and a global IPv6 on a NIC does not imply inbound firewall pass-through (`promotion.rs:114-131,88-94`). Nodes advertise themselves as FullNode while unreachable, so peers waste handshake timeouts (up to `idle_timeout_ms`) before falling back. Redesign: add a reflective reachability test (STUN + inbound self-connect via a relay) and actually create/renew UPnP mappings.
- W-30 [medium, design gap] Cloudflare Tunnel cannot carry inbound UDP/QUIC (acknowledged in `config.rs:39-42`), yet `RouteKind::Tunnel` is modeled as a QUIC route (DESIGN §3.2 "QUIC via Cloudflare Tunnel") and `promotion.rs:199-206` counts it toward FullNode. In production the tunnel is never even started (no `tunnel.start()` in core). Redesign: drop Tunnel as a QUIC route or define a TCP/WebSocket fallback transport that cloudflared can actually proxy.
- W-31 [medium, design gap] Sequential, untimed route attempts: each `Route::Direct` try waits for the full QUIC handshake/idle timeout (`conduit/mod.rs:388-406`; no `tokio::time::timeout`), contradicting DESIGN §15.5 "< 3 s fallback" (`DESIGN.md:1768`). `probe_fqdn` probes addresses sequentially (`mesh/mod.rs:253-259`). Redesign: happy-eyeballs style parallel dial with per-attempt timeouts.
- W-32 [low, design gap] DNS: new `TokioAsyncResolver` per lookup (`mesh/mod.rs:344,376-377`) means a fresh DoH TLS session every time; fixed 300 s cache ignoring record TTL (`mesh/mod.rs:144,165`); no negative caching. Redesign: shared resolver, honor TTLs.
- W-33 [low, design gap] STUN client sends one request with no RFC 5389 retransmission schedule (`mesh/stun.rs:57-70`), no FINGERPRINT/MESSAGE-INTEGRITY, and `stun_servers` config is never read (`config.rs:571`). TURN `allocate_turn`/`refresh_turn` are stubs that mark sessions `Active` without network I/O (`mesh/mod.rs:263-292`). Redesign: real STUN/TURN (or ICE) or remove from config/DESIGN.
- W-34 [low, design gap] `connect()` holds the endpoint `RwLock` read guard across the whole handshake+Hello await (`quic/mod.rs:211-247`), so `shutdown()`/`bind()` (write lock, `quic/mod.rs:186-187,415`) block behind slow connects. Redesign: clone the endpoint and drop the guard as `accept()` does.
- W-35 [low, design gap] `PeerPinningVerifier::new` allocates a fresh `CryptoProvider` per connect (`quic/verifier.rs:29-34`) and `build_client_config_for` rebuilds rustls config per connect (`quic/mod.rs:465-486`). Minor CPU/alloc overhead per dial. Redesign: share one provider; cache configs per expected peer.
- W-36 [low, design gap] `QuicConfig.max_concurrent_streams` only bounds bidi streams; uni streams, receive windows and datagrams use quinn defaults (`quic/mod.rs:477-484,496-502`); no per-stream size cap at this layer (the daemon applies 1 MiB only to extension streams, `daemon.rs:775-777`). Redesign: explicit flow-control budget tied to transfer chunk sizes.

### AI/game asset production theme

- W-37 [medium, design gap] No transport-level notion of connection priority/QoS or stream classes: `StreamType` is decorative (`quic/mod.rs:40-45,301`), `StreamAllocationConfig` ratios (`config.rs:634-656`) are validated but not consumed by the transport (grep `stream_allocation.` in core/net empty). Large binary assets (multi-GB game/AI artifacts) and interactive control traffic share undifferentiated streams. Redesign: stream classes with per-class concurrency and bandwidth budgets enforced at `open_stream`.
- W-38 [medium, design gap] No connection resumption or 0-RTT (W-19) and no connection migration handling: a laptop changing networks must redo handshake + Hello and loses all in-flight streams; `remote_addr` is captured once (`quic/mod.rs:251,379`) and never refreshed. Redesign: enable quinn path migration awareness, session tickets with replay-safe policy.
- W-39 [low, design gap] Speed test/calibration from DESIGN §15.3 is only the pure `calibrate` function (`quic/mod.rs:106-130`); `SpeedTestConfig`, `PeerSelectionConfig`, `MonitorConfig` are parsed but unused by this crate (`config.rs:658-686`, grep empty). Bandwidth-aware peer selection for large asset distribution does not exist at this layer.

---

## 5. Performance characteristics

- Handshake cost: TLS 1.3 over QUIC (1 RTT) + one client-initiated bidi stream carrying ~200-300 bytes of msgpack Hello (`quic/hello.rs:108-130`); the server-side `accept()` does not return until Hello arrives, adding up to 1 RTT before dispatch starts; the client does not wait for an ack (`quic/hello.rs:109`). Server-side acceptance is strictly serial (`quic/mod.rs:362-407`, `daemon.rs:723-760`), so inbound connection throughput is bounded by handshake+Hello latency of the slowest client (<= 5 s each).
- Frame/chunk sizes: Hello <= 4096 B (`quic/hello.rs:155`); STUN recv buffer 1500 B (`mesh/stun.rs:64`); QUIC datagram size is quinn default (the `max_udp_payload_size=1452` knob is unused). Transport does no application framing itself.
- Concurrency: `max_concurrent_bidi_streams = 100` per connection (`config.rs:707`, `quic/mod.rs:478,497`); stream open is `open_bi()` on a cloned `quinn::Connection` outside any lock (`quic/mod.rs:284-299`). `DashMap` for connections, bandwidth estimates (`quic/mod.rs:141,147`), DNS cache, TURN sessions (`mesh/mod.rs:82-84`), Conduit peers (`conduit/mod.rs:57`): O(1) per op; `list_connections`/`shutdown`/`connected_peers` are O(n) clones (`quic/mod.rs:346-357,410-419`, `conduit/mod.rs:195-214`).
- Buffering: quinn defaults for receive/send windows (not tuned). cloudflared log piping spawns 2 tasks per child with unbounded line buffers (`tunnel/mod.rs:182-197`).
- Hashing/crypto: BLAKE3 for PeerId/NodeId/MessageId (`identity.rs:340`, `types.rs:43,128,138`); ed25519 sign/verify for Hello (1 verify per inbound connection); rcgen self-signed cert generation once per `bind` (`quic/mod.rs:169,536-565`); Argon2id default params once at startup (comment says ~100 ms, `identity.rs:21`); AES-256-GCM for 32-byte key.
- Round trips: connect = 1 RTT handshake + 0.5 RTT Hello send; `probe_rtt` = N stream opens with no round trip (`quic/mod.rs:445-450`); `send_control` = 1 new stream per message (`quic/mod.rs:312`).
- Disk I/O: identity read once at start (`identity.rs:111`), write once on generate; no other disk I/O in this slice.
- DNS: per-lookup resolver construction (`mesh/mod.rs:344`), 5 s DoH timeout per provider x 2 providers + system DNS = worst case > 10 s before a Direct route is even probed; cache 300 s.
- Promotion: three probes in parallel (`promotion.rs:65-72`), UPnP SSDP timeout 2 s (`promotion.rs:172-175`), daemon wraps with 3 s per probe (`daemon.rs:110-115`).
- Timers: daemon maintenance loop every 60 s copies RTT/bandwidth from `list_connections()` and attempts migration per connection (`daemon.rs:610-638`).
- Nothing O(n^2) found. `glob_match` is a DP O(|pattern| x |path|) (`config.rs:185-258`).

---

## 6. Config knobs and defaults (`config.rs`)

| Knob | Default | Consumed by | Secure/sane by default? |
|---|---|---|---|
| `quic.max_concurrent_streams` | 100 (`config.rs:707`) | `quic/mod.rs:478,497` | Reasonable; no per-peer global cap |
| `quic.idle_timeout_ms` | 30000 (`:708`) | `quic/mod.rs:479-483,498-502` | Sane, but no keepalive (W-24) |
| `quic.max_udp_payload_size` | 1452 (`:709`) | nothing | Dead knob |
| `quic.enable_0rtt` | false (`:712`) | nothing | Safe default, dead knob (W-19) |
| `quic.listen_addr` | None => `[::]:0` (`:713`, `daemon.rs:88-93`) | `daemon.rs:85-97`, dual-stack bind `quic/mod.rs:583-594` | Ephemeral port default; public nodes set fixed port |
| `tunnel.api_token_ref` | "" (`:692`) | only `is_empty()` for promotion (`daemon.rs:112`) | Not wired to a secret store (W-20) |
| `tunnel.hostname` | "" (`:693`) | `tunnel/mod.rs:309-314,339-341` | Validated (C11) |
| `tunnel.allow_simulation` | false (`:694`) | `tunnel/mod.rs:320-327` | Secure default |
| `tunnel.auto_restart` / `restart_base_ms` / `restart_max_ms` | true / 1000 / 60000 (`:695-697`) | `tunnel/mod.rs:235-248` | Sane |
| `mesh.doh_endpoint` | cloudflare (`:700`) | nothing (providers hardcoded `mesh/mod.rs:333-336`) | Dead knob |
| `mesh.dns_servers`, `mesh.turn_servers`, `mesh.stun_servers` | [] (`:701-703`) | nothing | Dead knobs |
| `mesh.probe_timeout_ms` | 3000 (`:704`) | `mesh/mod.rs:196,297` | Sane |
| `force_relay_only` | false (`:752`) | `daemon.rs:106-108`, `conduit/mod.rs:308-322,371-385` | Opt-in anonymization |
| `auto_promote` | true (`:753`) | `daemon.rs:109-126` | Probe is optimistic (W-29) |
| `peer_info_listen_addr` / `peer_info_advertised_url` / `quic_advertised_addr` / `bootstrap_urls` | None/None/None/[] (`:748-751`) | peer-info servlet in core (out of scope) | `http://` URLs allowed (W-6) |
| `control.*` | heartbeat disabled; key via env `SYNERGOS_NODE_KEY` (`:474-509`) | core | Env-var secret inherited by children (W-7) |
| `stream_allocation`, `speed_test`, `peer_selection`, `monitor` | 60/30/10, enabled/300 s/10, 0.7/0.3/60 s, 1000 ms/3600/1 s (`:727-746`) | ratio validation only (`:645-656,765`) | Dead knobs in this crate |
| `history.*`, `hooks.*`, `catalog.*` | disabled / disabled / 256,10 | core (out of scope) | Validated (C15) |
| env `SYNERGOS_IDENTITY_PASSPHRASE` | unset => plaintext key | `identity.rs:222-224` | Not secure-by-default (W-8) |

`NetConfig` is `#[serde(default)]` at the top level only; sub-sections must be written completely when present (`config.rs:5-9`). `validate()` covers stream ratios, history and hooks only (`config.rs:764-772`); nothing validates QUIC/mesh/tunnel values (e.g. `idle_timeout_ms=0`, `max_concurrent_streams=0`).

---

## 7. Test coverage observed

Tested (integration, `tests/`):
- `quic_peer_auth.rs`: connect succeeds with matching PeerId (`:39-66`); server learns client PeerId via HLO1 (`:68-97`); disconnect removes registry entry (`:99-127`); `open_stream` on unknown peer errors (`:129-139`); connect with mismatched PeerId is rejected with `Identity` or `Quic` error (`:141-173`).
- `quic_control_stream.rs`: `send_control` bytes reach a server `accept_bi` echo task (`:28-68`); `list_connections`/`get_connection` after connect (`:70-97`).
- `net_additional.rs`: `ConnectionCalibrator::calibrate` boundaries (`:27-74`); `Mesh::clear_dns_cache` / `cleanup_expired_sessions` on empty state (`:179-203`); the rest targets ledger/catalog/DHT (other subsystems).

Tested (unit, in-file): identity sign/verify, tamper, persistence, encrypted round trip, wrong passphrase, plaintext load, passphrase-required (`identity.rs:357-454`); verifier accept/reject/peer_id_from_cert (`quic/verifier.rs:146-199`); Hello round trip and tamper of pk/signature (`quic/hello.rs:179-202`); hostname validation (`tunnel/mod.rs:408-431`); STUN encode/parse/mismatch/mock server (`mesh/stun.rs:176-287`); IPv6 global classification, recommended_mode, detect no-panic (`promotion.rs:208-275`); mesh IPv4 range (`mesh_ip.rs:24-38`); config defaults/serde/hooks/glob (`config.rs:775-1064`); redact helpers (`types.rs:252-276`).

NOT tested:
- Hello replay / stale `ts_ms` / wrong first-stream magic / oversized Hello / Hello timeout on the server (no test opens a QUIC connection and withholds HLO1).
- Duplicate PeerId connection overwrite (W-4); many concurrent inbound connections (W-3/W-5).
- `accept()` behaviour when the client sends a non-HLO1 stream first.
- Conduit entirely (no test for `register_peer`, `connect_peer`, `try_migrate_route`, `detect_route`, relay-only branches).
- TunnelManager `start`/`stop`/supervisor/backoff (only `is_valid_hostname`).
- Mesh `resolve_fqdn`/`resolve_doh`/`probe_ipv6`/TURN stubs; STUN IPv6 XOR path and retransmission.
- `bind_udp_dual_stack` on Windows; `listen_addr` IPv6 bind.
- Idle timeout expiry / reconnect; `probe_rtt`; `update_bandwidth_estimate`.
- Identity file permission bits, atomicity, Windows path.
- Env-var tests in `identity.rs` mutate process env without serialization (`identity.rs:361-365`), so they are order-sensitive under parallel test execution.

---

## 8. Documented-vs-implemented deltas

| Doc claim | Reality |
|---|---|
| DESIGN §3.2 Conduit does RTT measurement, bandwidth estimation, route monitoring, transparent migration, keepalive (`DESIGN.md:344-352`) | Conduit unused by daemon; `rtt_ms` fixed 0 (`conduit/mod.rs:155,270`); no keepalive (`conduit/mod.rs:73,84`); migration disconnects first (`conduit/mod.rs:259`) |
| DESIGN §3.2 priority Direct -> "QUIC via Cloudflare Tunnel" -> Relay (`DESIGN.md:298-303`) | Tunnel route is a process-liveness check, no QUIC over it (`conduit/mod.rs:409-419`); `config.rs:39-42` states Cloudflare proxy does not pass UDP/QUIC; tunnel never started by daemon |
| DESIGN §3.3 `TunnelManager { quic_config }`, `TunnelState::Error { retry_at }` (`DESIGN.md:362-378`), "Tunnel 認証" | No `quic_config`; `Error { message }` only (`tunnel/mod.rs:33`); no token/auth handling |
| DESIGN §3.4 Mesh: TURN allocation/permission/channel binding, self-managed DNS servers, `TurnServer.credential` (`DESIGN.md:395-458`), TURN policy table (`:461-468`) | TURN is a stub (`mesh/mod.rs:263-292`); `dns_servers`/`doh_endpoint`/`turn_servers`/`stun_servers` unread; `credential_ref` unused |
| DESIGN §3.4 flow "DoH AAAA -> QUIC connect -> TURN fallback" (`DESIGN.md:416-423`) | DoH exists but no caller; no TURN fallback |
| DESIGN §3.5 `QuicStream { send, recv, stream_type }` (`DESIGN.md:474-490`) | Tuple return; `StreamType` only logged (`quic/mod.rs:277-308`) |
| DESIGN §3.6 protobuf `Envelope`/`Handshake{project_token,...}`/Ping/Pong/Bye (`DESIGN.md:492-578`) | Not implemented: no build.rs, prost unused; real handshake is msgpack HLO1 (`quic/hello.rs`); no project token; no Ping/Pong/Bye frames |
| DESIGN §15.3 speed test phases (Ping x5, 64 KiB x10, 4 parallel streams) and `SpeedTestResult{tested_at, route}` (`DESIGN.md:1583-1587,1614-1617`) | Only `calibrate()` exists (`quic/mod.rs:106-130`); struct lacks `tested_at`/`route` (`quic/mod.rs:79-90`); `speed_test` config unused |
| DESIGN §15.4 PeerSelector / bandwidth-aware selection (`DESIGN.md:1666-1732`) | Not in this crate; `peer_selection` config unused here |
| DESIGN §15.5 IPv6 probe = AAAA + ICMPv6 + QUIC handshake; tunnel probe = health + QUIC handshake; fallback < 3 s (`DESIGN.md:1750-1768`) | UDP one-byte send (`mesh/mod.rs:203-223`); tunnel probe = state check (`tunnel/mod.rs:273-285`); no timeout-bounded fallback |
| `docs/platforms.md:66` "QUIC は TLS1.3 + ring/aws-lc-rs" | Code pins `ring` (`Cargo.toml:16`, `quic/mod.rs:523`, `quic/verifier.rs:32`); aws-lc-rs is merely present in the lock |
| `docs/platforms.md:19` identity paths | Match `identity.rs:189-219` (plus undocumented `/tmp` and `.` fallbacks) |
| `docs/platforms.md:17` cloudflared on PATH required | True for `TunnelManager`, but daemon never starts it |
| `docs/operations-readiness.md:10` "QUIC 相互認証 (S1) ✅" | One-way TLS pinning + replayable HLO1 (W-1, W-12) |
| `tunnel/mod.rs:9` "stdout/stderr ... 最大バッファサイズで切る" | No cap implemented (`tunnel/mod.rs:184-196`) |
| `quic/mod.rs:359-361` accept is "土台" for mutual S1 "from the certificate" | Server derives PeerId from HLO1, not a client cert |
| `lib.rs:4` crate provides "Cloudflare Tunnel, IPv6 Direct/TURN" | TURN absent; Tunnel unused |

---

## 9. Open questions for designers

1. Which layer is the intended authorization boundary for inbound connections (project membership, invite tokens, Cernere identity)? Today nothing gates `accept()` beyond "has a valid key".
2. Should client authentication move to mTLS (ed25519 client certs pinned like the server) so HLO1 can be dropped, or should HLO1 be upgraded with a server nonce / TLS exporter binding? Either closes W-1; which fits the relay path?
3. Is Cloudflare Tunnel still a goal for the data path given it cannot proxy inbound UDP/QUIC (`config.rs:39-42`)? If so, what transport (WebSocket/HTTP2 over cloudflared) carries it, and should `RouteKind::Tunnel` be redefined?
4. Should Conduit be removed or become the single owner of peer connections (bootstrap, DHT-discovered peers, relay)? Who is responsible for reconnect and keepalive?
5. What are the target limits: max inbound connections, max pending handshakes, per-IP rate, per-peer stream budget, memory budget for quinn buffers for multi-GB asset transfer?
6. Key lifecycle: how are node keys rotated/revoked, and how is a PeerId bound to a Cernere principal or project membership record? Where is the revocation list consulted?
7. Secret handling: should `SYNERGOS_IDENTITY_PASSPHRASE`/`SYNERGOS_NODE_KEY` env vars be replaced by Infisical/agent or OS keyrings per org rules, and should child processes get a scrubbed env?
8. Is `ts_ms` in Hello meant to be enforced (clock-skew window)? If so, what window, and what happens on unsynchronized clocks?
9. Does any product scenario require 0-RTT? If not, remove the flag; if yes, which streams are replay-safe?
10. Which of the dead config knobs (`doh_endpoint`, `dns_servers`, `turn_servers`, `stun_servers`, `max_udp_payload_size`, `speed_test`, `peer_selection`, `monitor`, `stream_allocation`) should be implemented vs deleted before external users rely on them?
11. Should ALPN carry a protocol version so that future HLO2/stream-magic changes can be negotiated rather than breaking?
12. Is the `cloudflared tunnel run --hostname` invocation (`tunnel/mod.rs:337-341`) a valid CLI shape for named tunnels (unverified here), and what provides the tunnel credentials in production?
