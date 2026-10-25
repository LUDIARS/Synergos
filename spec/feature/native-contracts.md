# Native compatibility primitives

These primitives support the C++ migration; they do not constitute a complete
daemon, peer transport or custody HTTP service.

## [id: SPEC-SY-native-membership-codec] Cernere membership bindings

The membership LCT codec follows SPEC-SY-membership-token in
`spec/plan/redesign-security.md`. Block 0 and the effective caveats require
`iss: cernere`, `sub_peer_id`, `node_id`, `principal`, `device_id`, `org`, `jti`,
an integer `roster_epoch`, and a nonempty list of explicit projects with
`read`, `publish` or `history` roles. A legacy `{exp, org, sub}` record is
insufficient. Holder attenuation cannot change the peer, node, principal,
device, organization or roster epoch. Validate the root before attenuation;
a holder cannot supply authority bindings omitted by Cernere.

This is a codec invariant, not a completed session authorization gate. The
runtime must still pin Cernere membership keys, compare authenticated SESS
identity and node-card scope, verify fresh Cernere RosterStatus, enforce
revocations and check project/operation before reading stream bodies.

## [id: SPEC-SY-native-framing] Bounded IPC framing

A frame has a four-byte little-endian unsigned length followed by at most 1 MiB
of payload. Reject oversized lengths before payload allocation. Incremental input
may split any header or payload byte. Consume at most one frame per result and
preserve remaining input for the next frame. A truncated frame at EOF is an
error. An oversized frame poisons the decoder until its connection is discarded.
MessagePack and command authorization are separate required layers.

## [id: SPEC-SY-native-local-transport] Local IPC client transport

`synergos-ipc/native/local_channel.cpp` connects the bounded frame decoder to
the OS stream exposed in `synergos-ipc/native/include/synergos/ipc/local_channel.h`
and `synergos-ipc/native/include/synergos/ipc/local_stream.h`.
`synergos-ipc/native/local_stream_windows.cpp` accepts only local named pipes,
decodes endpoint names as UTF-8 and verifies the server process token SID equals
the caller's SID before application bytes are written. It requests identification
only, so connecting does not delegate impersonation rights to a pipe server.
`synergos-ipc/native/local_stream_posix.cpp` accepts absolute filesystem Unix
socket paths, rejects symlink/non-socket/wrong-owner endpoints, and verifies the
connected peer's effective OS user using SO_PEERCRED or getpeereid.

Every connect/read/write accepts one absolute monotonic deadline. Transfers
use at most 64 KiB per OS operation; frame reception allocates incrementally
under SPEC-SY-native-framing. A failed or partial frame closes the channel;
buffered bytes after a complete frame remain available for the next receive.
RAII owns sockets, process/token handles and overlapped events, including
cancelled operations. No IPC listener or network endpoint is created here.

Same-user verification is a transport prerequisite, never command authority.
SPEC-SY-ipc-capability still requires IHELLO, scoped tokens and destructive
operation confirmation before the production client/daemon is switched over.
This client transport alone does not implement those application protocols.

`synergos-ipc/native/tests/local_endpoint_test.cpp` covers the pure endpoint
allow/deny boundary and an expired deadline before OS connection, without a
listener. Peer-credential, cancellation, concurrent close and real frame I/O
acceptance need separate OS-specific runtime verification; a build is not that
evidence.

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
