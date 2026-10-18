# Build-memory milestone, 2026-10-17

The immediate user request is to establish a lower-memory C++ build with minimal
dependencies before expanding the runtime migration. The measured configuration
meets that limited milestone. It does not establish full-application parity.

## Results

Windows x64, MSVC 19.39.33523.0, CMake 4.1.2, Rust/Cargo 1.94.0. Release builds,
one compilation job, new output directories, dependency source downloads already
available. Values below are decimal MB (1,000,000 bytes). Raw counters are in
`build-memory-results.json`.

| Configuration | Peak summed working set | Peak summed private commit | Wall seconds |
|---|---:|---:|---:|
| C++ native libraries + five test executables, yyjson, NMake | 298.9 MB | 400.0 MB | 116.5 |
| Rust synergos-ipc library + dependencies | 486.9 MB | 498.2 MB | 58.5 |

The C++ configuration used 38.6% less sampled working set and 19.7% less private
commit. The C++ target includes framing, version metadata, custody signing and
durable-publication primitives; the Rust target includes the IPC command/event
types and asynchronous client, which the native port does not yet implement.
These are different targets: do not generalize the percentages to the full
application, all platforms, or the languages in isolation.

The first Visual Studio-generator C++ build with nlohmann/json used 510.8 MB
private commit / 410.1 MB working set, so it did not beat Rust on both metrics.
Replacing it with yyjson alone also did not establish the desired result.
The reported result uses the existing NMake build driver as well as the reduced
JSON configuration. The change should not be attributed to the JSON library alone.

## Minimal dependencies

- One JSON dependency, yyjson, replaces nlohmann/json. No second JSON parser is
  retained in the build. JSON5, Pointer/Patch, incremental parsing and custom
  fast floating conversion are disabled; strict UTF-8/integer checks remain.
- Windows hashing uses the OS CNG library. No new crypto or UI framework is added.
- NMake is bundled with the installed compiler; no new build tool is installed.
- Measurement uses Python standard-library ctypes and Windows process APIs;
  no monitoring package is installed.

## Method and reproduction

From an x64 Visual Studio Developer Command Prompt, configure a **new** directory
using the NMake commands in `cpp-toolchain.md`, then run:

```
py -3 scripts/measure-build-memory.py --output cpp-memory.json --log cpp-memory.log --label cpp-native -- cmake --build build-native/nmake
py -3 scripts/measure-build-memory.py --output rust-memory.json --log rust-memory.log --label rust-ipc -- cargo build --manifest-path synergos-ipc/Cargo.toml --release --lib --locked --offline --jobs 1 --target-dir build-native/rust-measurement
```

Output directories must not contain prior build artifacts. Use a distinct empty
directory for each repeat. Rust's missing `tokio/rt` feature declaration was
corrected before the successful measurement; the failed attempt is excluded.

The helper tracks driver/compiler descendants, validates creation times against
parent process IDs, and sums concurrent counters. Handles prevent recycled IDs
from silently substituting unrelated processes. Sampling sleeps nominally 50 ms
between snapshots; enumeration adds overhead (see sample counts). Very short
processes can be missed. RSS may count shared pages more than once. Private
commit includes committed private pages, not just resident RAM. The observer is
excluded. CMake configuration is excluded; the measured C++ wrapper includes
developer-environment setup. Consequently wall time is not a controlled speed
comparison. Each reported configuration has one successful clean-output run.

Five native tests and five Node policy tests pass after the changes. Full Revisor
checks, merge/reflection, Linux/macOS and production runtime remain unverified.
