# Capability-only HTTP block transfer

## [id: SPEC-SY-content-http] Additive legacy content adapter

Authorization: neco 2026-10-27 requested inspection of the IPFS-file-stream
origin and implementation of TCP or HTTP transport with chunk reuse, compression
and resumability. This additive Rust CLI adapter is scoped to that request; it
does not implement/deploy the frozen Rust relay or declare the C++ redesign done.

Baseline f5ce437: content/block.rs and chunker.rs implement BLAKE3-addressed
blocks and MessagePack ChunkDag; session.rs implements bounded QUIC Bitswap.
The ordinary exchange path still sends entire files. Compression and persistent
resume are not connected there. Praeforma project 01M3FVMGBFEGCFNRK8WG3CBAT8
has no spec records; Anatomia context identified the same transfer gaps.

Reuse the existing block CID and ChunkDag bytes unchanged. Prepare a private
disk cache and a public manifest with unique block IDs, encoded lengths, SHA-256
of wire bytes and identity/gzip codec. Fast gzip is used only with >=10% savings.
The default chunk is 4 MiB, independent of TCP MSS/MTU. Bound decoded/encoded
blocks to 16 MiB and DAG children to 131072. TCP segmentation belongs to the OS;
no UDP, TUN or QUIC connection is created by this path.

Upload/download use caller-supplied, method-scoped HTTP capabilities, supplied
via stdin. The client does not issue credentials or receive custody admin keys.
The existing authorized custody issuer can map encoded blocks to opaque objects
and mint PUT/GET URLs. Its authorization and retention decisions are unchanged.
An origin allowlist is provided separately by the operator; redirects, userinfo,
fragments and unapproved origins are refused. Cleartext is loopback-only.
Capability values never appear in reports or network errors.

Download requires an independently trusted root CID. It validates the root
before scheduling children and validates every block's decoded BLAKE3 before
persisting it. Cache presence alone is not verification. Completed blocks survive
interruption; retry only missing/corrupt blocks. Do not permanently mark uploads
complete locally: a refreshed plan is authority for which uploads remain needed.
Bound concurrency and retry count; never retry policy/auth failures as timeouts.
Assemble to a sibling temporary file, sync, and publish without replacing any
existing target. Missing/invalid data must not truncate the user's destination.

### Validation plan

Domain ownership: existing `mesh-core.domain.json` membership covers the new
`synergos-core/src/content_http` implementation and both colocated test modules.
Anatomia context/plan was consulted; its cache write returned EPERM and Rust
analysis is limited. Augur's automatic plan classified the objective as unknown;
the old repository `.augur-plan.json` describes a different bind-IP change and is
not evidence for this feature. The explicit acceptance cases below remain required.

Test repetitive/incompressible/empty data, encoded-size and decompression limits,
bad CID and length, corrupt cached blocks, wrong trusted root, missing grants,
duplicate blocks, origin/redirect/secret-log protection, bounded concurrency,
interrupted transfer followed by reuse, existing destination preservation, and
PUT/GET against actual ephemeral HTTP fixture. Production startup remains an
Excubitor operation from the main checkout. Cloud acceptance is separate.

This is explicit file transfer, not automatic project synchronization, Cernere
membership issuance, standard IPFS interoperability, CDC, Bao or native TXF2.

### CLI and storage integration

`synergos-core content-http prepare <stable-file> --cache <private-cache>`
emits the public UTF-8 JSON manifest. Do not modify the source during preparation;
metadata checks are detection aids, not an operating-system snapshot. The cache
must be private to its user; the exclusive lock coordinates cooperating clients,
not hostile local writers. There is no automatic cache eviction.

An existing authorized issuer must construct a UTF-8 JSON plan:
`{"manifest": <prepare-result>, "grants": [{"cid": "blake3-...",
"get_url": "https://storage.example/object?read-capability",
"put_url": "https://storage.example/object?write-capability",
"get_headers": {}, "put_headers": {}}]}`. Upload requires both grants for
every unique block. Download may omit grants for already verified cached blocks.
The issuer must bind opaque object length/digest to `wire_bytes`/`wire_sha256`,
apply its normal authorization, and provide read-after-write visibility. Issuer
wiring is not implemented by this adapter. An expired grant requires a refreshed
plan; no local cache flag overrides remote authorization or retention.

Pipe the plan through stdin to `synergos-core content-http push --cache <cache>
--root <independently-trusted-root> --origin https://storage.example`, or
`synergos-core content-http pull <new-output> --cache <cache> --root <trusted-root>
--origin https://storage.example`. Do not put capability values in shell arguments
or shell history. Do not derive the trusted root or origin policy from an untrusted
incoming plan. The executable disables tracing for these commands, including
dependency tracing, to protect capabilities and keep stdout as JSON only.

HTTP uses pooled connections and up to four concurrent blocks by default (maximum
16). Retries cover connection/body failures and HTTP 408/429/500/502/503/504 with
bounded backoff; redirects, authorization and integrity failures are terminal.
The output filesystem must support same-directory hard links for no-clobber
publication. Per-block PUT verification currently reads the uploaded bytes back;
this costs bandwidth and is deliberately not a throughput optimization claim.
`wire_bytes` counts completed payloads, not headers or failed partial attempts.
No Cloud throughput, MTU tuning, proxy certificate compatibility, or node-to-node
acceptance is established by loopback fixtures.

Dependencies: flate2 1.1.10 and sha2 0.10.9 were already in the baseline lockfile;
the adapter adds direct exact-version references without version upgrades. Both
are MIT OR Apache-2.0, have `build = false`, and provide local byte processing
(flate2 default rust_backend/miniz_oxide; sha2 std/digest/cpufeatures). Their library
source has no runtime network or child-process entrypoints. HTTP calls occur only
on explicit push/pull, send selected encoded blocks to operator-approved origins,
and use only supplied per-request capabilities. Preparation is offline. Existing
reqwest proxy configuration is inherited; this does not grant Cloud network access.
For an intercepting HTTPS proxy, pass the environment-provisioned PEM certificate
bundle with `--ca-bundle <path>`. This adds trust anchors without disabling TLS
verification; absent, empty, malformed or oversized specified bundles fail closed.
Never obtain a trust anchor from the incoming transfer plan. Default public roots
remain available. The actual Cloud policy and CA path must be checked in Orbis
before acceptance; the client does not change the executor network policy.

### Verification record (2026-10-27)

Actio: `fe85d990-baed-4a01-9266-4ed35b2d2666`, linked to neco's HTTP/TCP
implementation instruction. Do not close the older HQ custody deployment task
based on this adapter.

- `cargo check -p synergos-core --offline`: passed, including the CLI binary.
- `cargo check -p synergos-core --tests --offline`: passed; this is static checking,
  not execution of the test cases.
- Targeted rustfmt checks and `git diff --check`: passed.
- `cargo test ... --lib content_http:: --offline -- --test-threads=1`: blocked
  before execution by the external-import PreToolUse gate, review key
  `db71761535db9ee2c39cbb17c2fe4bad1980f26b7e9743d6faa2b95aed4bae31`.
  Required seven-day upstream/transitive history review is unavailable; no allow
  record was fabricated, no test was run, and the blocked command made no claim.
- Runtime fixtures, production custody grant integration, large-file throughput,
  and Orbis-mediated Codex Cloud/HQ transfer remain unverified. Main is unchanged.
