// @implements SPEC-SY-native-framing
// @decoder ipc.frame
#include "synergos/ipc/frame.hpp"
#include <algorithm>
#include <utility>

namespace synergos::ipc {
std::array<std::uint8_t, 4> frame_header(const std::size_t size) {
    // Validate before narrowing: size_t may be wider than the wire integer.
    if (size > max_message_size) throw FrameError("IPC message exceeds 1 MiB");
    const auto length = static_cast<std::uint32_t>(size);
    return {static_cast<std::uint8_t>(length), static_cast<std::uint8_t>(length >> 8U),
            static_cast<std::uint8_t>(length >> 16U), static_cast<std::uint8_t>(length >> 24U)};
}

DecodeProgress FrameDecoder::consume(const std::span<const std::uint8_t> bytes) {
    if (failed_) throw FrameError("IPC decoder is failed; close the connection");
    if (complete_) return {0, true};
    std::size_t offset = 0;
    while (header_size_ < header_.size() && offset < bytes.size()) {
        header_[header_size_++] = bytes[offset++];
        if (header_size_ == header_.size()) {
            expected_ = static_cast<std::uint32_t>(header_[0])
                | (static_cast<std::uint32_t>(header_[1]) << 8U)
                | (static_cast<std::uint32_t>(header_[2]) << 16U)
                | (static_cast<std::uint32_t>(header_[3]) << 24U);
            if (expected_ > max_message_size) {
                failed_ = true;
                throw FrameError("IPC message exceeds 1 MiB");
            }
        }
    }
    if (header_size_ < header_.size()) return {offset, false};
    // Do not reserve the attacker-declared length before bytes arrive.
    while (offset < bytes.size() && payload_.size() < expected_) {
        const auto count = std::min({read_chunk_size, bytes.size() - offset,
                                    static_cast<std::size_t>(expected_) - payload_.size()});
        payload_.insert(payload_.end(), bytes.begin() + static_cast<std::ptrdiff_t>(offset),
                        bytes.begin() + static_cast<std::ptrdiff_t>(offset + count));
        offset += count;
    }
    complete_ = payload_.size() == expected_;
    return {offset, complete_};
}

std::vector<std::uint8_t> FrameDecoder::take() {
    if (failed_ || !complete_) throw FrameError("IPC frame is not complete");
    auto result = std::move(payload_);
    payload_ = {};
    header_ = {};
    header_size_ = 0;
    expected_ = 0;
    complete_ = false;
    return result;
}

void FrameDecoder::finish() const {
    if (failed_) throw FrameError("IPC decoder is failed");
    if (header_size_ != 0 && !complete_) throw FrameError("Connection closed within IPC frame");
}
}
