#include "synergos/core/version_state.hpp"
#include "synergos/core/relative_path.hpp"
#include <algorithm>
#include <limits>
#include <stdexcept>
#include <utility>

namespace synergos::core {
namespace {
constexpr auto invalid_version = std::numeric_limits<std::uint64_t>::max();
void require_path(const std::string_view path) {
    if (!is_safe_relative_path(path)) throw std::invalid_argument("Invalid relative asset path");
}
void require_version(const std::uint64_t version) {
    if (version == 0 || version == invalid_version) throw std::invalid_argument("Invalid asset version");
}
}

std::uint64_t VersionState::highest(const std::string_view path) const {
    const auto it = versions_.find(path);
    return it == versions_.end() ? 0 : it->second;
}
bool VersionState::note(std::string path, const std::uint64_t version) {
    require_path(path);
    require_version(version);
    if (version <= highest(path)) return false;
    versions_.insert_or_assign(std::move(path), version);
    return true;
}
const std::map<std::string, std::uint64_t, std::less<>>& VersionState::entries() const noexcept {
    return versions_;
}

}
