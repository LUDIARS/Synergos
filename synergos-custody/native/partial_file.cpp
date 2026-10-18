#include "partial_file.hpp"
#include <array>
#include <cstdint>
#include <random>
#include <stdexcept>
#include <string>

namespace synergos::custody::detail {
std::filesystem::path partial_name(const std::filesystem::path& parent) {
    if (parent.empty()) throw std::invalid_argument("Durable file requires a parent directory");
    std::random_device random;
    std::array<std::uint8_t, 16> bytes{};
    for (auto& byte : bytes) byte = static_cast<std::uint8_t>(random());
    bytes[6] = static_cast<std::uint8_t>((bytes[6] & 15) | 64);
    bytes[8] = static_cast<std::uint8_t>((bytes[8] & 63) | 128);
    constexpr char digits[] = "0123456789abcdef";
    std::string name;
    for (std::size_t i = 0; i < bytes.size(); ++i) {
        if (i == 4 || i == 6 || i == 8 || i == 10) name.push_back('-');
        name.push_back(digits[bytes[i] >> 4]);
        name.push_back(digits[bytes[i] & 15]);
    }
    return parent / (name + ".part");
}
}
