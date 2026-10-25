#pragma once
#include <chrono>
#include <cstdint>
#include <memory>
#include <span>
#include <stdexcept>
#include <string_view>

namespace synergos::ipc {
using IoDeadline = std::chrono::steady_clock::time_point;

class LocalIoError final : public std::runtime_error {
public:
    using std::runtime_error::runtime_error;
};

// Local, same-OS-user transport only. This does not grant any IPC capability;
// the application must still complete IHELLO before executing commands.
class LocalStream final {
public:
    static LocalStream connect(std::string_view endpoint, IoDeadline deadline);
    ~LocalStream();
    LocalStream(LocalStream&&) noexcept;
    LocalStream& operator=(LocalStream&&) noexcept;
    LocalStream(const LocalStream&) = delete;
    LocalStream& operator=(const LocalStream&) = delete;
    std::size_t read_some(std::span<std::uint8_t> output, IoDeadline deadline);
    void write_all(std::span<const std::uint8_t> input, IoDeadline deadline);
    void close() noexcept;
private:
    struct Impl;
    explicit LocalStream(std::unique_ptr<Impl> impl);
    std::unique_ptr<Impl> impl_;
};

// Pure endpoint validation is also used before the first OS operation.
void validate_local_endpoint(std::string_view endpoint);
}
