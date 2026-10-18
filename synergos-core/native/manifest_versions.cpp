// @implements SPEC-SY-native-metadata
#include "synergos/core/manifest_versions.hpp"
#include "synergos/core/relative_path.hpp"
#include <algorithm>
#include <limits>
#include <stdexcept>
#include <utility>

namespace synergos::core {
BumpOutcome ManifestVersions::bump(std::string path, const std::uint64_t size,
    const std::uint32_t crc, std::string publisher, const std::uint64_t now_ms,
    const std::uint64_t floor) {
    if (!is_safe_relative_path(path)) throw std::invalid_argument("Invalid relative asset path");
    const auto found = files_.find(path);
    if (found != files_.end() && found->second.size == size && found->second.crc == crc) {
        return {found->second.version, false};
    }
    const auto previous = found == files_.end() ? 0 : found->second.version;
    const auto highest = std::max(previous, floor);
    // The persisted format rejects UINT64_MAX. Leave the old entry intact when
    // no valid next version is representable.
    if (highest >= std::numeric_limits<std::uint64_t>::max() - 1) {
        throw std::overflow_error("Asset version exhausted");
    }
    const auto version = highest + 1;
    files_.insert_or_assign(std::move(path), ManifestEntry{version, size, crc, now_ms, std::move(publisher)});
    return {version, true};
}
bool ManifestVersions::receive(std::string path, ManifestEntry entry, const bool force) {
    if (!is_safe_relative_path(path) || entry.version == 0
        || entry.version == std::numeric_limits<std::uint64_t>::max()) {
        throw std::invalid_argument("Invalid received asset path or version");
    }
    const auto found = files_.find(path);
    if (!force && found != files_.end() && entry.version <= found->second.version) return false;
    files_.insert_or_assign(std::move(path), std::move(entry));
    return true;
}
const std::map<std::string, ManifestEntry, std::less<>>& ManifestVersions::entries() const noexcept {
    return files_;
}
}
