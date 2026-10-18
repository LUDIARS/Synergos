// @implements SPEC-SY-opaque-custody
#include "synergos/custody/signature.hpp"
#include "digest.hpp"

namespace synergos::custody {
namespace {
std::string hex(const detail::Digest& digest) {
    constexpr char digits[] = "0123456789abcdef";
    std::string result;
    result.reserve(64);
    for (const auto byte : digest) {
        result.push_back(digits[byte >> 4]);
        result.push_back(digits[byte & 15]);
    }
    return result;
}
int nibble(char c) {
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    return -1;
}
}
std::string object_key(std::string_view id) { return hex(detail::sha256(id)); }
std::string sign(std::string_view secret, std::string_view message) {
    return hex(detail::hmac_sha256(secret, message));
}
bool verify(std::string_view secret, std::string_view message, std::string_view signature) {
    if (signature.size() != 64) return false;
    detail::Digest supplied{};
    for (std::size_t i = 0; i < supplied.size(); ++i) {
        const auto high = nibble(signature[2 * i]);
        const auto low = nibble(signature[2 * i + 1]);
        if (high < 0 || low < 0) return false;
        supplied[i] = static_cast<std::uint8_t>((high << 4) | low);
    }
    const auto expected = detail::hmac_sha256(secret, message);
    // Volatile stores prevent an optimizer from replacing this fixed-size
    // comparison with an early-exit memcmp. Encoding validation is public.
    volatile unsigned difference = 0;
    for (std::size_t i = 0; i < expected.size(); ++i) {
        difference = difference | static_cast<unsigned>(expected[i] ^ supplied[i]);
    }
    return difference == 0;
}
}
