#pragma once
#include <cstdint>
#include <map>
#include <string>
#include <string_view>

namespace synergos::core {
class VersionState final {
public:
    [[nodiscard]] std::uint64_t highest(std::string_view path) const;
    bool note(std::string path, std::uint64_t version);
    [[nodiscard]] const std::map<std::string, std::uint64_t, std::less<>>& entries() const noexcept;
private:
    std::map<std::string, std::uint64_t, std::less<>> versions_;
};

}
