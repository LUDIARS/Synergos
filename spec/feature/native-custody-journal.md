# Native custody journal owner

## [id: SPEC-SY-native-custody-journal] Bounded durable access journal

This is the M25 implementation contract for SPEC-SY-custody-journal in
`../plan/redesign-security.md` section4.22. It is part of the existing complete
Synergos migration, not a new authority service or a production cutover.
G4 requires attributable, immutable custody access; G1 requires failures to close
new destructive work rather than lose the evidence needed to revoke access.
The existing opaque-custody domain owns the access records. Filesystem locking,
append/flush and recovery are infrastructure, not membership decisions.

## Storage and failure boundary

One journal owner holds an exclusive OS lock for the entire lifetime of a stream.
The service supplies an absolute, private, canonical storage directory outside
the checkout. The owner opens its fixed filename without following symlinks or
reparse points and rejects non-regular files and conflicting process ownership.
Network filesystems and externally mutable roots are outside this contract.
The OS service identity must own the parent directory; a file lock does not
protect against another principal replacing the directory or file.

Rows are UTF-8, binary-mode LF-delimited JCS objects. Each row carries `stream`,
`seq`, `prev_hash`, `row_hash` and the `event` security-event payload. Sequence
starts at1; the first previous hash is32 zero bytes. Hashes are lowercase SHA-256
hex. `row_hash = SHA-256(previous raw32 bytes || JCS(row without row_hash))`.
The event follows SPEC-CU-security-event (`ludiars.security-event/v1`); the
producer supplies already-redacted actor/subject/outcome/correlation metadata.
Raw credentials, capability bytes, IP addresses, email, paths and bodies are not
journal fields. Client-selected request IDs never become server trace IDs.

The first adapter deliberately accepts a metadata-only subset: numeric v1 and
nonnegative UTC Unix seconds `ts`; closed `custody.admit|serve|redeem|deny|reclaim|purge|scrub`
event types; `success|denied|failure` outcome; bounded printable ASCII identifiers;
an attributed actor; org-bound subject; optional before/after SHA-256 hashes;
and a nonzero lowercase32-hex trace ID. Unknown fields are rejected at every
object boundary. Arbitrary `change.diff`, IP/UA fields and unauthenticated
aggregation are not accepted by this adapter. Their later shared-schema adapter
must define redaction/aggregation explicitly; this writer is not evidence of
complete event coverage. Producers still own server-generated IDs and semantic
redaction: printable IDs cannot establish trustworthy provenance by themselves.

Startup scans complete rows in sequence with bounded buffers, validating JCS,
schema, hash and stream binding. A partial trailing row, altered row, missing
sequence or mismatched stream fails closed. Startup never truncates, silently
skips or rewrites a damaged record. Offline repair needs separately preserved
evidence and an operator procedure; an unavailable journal is not an empty one.
No historical records are imported or altered by opening this new owner.

An append serializes concurrent callers, bounds a row to1MiB and the spool to
1GiB, writes the entire row, then flushes before returning its new head. The
owner enters a poisoned state if write/flush has an uncertain result. It rejects
all later appends in that process; no automatic same-row retry is allowed.
Successful flush alone is not a custody admission or an object commit. A future
redemption/admission transaction must define its own record/state ordering and
recovery reconciliation before bytes or destructive work become observable.

## Checkpoints and remaining integration

Checkpoint payload is `{stream,seq,root_hash,ts}`. It is signed only through the
existing signed-envelope module with ctx `synergos/journal-checkpoint`, ver1,
custody signer and the injected custody key. The owner does not create an issuer
key, use legacy HMAC or log signing material. Production scheduling must persist
and distribute a checkpoint at least every5 minutes, independently of requests.
A hash chain without an externally preserved checkpoint cannot detect tail
truncation or replacement of the entire local stream; do not claim otherwise.

This increment is not a listener, runtime caller, durable budget, upload commit
ledger, retention deletion worker or Vestigium exporter.400-day security retention,
checkpoint scheduling/distribution, sensitive-event coverage and ingestion
`GET /v1/custody/journal?since=seq` remain acceptance conditions. No first-v2
admission, old-secret deletion or deployment is enabled. Existing Rust runtime
remains intact. Windows compile evidence is not POSIX/runtime/crash-injection
evidence. Local tests and service actions remain disabled (`tests=false`).
