#pragma once
#include "manifest_versions.hpp"
#include "version_state.hpp"
#include <string_view>

namespace synergos::core {
[[nodiscard]] ManifestVersions decode_manifest(std::string_view json, std::string_view project_id);
[[nodiscard]] std::string encode_manifest(const ManifestVersions& manifest, std::string_view project_id);
[[nodiscard]] VersionState decode_version_state(std::string_view json, std::string_view project_id);
[[nodiscard]] std::string encode_version_state(const VersionState& state, std::string_view project_id);
}
