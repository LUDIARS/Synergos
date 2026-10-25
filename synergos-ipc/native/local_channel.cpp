// @implements SPEC-SY-native-local-transport SPEC-SY-native-framing
#include "synergos/ipc/local_channel.h"
#include <array>
#include <utility>

namespace synergos::ipc {
LocalChannel::LocalChannel(LocalStream stream) : stream_(std::move(stream)) {}

LocalChannel LocalChannel::connect(std::string_view endpoint, IoDeadline deadline) {
    return LocalChannel(LocalStream::connect(endpoint, deadline));
}

void LocalChannel::send(std::span<const std::uint8_t> payload, IoDeadline deadline) {
    const auto header = frame_header(payload.size());
    try {
        stream_.write_all(header, deadline);
        stream_.write_all(payload, deadline);
    } catch (...) {
        // A partial frame must never be reused as the start of another command.
        stream_.close();
        throw;
    }
}

std::vector<std::uint8_t> LocalChannel::receive(IoDeadline deadline) {
    try {
        std::array<std::uint8_t, read_chunk_size> buffer{};
        while (true) {
            if (!pending_.empty()) {
                const auto progress = decoder_.consume(pending_);
                pending_.erase(pending_.begin(), pending_.begin() + static_cast<std::ptrdiff_t>(progress.consumed));
                if (progress.complete) return decoder_.take();
            }
            const auto count = stream_.read_some(buffer, deadline);
            if (count == 0) {
                decoder_.finish();
                throw LocalIoError("ipc.closed");
            }
            const auto progress = decoder_.consume(std::span(buffer).first(count));
            pending_.assign(buffer.begin() + static_cast<std::ptrdiff_t>(progress.consumed),
                            buffer.begin() + static_cast<std::ptrdiff_t>(count));
            if (progress.complete) return decoder_.take();
        }
    } catch (...) {
        stream_.close();
        throw;
    }
}
}
