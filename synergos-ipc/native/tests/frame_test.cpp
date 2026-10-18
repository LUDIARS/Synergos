#include "synergos/ipc/frame.hpp"
#include <algorithm>
#include <array>
#include <limits>
#include <stdexcept>

using namespace synergos::ipc;
namespace {
void require(bool value) { if (!value) throw std::runtime_error("Framing contract failed"); }
template<class F> void rejects(F action) {
    try { action(); } catch (const FrameError&) { return; }
    throw std::runtime_error("Invalid frame accepted");
}
}
int main() {
    // Literal wire fixture, not an encode/decode round trip.
    const std::array<std::uint8_t, 9> wire{5, 0, 0, 0, 0xa4, 'P', 'i', 'n', 'g'};
    const std::vector<std::uint8_t> ping{0xa4, 'P', 'i', 'n', 'g'};
    for (std::size_t split = 0; split <= wire.size(); ++split) {
        FrameDecoder decoder;
        const auto first = decoder.consume(std::span(wire).first(split));
        require(first.consumed == split);
        const auto rest = decoder.consume(std::span(wire).subspan(split));
        require(rest.complete && rest.consumed == wire.size() - split);
        require(decoder.take() == ping);
        decoder.finish();
    }
    FrameDecoder decoder;
    for (auto byte : wire) decoder.consume(std::span(&byte, 1));
    require(decoder.take() == ping);

    std::vector<std::uint8_t> coalesced(wire.begin(), wire.end());
    coalesced.insert(coalesced.end(), wire.begin(), wire.end());
    require(decoder.consume(coalesced).consumed == wire.size());
    require(decoder.take() == ping);
    require(decoder.consume(std::span(coalesced).subspan(wire.size())).complete);
    require(decoder.take() == ping);

    for (std::size_t cut = 1; cut < wire.size(); ++cut) {
        FrameDecoder partial;
        partial.consume(std::span(wire).first(cut));
        rejects([&] { partial.finish(); });
        rejects([&] { static_cast<void>(partial.take()); });
    }
    const std::array<std::uint8_t, 4> too_big{1, 0, 16, 0};
    rejects([&] { decoder.consume(too_big); });
    rejects([&] { decoder.consume(wire); });
    rejects([] { static_cast<void>(frame_header(std::numeric_limits<std::size_t>::max())); });

    const std::array<std::uint8_t, 4> maximum{0, 0, 16, 0};
    require(frame_header(max_message_size) == maximum);
    FrameDecoder large;
    require(!large.consume(maximum).complete);
    std::vector<std::uint8_t> payload(max_message_size, 0xff);
    require(large.consume(payload).complete);
    require(large.take() == payload);
    const std::array<std::uint8_t, 4> zero{};
    require(large.consume(zero).complete);
    require(large.take().empty()); // Codec must reject empty MessagePack separately.
}
