# Full migration checkpoint — 2026-10-25

## Authorized scope

neco requested the full rewrite/migration delegated to Sol, including the
Cernere authority design. The earlier build-memory-only milestone is superseded.
Synergos transfers/synchronizes/stores independently of Curare. Cernere owns
membership and roster authority; Curare owns production versions/review/approval.

## Saved implementation

Base: local main `0510896`. Branch: `feat/native-full-migration-20261025`.

- Reused existing P1/P1b work: JCS, Ed25519 envelopes, LCT, receipt v2,
  delegation/capability profiles, golden vectors and bounded decoder fuzz targets.
- Corrected membership decoding against SPEC-SY-membership-token: require
  Cernere issuer, peer/node/principal/device/organization, jti, roster epoch and
  explicit projects. Reject the old `{exp, org, sub}` fixture and holder changes
  to peer identity or roster epoch. This changes the membership vector contract.
- Updated the migration plan and documented implementation/contract ownership in
  `spec/feature/native-trust.md`.
- Added native LocalStream/LocalChannel client transport: same-user Windows named
  pipes and Linux/macOS Unix sockets, monotonic deadlines, RAII and bounded frame
  reception. No server, IHELLO or application command authorization is claimed.

## Evidence and limits

- MSVC 19.39 / Visual Studio 2022 Release build of `synergos_trust_test` succeeded.
  CMake used existing pinned yyjson/Monocypher source downloads from the prior
  P1b worktree; a clean dependency download has not been demonstrated.
- Explicit `--write-vectors` generation succeeded. This is fixture generation,
  not independent execution of the native acceptance suite.
- Contract registry, direct-sign prohibition and fuzz-registration checks passed.
- The Windows local-IPC library and endpoint contract executable compile with
  MSVC. The executable was not run; named-pipe ownership/cancellation/frame-I/O
  acceptance and Linux/macOS compilation/runtime remain unverified.
- Initial `git diff main | anatomia verify --repo <worktree> --json` found
  specification orphans in inline `.hpp` entities. The new trust module now uses
  `.h`, which the current analyzer indexes, and the existing metadata exception
  keeps its public header path with definitions moved to its `.cpp` owner.
  Rule conformance, duplication and specification linkage now pass (zero
  specification orphans). Coupling/naming advisories remain and the CLI still
  returns exit 1. Do not suppress them or report the overall check complete.
- No unit/integration suite, service startup/restart, real transfer, production
  reflection or GitHub push was performed locally for Synergos.
- Synergos successor Revisor #3461 was submitted at `fad0341`; its first attempt
  stopped before registered tests on Praesidium `native.configuration-required`.
  All 44 native units are now declared in `praesidium.json`. Pinned dependency
  headers and licenses were extracted only after both archive SHA256 checks,
  installed outside this checkout, and registered as host-owned aliases without
  changing existing compilers/packages. Compiler-only preflight now reports
  passed for 44/44 units, findings empty. This is not native test execution.
- The migrated native/gate/fuzz scope now has zero unclassified program modules
  under the explicit layer declaration. Remaining project-level unclassified
  symbols belong to untouched site/workflow/Tauri code. Public native APIs that
  still lack application callers remain unintegrated, rather than gaining
  artificial calls to hide graph diagnostics. New review results are required.

## Coordination state

The API subagent and parent inherit one Lictor sidecar. The actual task binding
was set to this worktree/branch; an independent persistent worker is preferable
before multi-repository review submissions.

Repository binding was subsequently corrected through `implement begin` and
read back against the exact worktree/origin/branch. Exact safe.directory entries
were required for worktrees created by the sandbox account; no broad trust rule
was added. Curare's independent read-only IPC increment passed Revisor #3460
and merged as `11c0d5f15f3b`. Parent readback confirms Curare main contains it,
but the running service still reports the older revision and its new compiled
module is absent. Build/restart/reflection and user-experience acceptance remain
unperformed; the merge is not evidence of live behavior.

A formal `sol-6-1` delegation failed with `actio_outcome_unknown` during
`sealDelegationTask`, before prompt persistence, run registration or spawn.
Readback found no matching delegation run. The request was not resent because
the Actio write outcome remains unknown. Reading existing Actio task
`f18ad452-0315-4930-826a-d2935c51b3d1` through Cc content returned HTTP 403;
do not rewrite its source/ownership to force a link or silently create another.

## Work still required

Resolve task/worker coordination and header analysis coverage; validate the
codec increment and submit through Revisor. Inspect Cernere's actual issuer,
roster, key-publication and revocation contracts against its existing owner
tasks (`actio:fd0e4de8`, `actio:2a7af0fa`). Do not implement their authority in
Synergos or Curare as a temporary substitute.

Pre-submission duplicate check found existing Synergos Revisor #3418 open with
action_required for the original P1 branch (`1028dbe`). This worktree is its
explicit successor: it aggregates that implementation, P1b and subsequent
Cernere-binding fixes/transport. Revisor submission has no headRef reassignment
operation; the old branch and old review are left untouched rather than moving
another session's checkout or silently closing it. Submit this complete
successor once through the normal Cc session endpoint and reconcile the old
partial review after successor acceptance. Do not enqueue #3418 in parallel.
Other open Synergos PRs #3434 (IPC path) and #3429 (node bind) are separate work.

Cernere source check: local main `d163e98`, `server/src/commands.ts` dispatches
organization/member/project_definition/org_project/managed_project operations.
No Synergos membership/roster contract names or payload types were found in
`server/src`, `packages`, `contracts`, `spec/feature` or `spec/interface` by the
targeted contract search. `server/src/http/oidc-handler.ts` publishes the OIDC
JWKS, but `server/src/auth/oidc-keys.ts` explicitly owns RS256 ID-token keys,
separate from Ed25519 keys. That endpoint is not evidence of the required
Cernere membership/roster issuer keys. The owner tasks remain dependencies;
no guessed endpoint or control-owned issuer is substituted.

Continue Curare runtime redesign and the C++ transport/session/authorization,
control/custody, persistence, synchronization, CLI/GUI/adapter and operation
surfaces. Membership codec success is not fresh RosterStatus validation or a
peer authorization gate. Keep Rust production intact until the redesigned
acceptance gates pass. Distinguish implementation, review, merge, reflection and
real environment acceptance throughout.
