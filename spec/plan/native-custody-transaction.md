# Native custody transaction integration checkpoint

This plan refines the existing security P3/M09/M25 and transfer-v2 completion
conditions. It grants no deployment, first-v2-admission, legacy-secret deletion,
test or service-operation permission. Source contract owners remain
`redesign-security.md` sections4.9/4.22/4.35 and
`redesign-transfer-efficiency.md` section custody-transfer-v2.

## Established prerequisites — 2026-10-27

| Prerequisite | Evidence | Remaining boundary |
|---|---|---|
| Native sealed request constraints | Formal Sy#3492 Test OK/automatic merge7dd63f58e076; parent confirmed ancestor of current main69f1c25 | No production caller or durable consumption; Cr snapshot input is not signature/freshness validation |
| Native journal owner | Formal Sy#3496 Test OK/automatic merge51007a48ced9; notification-driven saved-id read confirmed merged/test_ok, head4b8a341, no human question/conflict | No listener, transaction integration, retention deletion or runtime reflection |
| Existing production compatibility | Current main69f1c25 includes a separate Rust capability-scoped HTTP block-transfer increment | It is not the native custody admission/receipt transaction and must not be rewritten or duplicated here |
| Existing full migration task | Parent existing-task GET: in_progress/sourcecc-command/completionEvidence null | The instruction/task link remains separate and unestablished |
| Runtime reflection | No startup/restart/deployment or real transfer acceptance performed | Checkout hash does not establish loaded-code identity |

The full product matrix remains
`../../../Curare/spec/plan/full-migration-execution.md`. This custody plan does
not close Cr/runtime/platform/product conditions in that matrix.

## One durable owner, one recovery contract

The next coherent implementation is the native redemption/admission transaction,
not a memory-only counter or a second isolated codec PR. It must combine these
conditions before a production listener can expose it:

1. Acquire checked sealed constraints using current, separately verified
   revocation evidence and trusted persisted object/upload metadata.
2. Atomically reserve applicable delegation and subject budgets before reading
   or serving bytes. Expiry/refresh must not reset a still-applicable parent's
   cumulative budget. Concurrent handlers and a process restart must not create
   independent counters for the same authority.
3. Record the redemption with its jti and server correlation in the authoritative
   journal. Define recovery for the crash points before/after reservation,
   journal flush, body append, final publication and receipt persistence.
4. Enforce streamed bytes against the reservation. A caller's declared length
   cannot override measured persisted upload bytes or authorize a replacement.
5. Commit an upload_id at most once, exclusively publish durable bytes, and
   persist the exact signed v2 receipt returned on subsequent receipt reads.
   Capability renewal changes jti, not the upload's commit identity.

There is no distributed-transaction claim between an independent journal file
and a new budget store. The implementation must choose an explicit durable
write/replay ordering and refusal state; writing two files in sequence is not
an atomic transaction. A failed/uncertain flush must not trigger an unqualified
automatic replay of admission, budget consumption or object publication.

## Budget identity information to settle in the transaction

The request boundary currently returns effective last-jti/subject, numeric
limits and the verified root delegation_jti. LCT attenuation may change jti and
narrow sub. This is sufficient to reject individual requests but is not by
itself a complete cumulative-budget schema. Before wiring counters, define:

- issuer/root-delegation namespace and retention horizon;
- root/delegated budget limits versus a child's narrowed limits;
- subject scope at each applicable limit, including narrowed subject paths;
- max_uses behavior across descendants and labels that are omitted/reused;
- the byte-window definition, concurrent PUT/object quotas and reservation
  cancellation/expiry treatment;
- fixed identities and conservative recovery for a request whose durable write
  result or client acknowledgement is unknown.

These are storage/enforcement design requirements, not evidence of a live
vulnerability: no production caller currently consumes these constraints.
Do not guess that keying only by the final child jti enforces a parent limit,
or mistake a caller-constructed freshness timestamp for verified Cr authority.
Numeric caveat intersection verifies attenuation; it does not consume state.

## Integration and review sequencing

The notified merge conflict was resolved in the journal branch, formally
accepted as#3496, and is now present in local main51007a48. This transaction
design branch is rebased onto that local main while retaining both saved
progress sections. Implement the durable transaction in this dedicated worktree
after the following design boundaries are settled. Preserve Rust production and v1 verify-only history;
the default v2 admission gate stays disabled. No handcrafted empty cache,
control-owned membership issuer or OIDC-key substitution is permitted.

The transfer-v2 contract also requires offset resume and serialized hasher state;
an old whole-object PUT port would not satisfy it. Content-key derivation and
encryption/placement ownership must follow their existing contracts rather than
be silently replaced with a convenient filename. A new hashing/storage library
would require normal dependency/license/install/transitive review; no such
dependency is introduced by this plan.

### Owner and phase constraints found in the source contracts

Curare's SPEC-CU-quota-reservation owns `{org,project,bytes}` admission capacity
for Curare-bound projects; custody enforces that reservation. Its quota policy
also includes max_objects/max_inflight_admissions. Synergos-only transfer/store
must not acquire a mandatory Curare dependency. Keep capacity reservation origin
and its authenticated owner explicit; an LCT's numeric ceiling is not evidence
that the business quota reservation exists. Do not fabricate Curare capacity
rows from control membership data or let a self-reported reservation authorize
bytes. The contract's AbusePolicy row shape is available in Curare M21, but no
current custody row or production numbers were found in either repository.

Synergos security M27/section4.24 defines the P5 project root secret in Vault KV,
owned by custody-admin, HKDF-separated project keys and Vault Transit KEKs.
Transfer-v2's content filename depends on that `HMAC(project_key,blake3)` owner.
The native custody library currently has no BLAKE3/resumable-hasher implementation
or Vault adapter. A P3 transaction must express these as required owner inputs;
it cannot implement a plaintext/fake-key fallback or claim P5 data protection.
Any real dependency selection and serialization ABI/version decision require
their normal crypto/dependency review before writing persisted state.

### Closed transition sketch for the implementation review

The future typed `change.diff` must carry an explicit version and closed kind;
it is not an arbitrary JSON escape hatch. The minimum replay records are:

| Transition | Durable contents | Replay/invariant |
|---|---|---|
| reserve | server attempt ID, immutable request fingerprint, scoped signed-budget identities, time, byte/use debits, optional active-PUT slot and upload_id | Debit every applicable parent/sub scope atomically in one row; duplicate attempt+fingerprint is one intent, a different fingerprint is conflict |
| append_complete | reservation attempt, upload_id, prior/new durable offset, hasher-state format/version and validated state binding | Advance only the expected offset after data and state are durable; do not trust an offset from a header or infer durable state from file length alone |
| release | attempt, terminal/cancel/recovery reason, slot release | Release an active operation slot once; do not refund uncertain bytes/uses or silently drop an unknown reservation |
| commit_intent | immutable upload/admission owner/scope, expected content key, digest/size and receipt issuance identity | One pending commit per upload_id; conflicting intent cannot replace it |
| commit_complete | pending intent identity, exact canonical receipt bytes/signature/kid and durable publication binding | One terminal result per upload_id, reusable verbatim on retry; no second receipt issuance |

All fields require explicit length/item bounds before allocation; sequence and
numeric arithmetic use existing JCS safe-integer limits. Scope lists remain
bounded by16LCT blocks. The full v2 receipt limit64KiB fits inside the current
event/row limits, but that does not permit nesting arbitrary receipt-shaped
JSON: validate through the existing receipt codec and preserve its exact bytes.
The decoder registry/corpus must cover these transition kinds and invalid replay
orders before any new writer can emit them. Requests/attempt IDs correlate
records; neither a client request ID nor a transition creates authorization.

Recovery must distinguish a committed terminal result, a fully persisted pending
intent, a torn/invalid journal and an unproven data/hasher state. The last two
cases refuse traffic and retain evidence. Unknown results are not fresh attempts.
For an active PATCH, quota concurrency is an active operation slot, not a lock
held throughout a paused multi-hour upload. Each subsequent PATCH/byte chunk
needs the currently valid capability/revocation and applicable budget evidence.
Offset resume does not renew permission, and capability renewal does not reset
the upload or ancestor counters. The min-append-size10MiB and idle120s values
come from transfer-v2; no new numeric abuse limits are inferred from them.

This sketch still requires a complete persisted format, state-machine proof and
attack/crash matrix. No transition decoder, authoritative reducer, storage writer
or production configuration is implemented by these design-only commits.

Permitted now: preserve formal evidence, inspect existing source/contracts and
prepare this transaction/recovery design. Pending review dependencies are not a
reason to manufacture more implementation branches. Revisor service tests and
crash/platform/real-transfer acceptance remain distinct from local static/build
evidence. The current tests=false and lack of service permission are unchanged.

## Proposed integration protocol and implementation prerequisites

The existing contracts do not specify an exact byte-window implementation or
provide a current custody AbusePolicy row with object/concurrent-PUT limits.
No numeric production defaults can be inferred from a codec vector. Missing
policy must fail configuration before the v2 admission gate can be enabled.
The following protocol is a concrete implementation direction to review, not
an assertion that its state machine or storage has already been built.

Use the authoritative journal as the durable source of transition truth rather
than add a second database whose commit cannot be atomic with journal append.
Extend its metadata-only writer with a closed typed `change.diff` transition
vocabulary; the shared event schema already permits diff, while the current
native adapter intentionally rejects arbitrary diff/body data. Never bypass that
adapter by appending unvalidated state JSON. Internal transitions must remain
bounded, contain no credentials and be covered by decoder/fuzz registration.
Counters/upload states are rebuilt from verified transitions; any derived index
is rebuildable, not a second authority. Required active-state/index cardinality
and spool limits must be explicit configuration and must fail closed.

Reservation and redemption evidence are fields of one flushed journal row.
Only after that flush can a handler accept/serve the reserved bytes. An uncertain
append poisons the owner; it must not start another handler or infer absence of
the record. Startup verifies and replays the journal before accepting work. This
orders state/evidence durably without claiming that independent data files and
the journal are one filesystem transaction.

For final publication, record a durable intent, exclusively publish flushed
content, then append a terminal commit with the exact receipt bytes/signature/kid
and immutable upload/content bindings before returning success. A crash between
publication and the terminal record remains a pending intent: recovery must
reconcile exclusive content and its verified digest/size before deriving a
terminal result. A preexisting conflicting object or receipt is never replaced.
Repeated committed upload_id returns its already stored terminal receipt only;
it cannot mint a different receipt or consume another commit. A pending or
uncertain attempt is not treated as a new upload. Recovery-specific rehash is
distinct from the transfer contract's no-rehash normal resume requirement.

Verified budget input must preserve every applicable signed ancestor limit and
its immutable scope, not only the final jti/sub. The design needs a bounded
verified budget-scope list (at most the existing16blocks), issuer/root binding,
the caveat block identity and subject at that constraint. Descendants debit all
applicable ancestor scopes. M21 per-sub policy must also span renewed delegation
tokens so a new issuer-approved delegation does not bypass the subject's policy.
This metadata derives only from the already verified chain and never grants
membership or accepts self-declared scopes.

For max_bytes_per_hour, a rolling3600-second window is a conservative proposed
interpretation: it prevents fixed-window boundary doubling. The immutable
transition records retain reservation times; clock regression must not expire
debits early. Long uploads reserve bounded chunks before I/O rather than charge
the entire object once and let its reservation age out mid-stream. Unknown or
failed transfers do not automatically refund bytes/uses. Concurrency slots may
be released only by an explicit durable terminal/cancel/recovery transition.
Refresh changes capability jti but not upload identity or inherited budgets.
These conservative accounting choices still need their attack/crash acceptance
cases before any production caller is enabled.

Remaining before code integration: specify the closed transition schema and
replay invariants, bounded active-state policy, complete budget-scope extraction,
checkpoint/index retention anchor, resume-hasher serialization/version binding,
and the existing encryption/content-key input contract. Do not fill any of those
with mock policy, an in-memory-only ledger, arbitrary filenames or a parallel
issuer. This plan introduces no package, install script or transitive dependency.
