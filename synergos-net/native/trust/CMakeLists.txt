# synergos-net/native/trust (D-24): 信頼の土台 (redesign-security §4.1 SECURITY-M01、フェーズ P1)。
# 署名封筒 (SPEC-SY-signed-envelope)・JCS・LCT v1 (SPEC-SY-lct-v1)。Ed25519 は Monocypher (RFC 8032 の optional 実装)。
FetchContent_Declare(monocypher
  URL https://github.com/LoupVaillant/Monocypher/archive/refs/tags/4.0.2.tar.gz
  URL_HASH SHA256=bc1ca30b1b2654e4e7daf2492c0d204200e55137f23fda6b7142fd7d523bd6b4
  DOWNLOAD_EXTRACT_TIMESTAMP TRUE)
FetchContent_MakeAvailable(monocypher)

# 外部コードなので synergos_native_options (警告をエラー扱い) は掛けない。
add_library(synergos_monocypher STATIC
  ${monocypher_SOURCE_DIR}/src/monocypher.c
  ${monocypher_SOURCE_DIR}/src/optional/monocypher-ed25519.c)
target_include_directories(synergos_monocypher PUBLIC ${monocypher_SOURCE_DIR}/src ${monocypher_SOURCE_DIR}/src/optional)

add_library(synergos_trust_native STATIC
  error.cpp jcs.cpp crypto.cpp envelope.cpp caveats.cpp profile_rules.cpp lct.cpp receipt.cpp)
target_include_directories(synergos_trust_native PUBLIC include)
target_link_libraries(synergos_trust_native PUBLIC synergos_native_options PRIVATE yyjson synergos_monocypher)
if(WIN32)
  target_link_libraries(synergos_trust_native PRIVATE bcrypt)
endif()

if(BUILD_TESTING)
  add_executable(synergos_trust_test tests/trust_test.cpp tests/vectors.cpp tests/vector_gen.cpp tests/vector_gen_lct.cpp
    tests/vector_gen_receipt.cpp tests/token_surgery.cpp)
  target_link_libraries(synergos_trust_test PRIVATE synergos_trust_native yyjson)
  add_test(NAME native.trust COMMAND synergos_trust_test "${PROJECT_SOURCE_DIR}/spec/vectors")
endif()
