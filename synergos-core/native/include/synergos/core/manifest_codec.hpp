#pragma once
#include "manifest_versions.hpp"
#include "version_state.hpp"
#include <cstddef>
#include <stdexcept>
#include <string>
#include <string_view>
#include <utility>

namespace synergos::core {
// SPEC-SY-decode-bounds: 確保より前に長さ、要素を読む前に要素数で拒否する。
inline constexpr std::size_t max_manifest_bytes = 64U * 1024U * 1024U;   // decoder_id core.manifest
inline constexpr std::size_t max_manifest_files = 1'000'000U;
inline constexpr std::size_t max_version_state_bytes = 16U * 1024U * 1024U; // decoder_id core.version_state
inline constexpr std::size_t max_version_state_entries = 1'000'000U;

// decode.too_large{decoder_id, declared, max}
class DecodeTooLarge final : public std::length_error {
public:
    DecodeTooLarge(std::string decoder_id, std::size_t declared, std::size_t max);
    [[nodiscard]] const std::string& decoder_id() const noexcept;
    [[nodiscard]] std::size_t declared() const noexcept;
    [[nodiscard]] std::size_t max() const noexcept;
private:
    std::string decoder_id_;
    std::size_t declared_;
    std::size_t max_;
};

[[nodiscard]] ManifestVersions decode_manifest(std::string_view json, std::string_view project_id);
[[nodiscard]] std::string encode_manifest(const ManifestVersions& manifest, std::string_view project_id);
[[nodiscard]] VersionState decode_version_state(std::string_view json, std::string_view project_id);
[[nodiscard]] std::string encode_version_state(const VersionState& state, std::string_view project_id);
}
