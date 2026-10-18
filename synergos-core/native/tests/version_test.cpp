#include "synergos/core/version_state.hpp"
#include "synergos/core/manifest_versions.hpp"
#include "synergos/core/relative_path.hpp"
#include <limits>
#include <stdexcept>

using namespace synergos::core;
namespace {
void require(bool value) { if (!value) throw std::runtime_error("Version contract failed"); }
template<class F> void rejects(F action) {
    try { action(); } catch (const std::invalid_argument&) { return; }
    throw std::runtime_error("Invalid asset accepted");
}
}
int main() {
    for (const auto path : {"", "../x", "a/../x", "/abs", "C:/abs", ".synergos/manifest.json",
                            "a\\b", "a/b:stream", "a/line\nfeed", "a//b", "a/", "./a"}) {
        require(!is_safe_relative_path(path));
    }
    require(!is_safe_relative_path(std::string_view("a\0b", 3)));
    require(is_safe_relative_path("a/b.bin"));
    require(is_safe_relative_path(".hidden/x"));
    require(is_safe_relative_path("素材/試作.bin"));
    VersionState state;
    require(state.note("assets/a.bin", 7));
    require(!state.note("assets/a.bin", 3));
    require(state.highest("assets/a.bin") == 7);
    rejects([&] { state.note("../a", 8); });
    rejects([&] { state.note("assets/a.bin", 0); });
    rejects([&] { state.note("assets/a.bin", std::numeric_limits<std::uint64_t>::max()); });
    ManifestVersions manifest;
    require(manifest.bump("assets/a.bin", 3, 123, "peer-a", 10).version == 1);
    const auto same = manifest.bump("assets/a.bin", 3, 123, "peer-b", 20, 7);
    require(!same.changed && same.version == 1);
    require(manifest.entries().at("assets/a.bin").publisher == "peer-a");
    require(manifest.bump("assets/a.bin", 4, 456, "peer-a", 30, state.highest("assets/a.bin")).version == 8);
    require(!manifest.receive("assets/a.bin", {7, 2, 111, 31, "peer-b"}));
    require(manifest.receive("assets/a.bin", {2, 2, 111, 31, "peer-b"}, true));
    // Restore does not lower the node-local high water mark.
    require(manifest.bump("assets/a.bin", 5, 789, "peer-a", 40, 8).version == 9);
    bool overflow = false;
    try { manifest.bump("assets/a.bin", 6, 999, "peer-a", 50, std::numeric_limits<std::uint64_t>::max()-1); }
    catch (const std::overflow_error&) { overflow = true; }
    require(overflow && manifest.entries().at("assets/a.bin").version == 9);
}
