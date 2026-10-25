# Synergos C++ migration

## Authorization and outcome

On 2026-10-25 neco requested the complete rewrite and migration, delegated to
Sol, including alignment with Cernere. This supersedes the 2026-10-17 immediate
build-memory milestone. Transport, daemon, custody/control, GUI and operations
are now in scope; build-memory measurements remain evidence, not completion.
The normative redesign is the three `redesign-*.md` documents in this directory.
Their approved authority decisions take precedence over the legacy contracts
below. Runtime cutover still requires review and the acceptance gate.

2026-10-17 neco requested the C++ rewrite, integration of reviewed work, build,
and execution. This authorizes implementation and verification; it does not
authorize treating missing behavior as a completed port or bypassing review.
Actio task: f18ad452-0315-4930-826a-d2935c51b3d1 (task-link rejected with
actio_task_out_of_scope; no substitute task or scope change).
Read-back confirms the task is in_progress under Sy and its registered team.
The Cc reference reader currently accepts cc-taskmd or concordia.taskflow.v3 tasks, while
this task was created with source cc-command. Do not relabel its provenance or
alter authorization checks to force a link; the link remains unregistered.

## Architecture and domain ownership

Use C++20, exception-safe RAII, explicit resource ownership, and CMake. The
language deviates from the default Rust client stack by explicit user request.
Keep existing module boundaries: network-transport (net, relay, IPC), mesh-core
(daemon, versions, history, hooks), opaque-custody, control-plane, desktop-gui,
admin-ui, and ars-plugin. Native source and tests live under each existing
module's `native/` directory, already covered by domain membership.

The purpose remains dependable asset sharing across Windows, Linux and macOS.
Preservation of stored data and authorization boundaries takes precedence over
cutover speed. No native layer may report success for an unsupported operation.

## Compatibility contracts

- IPC: `synergos-ipc/src/transport.rs`: four little-endian length bytes,
  MessagePack payload, maximum 1 MiB, incremental allocation. Preserve command,
  response and event variants and compact serde struct field ordering.
- Local IPC authorization: same-user checks, no network-exposed pipe/socket.
- Peer protocol: preserve protobuf schemas, QUIC ALPN, peer authentication,
  CRC/hash validation, transfer cancellation, and catalog exchange.
- Versioning: preserve `.synergos/manifest.json`, local high-water marks,
  restore/checkout semantics, tags and protected history. Reject path escapes,
  symlinks/junctions and version overflow before modifying data.
- Custody: SPEC-SY-opaque-custody, SPEC-SY-custody-durability,
  SPEC-SY-custody-acquisition remain normative. Preserve canonical receipt
  bytes, HMAC, capability method/object/expiry binding, exclusive publication,
  file/directory durability and tombstones. These are the legacy v1 contracts,
  not permission to issue new HMAC credentials in the redesigned runtime.
  New receipts/capabilities follow SPEC-SY-custody-receipt-v2 and
  SPEC-SY-custody-capability-v2; HMAC is verification-only for stored v1 rows
  during the bounded migration grace window.
- Authority: Cernere alone owns project membership (`read`, `publish`, `history`)
  and the node roster (SPEC-CR-membership-issuer, SPEC-CR-node-roster).
  Synergos control only relays, caches and verifies Cernere-signed membership
  LCTs, node cards, ProjectRoster, RosterStatus and OfflineLease. It holds no
  issuer key for those records and cannot change membership, ownership or
  reachability. Missing or expired authority evidence fails closed.
- Service boundary: Synergos transfer, synchronization and storage work without
  Curare. Curare owns production versions, review, approval and its business
  retention references; it neither relays bytes nor grants project membership.
  Curare's custody capability is an additional workflow restriction and cannot
  substitute for Cernere membership.
- Relay: implement the authenticated external relay only in C++. Until that
  acceptance gate passes, use Tailscale and bind the node only to its tailnet IP.
- Existing CLI, native GUI, admin UI and Ars adapter remain acceptance surfaces.

## Implementation and verification order

1. Integrate PR71 CI fixes and PR72 analysis snapshot; fix the verified-findings
   predicate so missing verification never promotes a finding.
2. Port bounded framing and version/path contracts, with deterministic boundary
   tests independent of a running service.
3. Port OS transports, message codecs, project/history services and durable I/O.
4. Port peer transport, discovery, transfer, custody and control endpoints.
5. Port user interfaces and adapter; add a service-owned Excubitor catalog with
   the C++ executable and explicit build command.
6. Review, merge into main, build from the project root, claim service testing,
   start via Excubitor, exercise real transfer/storage paths, verify process
   identity and release the testing claim in all outcomes.

## Completion gate

All existing feature families above must have passing acceptance evidence before
retiring Rust or redirecting the production service. Unit/library tests and
successful compilation are intermediate evidence, not full runtime acceptance.
Where the redesign replaces a legacy behavior, acceptance means conformance to
the new security/transfer/workflow contract, not preservation of that behavior.
Track codec completion separately from runtime enforcement: signature vectors
alone do not prove peer authorization, revocation propagation or node enrollment.
Windows execution cannot establish Linux/macOS runtime parity. External cloud
and multi-node tests must report unavailable prerequisites as unverified.
Keep the existing runtime intact until the C++ acceptance gate is satisfied.

## Evidence

Baseline main: 07af492. PR71 head: af6a9f2. PR72 source: d00529f7.
Praeforma project 01M3FVMGBFEGCFNRK8WG3CBAT8 had no spec records when queried.
Anatomia plan identified network-transport and opaque-custody; its source context
included generated Rust dependency headers, so it is not proof of Rust coverage.
Existing source/spec remains the compatibility authority. C++ source is placed
in existing module membership rather than introducing a cross-domain directory.

The first native increment implements bounded framing, path/version rules,
manifest/state JSON, receipt/capability signing, and exclusive durable file
publication. Windows unit tests pass; the Unix implementation is unverified.
Receipt JSON is bounded to 1 MiB / 128 nesting levels and rejects floating-point
numbers (custody receipt contracts are integral). Directory ownership/locking,
crash reaping, HTTP authorization, tombstone lifecycle and full request schema
validation are separate required layers; these primitives alone are not a store.
No production native executable, main reflection or Excubitor startup has occurred.

## Review correction

Local review 3182 timed out after 30 minutes while compiling the Rust workspace;
it did not report an assertion failure. Preserve the Rust suite and split its
compilation (`cargo test --no-run --locked --jobs 1`, 60-minute budget) from test
execution (30-minute budget). Register CMake configuration/build, native CTest,
and the Node verification-policy suite as additional cases. Registration alone
does not prove they passed in Revisor.

Normalize the archived analysis references to repository-relative paths and
documentation IP examples; do not alter confidential-term policies. The scanner
display-limit overflow classification was reported separately to its owner.
