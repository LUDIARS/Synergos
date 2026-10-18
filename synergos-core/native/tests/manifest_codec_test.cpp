#include "synergos/core/manifest_codec.hpp"
#include <stdexcept>
#include <string>

using namespace synergos::core;
namespace {
void require(bool value) { if (!value) throw std::runtime_error("Manifest compatibility failed"); }
template<class F> void rejects(F action) {
    try { action(); } catch (const std::exception&) { return; }
    throw std::runtime_error("Invalid metadata accepted");
}
}
int main() {
    const auto fixture = R"({"format":1,"project_id":"p","files":{"assets/a.bin":{"version":7,"size":18446744073709551615,"crc":4294967295,"updated_at":1700000000000,"publisher":"peer-a"}}})";
    const auto manifest = decode_manifest(fixture, "p");
    require(manifest.entries().at("assets/a.bin").version == 7);
    require(manifest.entries().at("assets/a.bin").size == 18446744073709551615ULL);
    const auto encoded = encode_manifest(manifest, "p");
    require(decode_manifest(encoded, "p").entries().at("assets/a.bin").publisher == "peer-a");
    require(decode_manifest(R"({"project_id":""})", "p").entries().empty());
    rejects([&] { static_cast<void>(decode_manifest(fixture, "other")); });
    rejects([] { static_cast<void>(decode_manifest(R"({"format":-1,"project_id":"p"})", "p")); });
    rejects([] { static_cast<void>(decode_manifest(R"({"format":1.0,"project_id":"p"})", "p")); });
    rejects([] { static_cast<void>(decode_manifest(R"({"project_id":"p","files":[]})", "p")); });
    rejects([] { static_cast<void>(decode_version_state(R"({"project_id":"p","versions":{"a":-1}})", "p")); });
    rejects([] { static_cast<void>(decode_version_state(R"({"project_id":"p","versions":{"a":18446744073709551615}})", "p")); });
    rejects([] { static_cast<void>(decode_version_state(R"({"project_id":"p","versions":{"../a":2}})", "p")); });
    rejects([] { static_cast<void>(decode_version_state(R"({"project_id":""})", "p")); });
    const auto state = decode_version_state(R"({"format":1,"project_id":"p","versions":{"a":100}})", "p");
    require(decode_version_state(encode_version_state(state, "p"), "p").highest("a") == 100);
    rejects([] { static_cast<void>(decode_version_state(R"({"project_id":"p","versions":{"a":1,"a":2}})", "p")); });
    rejects([] { static_cast<void>(decode_manifest(R"({"project_id":"p","project_id":"other"})", "p")); });
    rejects([] { static_cast<void>(decode_manifest(R"({"project_id":"p",})", "p")); });
    ManifestVersions unicode;
    unicode.receive("assets/素材.bin", {1, 3, 4, 5, "node\"\\\n"});
    require(decode_manifest(encode_manifest(unicode, "p"), "p").entries().at("assets/素材.bin").publisher == "node\"\\\n");
}
