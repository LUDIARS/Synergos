# Native preflight configuration

`praesidium.json` declares every tracked native translation unit for Revisor's
compiler-only preflight. It uses C++20, the include directory of the owning
CMake target and the external header packages that target actually consumes.
It does not select an executable, arbitrary compiler flags or external paths.
The quality-gates domain owns this inspection manifest.

## Trusted host prerequisites

Revisor owns `praesidium.host.json` next to its operator configuration, outside
the reviewed checkout. Its `compilers.c` / `compilers.cpp` must point to installed
Clang executables. Its `includePackages` provides these package names:

| Name | Header directory from the pinned source archive |
| --- | --- |
| `yyjson` | yyjson `8b4a38dc994a110abaec8a400615567bd996105f`, `src` |
| `monocypher` | Monocypher `4.0.2`, `src` |
| `monocypher-ed25519` | Monocypher `4.0.2`, `src/optional` |

The package locations must be stable, installed outside the reviewed repository
and independently owned by the inspection host. Do not point them at temporary
Revisor worktrees or use a reviewed repository as its own trust authority.
Preserve existing compiler settings and unrelated packages when adding aliases.

Use the same archives fixed by the root and trust CMake definitions:

- yyjson archive SHA256:
  `94e9c90f8f12d8329f2c0756782d21e5c278729023cd2c5fe47e323a35947ed6`.
- Monocypher archive SHA256:
  `bc1ca30b1b2654e4e7daf2492c0d204200e55137f23fda6b7142fd7d523bd6b4`.

Verify archive hashes before extracting the three headers and their license
files. These inspection headers are not a substitute for the pinned dependency
sources used by the actual CMake build. A header alias is not a claim that the
third-party implementation has been analyzed.

## Build variants and test fixtures

Windows-only and POSIX-only translation units have explicit preprocessor guards
matching their existing CMake platform selection. On a Windows host Clang
inspects active Windows code; an inactive Unix translation unit is parsed as
empty. That is not Linux/macOS analysis or runtime evidence. Those build variants
still require their own host toolchain and verification, including OpenSSL 3
for POSIX custody hashing. Do not report a Windows-only preflight as portable
acceptance.

The trust test accepts a fixture directory as an explicit argument, rather than
requiring a compiler-defined string with a machine path. CTest supplies the
repository's `spec/vectors` path. Fixture generation uses
`synergos_trust_test --write-vectors <vectors_dir>`; no implicit directory or
successful empty test is provided when the argument is missing.

## Evidence

The first #3461 attempt stopped before registered tests with
`native.configuration-required`. This is an incomplete preflight, not a failed
native assertion. The configuration and host prerequisites above address that
specific prerequisite; compiler findings, registered test execution, security
review and real environment acceptance remain separate gates.

## Program-layer declaration for the migrated scope

`.anatomia/layers.json` classifies the code introduced/reused by this migration;
it does not invent business ownership or classify untouched Rust/UI modules.
Business ownership remains in `spec/domains`.

- `domain`: native version/path invariants and shared trust/capability rules.
- `infrastructure`: local OS IPC and opaque-custody persistence/crypto adapters.
- `application`: verification/gate orchestration, native test drivers and fuzz
  harnesses that exercise the lower layers.

Test and production globs do not overlap. The declared order
allows verification/application to depend on domain and infrastructure, and
domain rules to consume low-level infrastructure. This is the actual current
modular CMake direction, not a claim of a pure onion domain. Analyze resulting
edges rather than treating the declaration as proof of architectural conformance.
Other `.anatomia` caches/state remain ignored; only this reproducible declaration
is tracked.
