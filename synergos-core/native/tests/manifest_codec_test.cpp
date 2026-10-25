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

    // SPEC-SY-decode-bounds: 長さは解析より前、要素数は要素を読む前に decode.too_large
    const std::string huge(max_manifest_bytes + 1, ' ');
    try {
        static_cast<void>(decode_manifest(huge, "p"));
        require(false);
    } catch (const DecodeTooLarge& e) {
        require(e.decoder_id() == "core.manifest" && e.declared() == max_manifest_bytes + 1 && e.max() == max_manifest_bytes);
    }
    std::string many = R"({"project_id":"p","versions":{)";
    for (std::size_t i = 0; i <= max_version_state_entries; ++i) many += (i ? ",\"f" : "\"f") + std::to_string(i) + "\":1";
    many += "}}";
    try {
        static_cast<void>(decode_version_state(many, "p"));
        require(false);
    } catch (const DecodeTooLarge& e) {
        require(e.decoder_id() == "core.version_state" && e.max() == max_version_state_entries);
    }
}
