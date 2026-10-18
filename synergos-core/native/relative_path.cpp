#include "synergos/core/relative_path.hpp"

namespace synergos::core {
bool is_safe_relative_path(const std::string_view path) noexcept {
    if (path.empty() || path.front() == '/') return false;
    for (const auto byte : path) {
        const auto c = static_cast<unsigned char>(byte);
        if (c < 32U || c == 127U || byte == '\\' || byte == ':') return false;
    }
    std::size_t begin = 0;
    while (begin <= path.size()) {
        auto end = path.find('/', begin);
        if (end == std::string_view::npos) end = path.size();
        const auto part = path.substr(begin, end - begin);
        if (part.empty() || part == "." || part == ".." || (begin == 0 && part == ".synergos")) {
            return false;
        }
        if (end == path.size()) return true;
        begin = end + 1;
    }
    return false;
}
}
