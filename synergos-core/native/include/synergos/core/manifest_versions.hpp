#pragma once
#include <cstdint>
#include <map>
#include <string>

namespace synergos::core {
struct ManifestEntry {
    std::uint64_t version;
    std::uint64_t size;
    std::uint32_t crc;
    std::uint64_t updated_at;
    std::string publisher;
};
struct BumpOutcome {
    std::uint64_t version;
    bool changed;
};
class ManifestVersions final {
public:
    BumpOutcome bump(std::string path, std::uint64_t size, std::uint32_t crc,
                     std::string publisher, std::uint64_t now_ms, std::uint64_t floor = 0);
    bool receive(std::string path, ManifestEntry entry, bool force = false);
    [[nodiscard]] const std::map<std::string, ManifestEntry, std::less<>>& entries() const noexcept;
private:
    std::map<std::string, ManifestEntry, std::less<>> files_;
};
}
