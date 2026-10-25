// @implements SPEC-SY-native-local-transport
#include "synergos/ipc/local_stream.h"
#include <chrono>
#include <cstdio>
#include <string>
#include <string_view>

using namespace synergos::ipc;
namespace {
void rejects_endpoint(std::string_view endpoint) {
    try { validate_local_endpoint(endpoint); }
    catch (const LocalIoError& error) {
        if (std::string_view(error.what()) == "ipc.invalid_endpoint") return;
        throw;
    }
    throw std::runtime_error("invalid local IPC endpoint accepted");
}
}

int main() {
    try {
#if defined(_WIN32)
        const std::string endpoint = R"(\\.\pipe\synergos-native-contract)";
        validate_local_endpoint(endpoint);
        validate_local_endpoint(R"(\\.\pipe\制作確認)");
        rejects_endpoint(R"(\\server\pipe\synergos)");
        rejects_endpoint(R"(\\?\pipe\synergos)");
        rejects_endpoint(R"(\\.\pipe\)");
        rejects_endpoint(R"(\\.\pipe\nested\name)");
        rejects_endpoint(R"(\\.\pipe\name:stream)");
        rejects_endpoint(R"(\\.\pipe\name with spaces)");
#else
        const std::string endpoint = "/tmp/synergos-native-contract.sock";
        validate_local_endpoint(endpoint);
        rejects_endpoint("relative.sock");
        rejects_endpoint(std::string("/tmp/name\0suffix", 16));
        rejects_endpoint(std::string(1024, '/'));
#endif
        rejects_endpoint("");
        // No daemon/listener: expiry must be checked before the first OS open.
        try { (void)LocalStream::connect(endpoint, std::chrono::steady_clock::now() - std::chrono::seconds(1)); }
        catch (const LocalIoError& error) {
            if (std::string_view(error.what()) == "ipc.timeout") return 0;
            throw;
        }
        throw std::runtime_error("expired connection deadline accepted");
    } catch (const std::exception& error) {
        std::fprintf(stderr, "FAIL: %s\n", error.what());
        return 1;
    }
}
