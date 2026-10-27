# Native trust codec implementation

The normative security contracts are defined in `spec/plan/redesign-security.md`.
This document maps the reusable P1 codec implementation to those contracts.
It does not declare P3 control/custody or P4 peer enforcement complete.

## Signed envelopes and canonical data

SPEC-SY-signed-envelope requires bounded, duplicate-free JCS, a closed context
registry and Ed25519 verification with the signer allowed for that context.
Membership, node cards and roster records are Cernere authority records;
permission to sign node data never permits signing those records.

| Implementation | Responsibility |
| --- | --- |
| `synergos-net/native/trust/jcs.cpp` | Canonical UTF-8 JSON and numeric/depth/size bounds |
| `synergos-net/native/trust/include/synergos/trust/jcs.h` | Canonicalization API and limits |
| `synergos-net/native/trust/crypto.cpp` | Ed25519 keys and canonical base64url encoding |
| `synergos-net/native/trust/include/synergos/trust/crypto.h` | Explicit key ownership and byte types |
| `synergos-net/native/trust/envelope.cpp` | Context registration, signer checks and domain-separated signatures |
| `synergos-net/native/trust/include/synergos/trust/envelope.h` | Closed context/signer API |
| `synergos-net/native/trust/error.cpp` | Stable public trust failure names |
| `synergos-net/native/trust/include/synergos/trust/error.h` | Typed trust failures |

## LCT verification and attenuation

SPEC-SY-lct-v1 verifies the signed block chain and proof, closed caveat vocabulary,
intersection, expiry, profile and revocations. SPEC-SY-membership-token and
SPEC-SY-native-membership-codec require Cernere's root authority bindings before
accepting a membership token. A holder may narrow project roles but cannot
replace peer/node/principal/device/organization/roster bindings. Root signature
verification is still insufficient to establish live session authorization.

P3 custody request consumers now set Presented.require_sealed=true; this is an
explicit PEP boundary and does not change default holder/codec inspection.
Verified additionally preserves the root delegation_jti for the durable budget
owner instead of inferring a delegation from the last attenuated label. These
fields do not provide a live revocation cache or reserve a redemption budget.

| Implementation | Responsibility |
| --- | --- |
| `synergos-net/native/trust/lct.cpp` | Token/proof framing, chain verification, attenuation, expiry and revocation |
| `synergos-net/native/trust/include/synergos/trust/lct.h` | Token/profile/issuer and verification API |
| `synergos-net/native/trust/caveats.cpp` | Closed caveat decoding and monotone intersection |
| `synergos-net/native/trust/caveats.h` | Effective caveat representation |
| `synergos-net/native/trust/profile_rules.cpp` | Membership, custody delegation and capability invariants |
| `synergos-net/native/trust/profile_rules.h` | Root/presentation rule boundary |
| `synergos-net/native/trust/json_read.h` | Internal typed JSON access and document lifetime |

## Custody receipt codec

SPEC-SY-custody-receipt-v2 uses a custody Ed25519 receipt key and validated receipt
fields. This codec neither publishes bytes nor claims durability for a store.

| Implementation | Responsibility |
| --- | --- |
| `synergos-net/native/trust/receipt.cpp` | Receipt schema, canonical envelope and verification |
| `synergos-net/native/trust/include/synergos/trust/receipt.h` | Receipt claims and issuer API |

## Compatibility vectors

The vector harness checks byte representation, typed failure names and exact
effective caveats. Test-only token surgery creates malicious chains without
weakening production minting. Fixture generation is separate from acceptance
execution; generated fixtures alone are not independent verification.

| Implementation | Responsibility |
| --- | --- |
| `synergos-net/native/trust/tests/trust_test.cpp` | Native trust assertions and explicit fixture-generation entry |
| `synergos-net/native/trust/tests/vectors.cpp` | Compare registered JCS, envelope, LCT and receipt fixtures |
| `synergos-net/native/trust/tests/vectors.h` | Vector harness entry declarations |
| `synergos-net/native/trust/tests/vector_gen.cpp` | Canonical/envelope/context fixture generation |
| `synergos-net/native/trust/tests/vector_gen_lct.cpp` | LCT success, rejection and attenuation fixtures |
| `synergos-net/native/trust/tests/vector_gen_receipt.cpp` | Receipt success and rejection fixtures |
| `synergos-net/native/trust/tests/vector_gen.h` | Fixture generator interfaces |
| `synergos-net/native/trust/tests/json_writer.h` | Fixture JSON document lifetime |
| `synergos-net/native/trust/tests/token_surgery.cpp` | Test-only malformed/adversarial token construction |
| `synergos-net/native/trust/tests/token_surgery.h` | Test-only token construction interfaces |

## Contract and decoder gates

SECURITY-M01 requires the declared contract registry to match normative text and
prevents production signing outside the envelope boundary. SPEC-SY-decode-bounds
requires decoder size bounds and registered fuzz coverage. The registry checks
coverage metadata; running bounded corpus replay is a separate verification.

| Implementation | Responsibility |
| --- | --- |
| `scripts/gate/contract-registry.mjs` | Contract registry check/regeneration entry |
| `scripts/gate/lib/contracts.mjs` | Extract, hash and compare normative definitions |
| `scripts/gate/no-direct-sign.mjs` | Production signing-boundary check entry |
| `scripts/gate/lib/direct-sign.mjs` | Detect signing outside allowed modules |
| `scripts/gate/fuzz-registry.mjs` | Decoder registration check entry |
| `scripts/gate/lib/fuzz-registry.mjs` | Match decoder IDs, targets and source declarations |
| `scripts/gate/fuzz-run.mjs` | Bounded replay/fuzz execution entry |
| `scripts/gate/lib/fuzz-plan.mjs` | Platform-specific replay paths and bounded invocation plan |
| `fuzz/replay_main.cpp` | Bounded corpus replay harness |
| `fuzz/targets/trust.jcs.cpp` | Canonical JSON decoder adversarial input boundary |
| `fuzz/targets/trust.envelope.cpp` | Envelope decoder adversarial input boundary |
| `fuzz/targets/trust.lct.cpp` | LCT decoder adversarial input boundary |
| `fuzz/targets/trust.receipt.cpp` | Receipt decoder adversarial input boundary |
| `fuzz/targets/core.manifest.cpp` | Manifest decoder adversarial input boundary |
| `fuzz/targets/core.version_state.cpp` | Version state decoder adversarial input boundary |
| `fuzz/targets/custody.canonical.cpp` | Legacy receipt canonicalizer adversarial input boundary |
| `fuzz/targets/ipc.frame.cpp` | Incremental IPC frame adversarial input boundary |

Native metadata bounds also apply in
`synergos-core/native/include/synergos/core/manifest_codec.hpp`,
`synergos-core/native/manifest_codec.cpp` and
`synergos-core/native/tests/manifest_codec_test.cpp`.
Legacy custody canonicalization in `synergos-custody/native/canonical.cpp` and
IPC framing in `synergos-ipc/native/frame.cpp` retain their existing contracts
and expose registered decoder limits.
