# Native compatibility primitives

These primitives support the C++ migration; they do not constitute a complete
daemon, peer transport or custody HTTP service.

## [id: SPEC-SY-native-framing] Bounded IPC framing

A frame has a four-byte little-endian unsigned length followed by at most 1 MiB
of payload. Reject oversized lengths before payload allocation. Incremental input
may split any header or payload byte. Consume at most one frame per result and
preserve remaining input for the next frame. A truncated frame at EOF is an
error. An oversized frame poisons the decoder until its connection is discarded.
MessagePack and command authorization are separate required layers.

## [id: SPEC-SY-native-metadata] Compatible version metadata

Manifest/state JSON preserves format 1, project identity, file metadata and
unsigned integer precision. Validate the project identity before accepting
entries. Paths must be relative, without dot segments, control characters,
alternate stream syntax or the metadata root. This lexical check does not replace
filesystem symlink/junction checks. Reject zero/maximum versions and overflow
before mutation. Highest-seen versions are monotone even after a forced restore;
the next publication uses the greater of manifest version and local high-water
mark. Unchanged size/CRC retains its existing version, matching the Rust contract.

Reject duplicate metadata/canonical receipt keys rather than relying on a JSON
library's first/last-value behavior. Canonical receipt objects sort UTF-8 keys;
arrays preserve order and control-character escapes use serde-compatible
lowercase hexadecimal digits. Literal escaped backslashes remain literal.

Custody cryptography/publication is governed by SPEC-SY-opaque-custody and
SPEC-SY-custody-durability in `opaque-custody.md`. Test constructors and platform
resource destructors are exercised by their owning test/library; absence of
static call edges is not proof they are unused.
