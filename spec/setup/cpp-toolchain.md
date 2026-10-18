# C++ toolchain

Initial verified build environment: CMake 4.1.2; Visual Studio 2022 Community,
MSVC 19.39.33523.0 / tools 14.39.33519; Windows SDK 10.0.19041.0; target x64.
C++20 is required. Native targets compile with warnings as errors, without
compiler language extensions. CMake minimum 3.25 is an interface requirement,
not a claim that every supported compiler/version has been verified.

JSON metadata uses [yyjson 0.12.0](https://github.com/ibireme/yyjson/releases/tag/0.12.0)
(MIT), pinned to upstream commit 8b4a38dc994a110abaec8a400615567bd996105f
and the archive SHA-256 in CMake. Its upstream LICENSE is retained in fetched
source. It replaces nlohmann/json; the two libraries are not both dependencies.
Only JSON reading/writing is required. Non-standard JSON, incremental reading,
JSON Pointer/Patch utilities and custom fast floating conversion are disabled.
UTF-8 validation remains enabled. No sibling-repository dependency is used.
Other transport/crypto/UI dependencies must be pinned and licensed before their
integration. A library build does not establish runtime parity.

Custody SHA-256 and HMAC use Windows CNG (`bcrypt`, supplied by Windows), or
OpenSSL 3 Crypto on Unix. CMake rejects OpenSSL versions below 3. The Unix
implementation and CI matrix have not yet run locally. Record the exact installed
OpenSSL version in release provenance before claiming a reproducible Unix build.
API references: [CNG hashing](https://learn.microsoft.com/en-us/windows/win32/seccng/creating-a-hash-with-cng),
[OpenSSL MAC](https://docs.openssl.org/3.3/man3/EVP_MAC/).

```
cmake -S . -B build-native -G "Visual Studio 17 2022" -A x64 -DBUILD_TESTING=ON
cmake --build build-native --config Release --parallel 4
ctest --test-dir build-native -C Release --output-on-failure
node --test spec/plan/redesign-analysis/workflows/tests/find-verify.test.mjs
```

The above test binaries are finite unit tests, not a daemon/service start.
Service lifecycle and runtime acceptance require the main checkout and Excubitor.

Verified on 2026-10-17: five native CTest cases passed on Windows Release (framing,
version/path rules, manifest/state JSON, SHA-256/HMAC/receipt compatibility,
exclusive durable publication). Node verification-policy tests also passed.
The publication test covers Unicode paths, a binary payload over 2 MiB, empty
files, refusal to overwrite a file/directory, and partial cleanup on failure.
It does not simulate power loss or establish storage-device durability.

There is no production native executable yet. Native peer transport, IPC server,
project lifecycle, user interfaces and the Excubitor service definition remain
required before main-folder execution can meet the user request.

## Dependency budget

Windows native contracts use the C++ standard library, Windows SDK/CNG, and one
C JSON library for exact integer JSON compatibility. OpenSSL is
required only on Unix for the same cryptographic primitives. No GUI toolkit,
network framework, package manager or separate crypto implementation is added.
The Windows build-memory measurement helper uses Python's standard library and
the existing OS process APIs; it installs no monitoring package.

For the lowest measured build-driver overhead on Windows, use an existing x64
Visual Studio Developer Command Prompt and the bundled NMake tool:

```
cmake -S . -B build-native/nmake -G "NMake Makefiles" -DCMAKE_BUILD_TYPE=Release -DBUILD_TESTING=ON
cmake --build build-native/nmake
ctest --test-dir build-native/nmake --output-on-failure
```

NMake runs serially here. No new build package is needed. The Visual Studio
generator remains supported and tested. Measurements and scope limitations are
recorded in `build-memory.md`.
