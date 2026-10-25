#pragma once
#include "local_stream.h"
#include "frame.hpp"
#include <vector>

namespace synergos::ipc {
class LocalChannel final {
public:
    static LocalChannel connect(std::string_view endpoint, IoDeadline deadline);
    void send(std::span<const std::uint8_t> payload, IoDeadline deadline);
    std::vector<std::uint8_t> receive(IoDeadline deadline);
private:
    explicit LocalChannel(LocalStream stream);
    LocalStream stream_;
    FrameDecoder decoder_;
    std::vector<std::uint8_t> pending_;
};
}
