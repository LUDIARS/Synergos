# Native custody request boundary

## [id: SPEC-SY-native-custody-request-boundary] Bind a sealed capability to one request

This P3 foundation consumes SPEC-SY-custody-capability-v2 and
SPEC-SY-revocation-bundle. It belongs to the existing generic opaque-custody
boundary and supports security value IDs G1/G2/G4: refuse stale evidence,
respect credential lifetimes and isolate tenants. It does not create a core
business domain or take Curare's version/review/retention decisions or Cernere's
membership/roster authority.

capability_gate.h/.cpp expose check_capability_request. Before a future body
reader can proceed, it requires an explicit revocation-cache view, both signed
parts' 300-second validity windows (neither future-issued nor expired), the
matching organization and a sealed custody-capability presentation. The LCT
library's new require_sealed option is true here; default codec inspection
continues to accept/prove unsealed holder credentials. A PEP refuses an
unsealed proof before copying its holder seed into the proof decoder.

The injected issuer Map and VerifiedRevocationState must come from trusted
signature/sequence-checked adapters. The type and timestamp checks do not prove
either signature, monotonic seq, current Cernere membership or a live cache.
No such adapter or fallback is supplied in this increment. Missing/default/
expired evidence fails with 503 revocation_stale. The future cache must verify
and combine both independently signed parts, preserve deny-only semantics and
poll/refresh as specified before it can supply this interface. Caller data must
remain immutable/locked during this synchronous check.

The request's org/project/object and stored sizes/digest come from the admission
or object owner, never self-reported scope/offset headers. Both scope strings,
the single get/put operation and exact object_keys/prefix must match. Optional
expected subject/session bindings, organization freeze, denied subject/session,
full project freezes, read-only project mode and token-chain revocation apply.
GET filenames come only from the safe signed download_name. A signed GET digest
requires an equal stored digest. A signed byte_range requires an explicit
contained request range and a known stored object size; a whole-object request
cannot bypass it. Inclusive range arithmetic cannot overflow.

PUT requires a known persisted upload byte count and max_bytes. Declared bytes
cannot exceed max_bytes minus the existing total, including across renewed jtis.
The returned request_byte_ceiling must cap actual streamed bytes, even for a
lying or absent Content-Length. GET max_bytes similarly bounds the selected
response length. No body buffer is allocated by this function. Logical object
keys remain opaque labels: they are not safe filesystem paths and must never be
joined directly to a storage root.

The result is RedemptionConstraints, not an admission or a durable reservation.
It carries the latest jti, the verified root delegation_jti when applicable,
subject/session, byte ceiling and signed max_uses/max_bytes_per_hour. A durable
owner still must atomically reserve/consume per-delegation and per-sub budgets,
journal the redemption, bind upload_id to its single commit, verify PUT digest,
stream bytes within the returned ceiling and publish durably before issuing a
v2 receipt. No successful transfer, storage or redemption is reported here.

CapabilityError exposes only generic public failure names and 403/503 mapping;
tokens, seeds, scope identifiers and internal paths are not included in errors.
The yyjson document is RAII-owned and returned strings are copies, so no view
outlives it. Optional sizes distinguish missing metadata from real zero bytes.
The operation defaults to unspecified (denied), numeric outputs to zero, and
arithmetic checks order values before subtraction or inclusive addition. There
is no retained shared mutable state, thread, socket, process or startup traffic.

Existing yyjson/Monocypher/Bcrypt/standard C++ dependencies are reused without
new packages. No custom HTTP parser, cryptography, service listener or network
transport is introduced. This library currently has no production caller.
Windows MSVC Release compilation of the new request-boundary executable passes;
it is not executed locally (tests=false). Linux/macOS compilation, native review
execution, real requests, durable ledgers/store/journal/streaming and production
reflection remain unverified or unimplemented as separately recorded.
