# Versioned Transfer Safety Regressions

- Date: 2026-08-16
- Status: fixed in working tree
- Area: file exchange, manifests, peer bootstrap
- Severity: high — data loss, path escape, cross-project disclosure, and silent divergence

## Summary

The versioned re-publish change introduced regressions where a failed destination replacement could delete the last valid file, network paths could traverse a pre-existing symlink/junction, concurrent manifest saves could lose newer state, and equal-version concurrent edits could silently diverge. Project-agnostic offer caching could also serve a same-named file from another project.

## Evidence

- `synergos-core/src/exchange/mod.rs`: removed an existing destination before rename and keyed `shared_files` only by `FileId`.
- `synergos-core/src/manifest.rs`: used a PID-only temporary name, deleted the manifest before rename, and performed lexical-only relative-path validation.
- `synergos-core/src/project.rs`: allowed concurrent snapshots to save out of order.
- `synergos-core/src/daemon.rs`: treated any local version greater than or equal to an offer as already held, even when equal-version content differed.
- `synergos-core/src/peer_bootstrap.rs`: followed HTTP redirects from invite-controlled bootstrap URLs.

## Regression Context

These failures are regressions in the PR that adds persistent manifests, cross-machine invites, and repeat publish support. The original single-writer E2E did not exercise replacement failure, symlinked parents, multiple projects with the same file ID, concurrent manifest writers, or concurrent edits.

## Cause

The implementation assumed rename-overwrite behavior was uniform across operating systems, treated component validation as equivalent to filesystem containment, persisted unlocked snapshots, and treated `(file_id, version)` as globally unique without project or content identity.

## Fix Requirements

- Preserve the old destination when replacement fails and use an atomic same-volume replace.
- Reject existing symlink/junction components and revalidate after directory creation.
- Serialize each project's manifest mutation through durable replacement.
- Scope shared-file and gossip deduplication state by project.
- Report equal-version/different-content offers as conflicts without overwriting local data.
- Validate bootstrap URL forms before local side effects and do not follow redirects.
- Reject peers using the pre-versioned transfer protocol.

## Verification

Static regression tests were added or updated for symlink containment, project-scoped gossip deduplication, URL validation, versioned transfer framing, and project-scoped exchange calls. No tests were run during review because execution was outside the review constraints. CI should run the registered Rust test suite, followed by a real two-machine invite/publish/re-publish exercise.

## Follow-up

Project membership remains enforced by network reachability rather than a project ACL. A later protocol change should authenticate membership and add parent-version/content-hash metadata for richer conflict resolution.
