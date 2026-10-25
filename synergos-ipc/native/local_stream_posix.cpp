// @implements SPEC-SY-native-local-transport
#if !defined(_WIN32)
#include "synergos/ipc/local_stream.h"
#include "synergos/ipc/frame.hpp"
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <poll.h>
#include <fcntl.h>
#include <unistd.h>
#include <algorithm>
#include <cerrno>
#include <climits>
#include <cstring>
#include <string>
#include <utility>

namespace synergos::ipc {
namespace {
struct Descriptor final {
    int value = -1;
    explicit Descriptor(int fd) : value(fd) {}
    ~Descriptor() { if (value >= 0) ::close(value); }
    Descriptor(const Descriptor&) = delete;
    Descriptor& operator=(const Descriptor&) = delete;
    Descriptor(Descriptor&& other) noexcept : value(std::exchange(other.value, -1)) {}
};

int remaining(IoDeadline deadline) {
    const auto now = std::chrono::steady_clock::now();
    if (now >= deadline) throw LocalIoError("ipc.timeout");
    const auto count = std::chrono::ceil<std::chrono::milliseconds>(deadline - now).count();
    return static_cast<int>(std::min<std::int64_t>(count, INT_MAX));
}

void await_ready(int fd, short event, IoDeadline deadline) {
    while (true) {
        pollfd descriptor{fd, event, 0};
        const auto result = ::poll(&descriptor, 1, remaining(deadline));
        if (result < 0 && errno == EINTR) continue;
        if (result == 0) throw LocalIoError("ipc.timeout");
        if (result < 0 || (descriptor.revents & POLLNVAL)) throw LocalIoError("ipc.io_error");
        // HUP/ERR are consumed by recv/send/getsockopt so EOF stays distinct.
        return;
    }
}

void same_user(int fd) {
#if defined(__linux__)
    ucred peer{};
    socklen_t length = sizeof(peer);
    if (::getsockopt(fd, SOL_SOCKET, SO_PEERCRED, &peer, &length) != 0 || length != sizeof(peer))
        throw LocalIoError("ipc.peer_identity_unavailable");
    if (peer.uid != ::geteuid()) throw LocalIoError("ipc.peer_user_mismatch");
#elif defined(__APPLE__)
    uid_t user = 0;
    gid_t group = 0;
    if (::getpeereid(fd, &user, &group) != 0) throw LocalIoError("ipc.peer_identity_unavailable");
    if (user != ::geteuid()) throw LocalIoError("ipc.peer_user_mismatch");
#else
#error Synergos local IPC requires an implemented OS peer credential verifier
#endif
}
}

struct LocalStream::Impl final {
    Descriptor socket;
    explicit Impl(Descriptor&& fd) : socket(std::move(fd)) {}
};

void validate_local_endpoint(std::string_view endpoint) {
    const sockaddr_un address{};
    if (endpoint.empty() || endpoint.front() != '/' || endpoint.size() >= sizeof(address.sun_path)
        || endpoint.find('\0') != std::string_view::npos)
        throw LocalIoError("ipc.invalid_endpoint");
}

LocalStream LocalStream::connect(std::string_view endpoint, IoDeadline deadline) {
    validate_local_endpoint(endpoint);
    (void)remaining(deadline);
    const std::string path(endpoint);
    struct stat metadata{};
    if (::lstat(path.c_str(), &metadata) != 0 || !S_ISSOCK(metadata.st_mode) || metadata.st_uid != ::geteuid())
        throw LocalIoError("ipc.invalid_endpoint_owner");
#if defined(__linux__)
    Descriptor socket(::socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC | SOCK_NONBLOCK, 0));
#else
    Descriptor socket(::socket(AF_UNIX, SOCK_STREAM, 0));
#endif
    if (socket.value < 0) throw LocalIoError("ipc.connect_failed");
    if (::fcntl(socket.value, F_SETFD, FD_CLOEXEC) < 0 || ::fcntl(socket.value, F_SETFL, O_NONBLOCK) < 0)
        throw LocalIoError("ipc.io_error");
#if defined(__APPLE__)
    const int enabled = 1;
    if (::setsockopt(socket.value, SOL_SOCKET, SO_NOSIGPIPE, &enabled, sizeof(enabled)) != 0)
        throw LocalIoError("ipc.io_error");
#endif
    sockaddr_un address{};
    address.sun_family = AF_UNIX;
#if defined(__APPLE__)
    address.sun_len = static_cast<std::uint8_t>(sizeof(address));
#endif
    std::memcpy(address.sun_path, path.c_str(), path.size() + 1);
    const auto result = ::connect(socket.value, reinterpret_cast<const sockaddr*>(&address), sizeof(address));
    if (result != 0) {
        if (errno != EINPROGRESS) throw LocalIoError("ipc.connect_failed");
        await_ready(socket.value, POLLOUT, deadline);
        int error = 0;
        socklen_t length = sizeof(error);
        if (::getsockopt(socket.value, SOL_SOCKET, SO_ERROR, &error, &length) != 0 || error != 0)
            throw LocalIoError("ipc.connect_failed");
    }
    same_user(socket.value);
    return LocalStream(std::make_unique<Impl>(std::move(socket)));
}

LocalStream::LocalStream(std::unique_ptr<Impl> impl) : impl_(std::move(impl)) {}
LocalStream::~LocalStream() = default;
LocalStream::LocalStream(LocalStream&&) noexcept = default;
LocalStream& LocalStream::operator=(LocalStream&&) noexcept = default;
void LocalStream::close() noexcept { impl_.reset(); }

std::size_t LocalStream::read_some(std::span<std::uint8_t> output, IoDeadline deadline) {
    if (!impl_) throw LocalIoError("ipc.closed");
    if (output.empty()) throw LocalIoError("ipc.empty_read");
    while (true) {
        await_ready(impl_->socket.value, POLLIN, deadline);
        const auto count = ::recv(impl_->socket.value, output.data(), std::min(output.size(), read_chunk_size), 0);
        if (count >= 0) return static_cast<std::size_t>(count);
        if (errno != EINTR && errno != EAGAIN && errno != EWOULDBLOCK) throw LocalIoError("ipc.io_error");
    }
}

void LocalStream::write_all(std::span<const std::uint8_t> input, IoDeadline deadline) {
    if (!impl_) throw LocalIoError("ipc.closed");
    while (!input.empty()) {
        await_ready(impl_->socket.value, POLLOUT, deadline);
#if defined(__linux__)
        constexpr int flags = MSG_NOSIGNAL;
#else
        constexpr int flags = 0; // SO_NOSIGPIPE was required at connection time.
#endif
        const auto count = ::send(impl_->socket.value, input.data(), std::min(input.size(), read_chunk_size), flags);
        if (count == 0) throw LocalIoError("ipc.closed");
        if (count < 0) {
            if (errno == EINTR || errno == EAGAIN || errno == EWOULDBLOCK) continue;
            throw LocalIoError("ipc.io_error");
        }
        input = input.subspan(static_cast<std::size_t>(count));
    }
}
}
#endif
