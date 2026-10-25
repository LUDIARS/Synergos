// @implements SPEC-SY-native-local-transport
#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include "synergos/ipc/local_stream.h"
#include "synergos/ipc/frame.hpp"
#include <algorithm>
#include <string>
#include <utility>
#include <vector>

namespace synergos::ipc {
namespace {
struct Handle final {
    HANDLE value = INVALID_HANDLE_VALUE;
    explicit Handle(HANDLE v) : value(v) {}
    ~Handle() { if (value && value != INVALID_HANDLE_VALUE) CloseHandle(value); }
    Handle(const Handle&) = delete;
    Handle& operator=(const Handle&) = delete;
    Handle(Handle&& other) noexcept : value(std::exchange(other.value, INVALID_HANDLE_VALUE)) {}
};

DWORD remaining(IoDeadline deadline) {
    const auto now = std::chrono::steady_clock::now();
    if (now >= deadline) throw LocalIoError("ipc.timeout");
    const auto count = std::chrono::ceil<std::chrono::milliseconds>(deadline - now).count();
    return static_cast<DWORD>(std::min<std::int64_t>(count, static_cast<std::int64_t>(INFINITE) - 1));
}

std::vector<std::uint8_t> user_sid(HANDLE process) {
    HANDLE raw = nullptr;
    if (!OpenProcessToken(process, TOKEN_QUERY, &raw)) throw LocalIoError("ipc.peer_identity_unavailable");
    const Handle token(raw);
    DWORD size = 0;
    if (GetTokenInformation(token.value, TokenUser, nullptr, 0, &size) || GetLastError() != ERROR_INSUFFICIENT_BUFFER)
        throw LocalIoError("ipc.peer_identity_unavailable");
    std::vector<std::uint8_t> data(size);
    if (!GetTokenInformation(token.value, TokenUser, data.data(), size, &size))
        throw LocalIoError("ipc.peer_identity_unavailable");
    const auto* user = reinterpret_cast<const TOKEN_USER*>(data.data());
    const auto length = GetLengthSid(user->User.Sid);
    std::vector<std::uint8_t> sid(length);
    if (!CopySid(length, sid.data(), user->User.Sid)) throw LocalIoError("ipc.peer_identity_unavailable");
    return sid;
}

void same_user(HANDLE pipe) {
    ULONG pid = 0;
    if (!GetNamedPipeServerProcessId(pipe, &pid)) throw LocalIoError("ipc.peer_identity_unavailable");
    const Handle process(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid));
    if (!process.value) throw LocalIoError("ipc.peer_identity_unavailable");
    const auto mine = user_sid(GetCurrentProcess());
    const auto peer = user_sid(process.value);
    if (!EqualSid(const_cast<std::uint8_t*>(mine.data()), const_cast<std::uint8_t*>(peer.data())))
        throw LocalIoError("ipc.peer_user_mismatch");
}

DWORD transfer(HANDLE pipe, void* data, DWORD length, bool writing, IoDeadline deadline) {
    (void)remaining(deadline);
    const Handle event(CreateEventW(nullptr, TRUE, FALSE, nullptr));
    if (!event.value) throw LocalIoError("ipc.io_error");
    OVERLAPPED operation{};
    operation.hEvent = event.value;
    DWORD count = 0;
    const auto ready = writing ? WriteFile(pipe, data, length, &count, &operation)
                               : ReadFile(pipe, data, length, &count, &operation);
    if (ready) return count;
    const auto error = GetLastError();
    if (!writing && error == ERROR_BROKEN_PIPE) return 0;
    if (error != ERROR_IO_PENDING) throw LocalIoError("ipc.io_error");
    DWORD wait = WAIT_FAILED;
    try { wait = WaitForSingleObject(event.value, remaining(deadline)); }
    catch (...) {
        CancelIoEx(pipe, &operation);
        GetOverlappedResult(pipe, &operation, &count, TRUE);
        throw;
    }
    if (wait != WAIT_OBJECT_0) {
        CancelIoEx(pipe, &operation);
        // The OVERLAPPED and event must outlive cancellation completion.
        GetOverlappedResult(pipe, &operation, &count, TRUE);
        throw LocalIoError(wait == WAIT_TIMEOUT ? "ipc.timeout" : "ipc.io_error");
    }
    if (!GetOverlappedResult(pipe, &operation, &count, FALSE)) {
        if (!writing && GetLastError() == ERROR_BROKEN_PIPE) return 0;
        throw LocalIoError("ipc.io_error");
    }
    return count;
}
}

struct LocalStream::Impl final {
    Handle pipe;
    explicit Impl(Handle&& handle) : pipe(std::move(handle)) {}
};

void validate_local_endpoint(std::string_view endpoint) {
    constexpr std::string_view prefix = R"(\\.\pipe\)";
    if (!endpoint.starts_with(prefix) || endpoint.size() <= prefix.size() || endpoint.size() > 256)
        throw LocalIoError("ipc.invalid_endpoint");
    for (const auto c : endpoint.substr(prefix.size()))
        if (static_cast<unsigned char>(c) <= 0x20 || c == '\\' || c == '/' || c == ':')
            throw LocalIoError("ipc.invalid_endpoint");
}

LocalStream LocalStream::connect(std::string_view endpoint, IoDeadline deadline) {
    validate_local_endpoint(endpoint);
    const auto chars = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, endpoint.data(), static_cast<int>(endpoint.size()), nullptr, 0);
    if (chars == 0) throw LocalIoError("ipc.invalid_endpoint");
    std::wstring name(static_cast<std::size_t>(chars), L'\0');
    if (!MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, endpoint.data(), static_cast<int>(endpoint.size()), name.data(), chars))
        throw LocalIoError("ipc.invalid_endpoint");
    (void)remaining(deadline);
    HANDLE pipe = CreateFileW(name.c_str(), GENERIC_READ | GENERIC_WRITE, 0, nullptr, OPEN_EXISTING,
                              FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION, nullptr);
    while (pipe == INVALID_HANDLE_VALUE && GetLastError() == ERROR_PIPE_BUSY) {
        if (!WaitNamedPipeW(name.c_str(), remaining(deadline)))
            throw LocalIoError(GetLastError() == ERROR_SEM_TIMEOUT ? "ipc.timeout" : "ipc.connect_failed");
        pipe = CreateFileW(name.c_str(), GENERIC_READ | GENERIC_WRITE, 0, nullptr, OPEN_EXISTING,
                           FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION, nullptr);
    }
    if (pipe == INVALID_HANDLE_VALUE) throw LocalIoError("ipc.connect_failed");
    Handle opened(pipe);
    same_user(opened.value);
    auto impl = std::make_unique<Impl>(std::move(opened));
    return LocalStream(std::move(impl));
}

LocalStream::LocalStream(std::unique_ptr<Impl> impl) : impl_(std::move(impl)) {}
LocalStream::~LocalStream() = default;
LocalStream::LocalStream(LocalStream&&) noexcept = default;
LocalStream& LocalStream::operator=(LocalStream&&) noexcept = default;
void LocalStream::close() noexcept { impl_.reset(); }

std::size_t LocalStream::read_some(std::span<std::uint8_t> output, IoDeadline deadline) {
    if (!impl_) throw LocalIoError("ipc.closed");
    if (output.empty()) throw LocalIoError("ipc.empty_read");
    return transfer(impl_->pipe.value, output.data(), static_cast<DWORD>(std::min(output.size(), read_chunk_size)), false, deadline);
}

void LocalStream::write_all(std::span<const std::uint8_t> input, IoDeadline deadline) {
    if (!impl_) throw LocalIoError("ipc.closed");
    while (!input.empty()) {
        const auto size = static_cast<DWORD>(std::min(input.size(), read_chunk_size));
        const auto count = transfer(impl_->pipe.value, const_cast<std::uint8_t*>(input.data()), size, true, deadline);
        if (count == 0) throw LocalIoError("ipc.closed");
        input = input.subspan(count);
    }
}
}
#endif
