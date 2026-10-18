#pragma once
#include <array>
#include <cstddef>
#include <cstdint>
#include <span>
#include <stdexcept>
#include <vector>

namespace synergos::ipc {
inline constexpr std::uint32_t max_message_size = 1024U * 1024U;
inline constexpr std::size_t read_chunk_size = 64U * 1024U;

class FrameError final : public std::runtime_error {
public:
    using std::runtime_error::runtime_error;
};

// The decoder owns only one frame; callers retain bytes after consumed.
// Payload validation (MessagePack) belongs to the codec, not framing.
struct DecodeProgress {
    std::size_t consumed;
    bool complete;
};

class FrameDecoder final {
public:
    DecodeProgress consume(std::span<const std::uint8_t> bytes);
    [[nodiscard]] std::vector<std::uint8_t> take();
    void finish() const;
private:
    std::array<std::uint8_t, 4> header_{};
    std::size_t header_size_ = 0;
    std::uint32_t expected_ = 0;
    std::vector<std::uint8_t> payload_;
    bool complete_ = false;
    bool failed_ = false;
};

[[nodiscard]] std::array<std::uint8_t, 4> frame_header(std::size_t payload_size);
}
