#pragma once
#include <array>
#include <cstdint>
#include <string_view>

namespace synergos::custody::detail {
using Digest = std::array<std::uint8_t, 32>;
Digest sha256(std::string_view message);
Digest hmac_sha256(std::string_view secret, std::string_view message);
}
