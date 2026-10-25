cmake_minimum_required(VERSION 3.25)
project(SynergosNative VERSION 0.1.0 LANGUAGES C CXX)
set(CMAKE_CXX_EXTENSIONS OFF)
include(CTest)
include(FetchContent)
set(YYJSON_BUILD_TESTS OFF CACHE BOOL "" FORCE)
set(YYJSON_BUILD_MISC OFF CACHE BOOL "" FORCE)
set(YYJSON_DISABLE_NON_STANDARD ON CACHE BOOL "" FORCE)
set(YYJSON_DISABLE_UTILS ON CACHE BOOL "" FORCE)
set(YYJSON_DISABLE_INCR_READER ON CACHE BOOL "" FORCE)
set(YYJSON_DISABLE_FAST_FP_CONV ON CACHE BOOL "" FORCE)
set(YYJSON_INSTALL OFF CACHE BOOL "" FORCE)
FetchContent_Declare(yyjson
  URL https://github.com/ibireme/yyjson/archive/8b4a38dc994a110abaec8a400615567bd996105f.tar.gz
  URL_HASH SHA256=94e9c90f8f12d8329f2c0756782d21e5c278729023cd2c5fe47e323a35947ed6
  DOWNLOAD_EXTRACT_TIMESTAMP TRUE)
FetchContent_MakeAvailable(yyjson)

add_library(synergos_native_options INTERFACE)
target_compile_features(synergos_native_options INTERFACE cxx_std_20)
if(MSVC)
  target_compile_options(synergos_native_options INTERFACE /W4 /WX /permissive- /utf-8)
else()
  target_compile_options(synergos_native_options INTERFACE -Wall -Wextra -Wpedantic -Werror)
endif()

# fuzz 用のビルド (fuzz/CMakeLists.txt・scripts/gate/fuzz-run.mjs)。全体に ASan と網羅計測を付け、
# libFuzzer 本体 (main) は fuzz target だけに付ける。通常のビルドでは OFF。
option(SYNERGOS_FUZZ "Build libFuzzer targets with ASan and coverage on every native library" OFF)
if(SYNERGOS_FUZZ)
  if(MSVC)
    add_compile_options(/fsanitize=address /Zi /fsanitize-coverage=inline-8bit-counters /fsanitize-coverage=edge
                        /fsanitize-coverage=trace-cmp /fsanitize-coverage=trace-div)
    add_link_options(/INCREMENTAL:NO /DEBUG)
  else()
    add_compile_options(-fsanitize=address,fuzzer-no-link -g)
    add_link_options(-fsanitize=address)
  endif()
endif()

add_subdirectory(synergos-ipc/native)
add_subdirectory(synergos-core/native)
add_subdirectory(synergos-custody/native)
add_subdirectory(synergos-net/native/trust)
if(BUILD_TESTING)
  add_subdirectory(fuzz)
endif()
