#pragma once
#include <cstdint>
#include <string>
#include <string_view>

namespace synergos::custody {
std::string object_key(std::string_view id);
std::string sign(std::string_view secret, std::string_view message);
bool verify(std::string_view secret, std::string_view message, std::string_view signature);
// Custody contracts contain integers only. Reject floats rather than silently
// producing a signature with different numeric formatting from serde_json.
std::string canonical(std::string_view json);
std::string capability_message(std::string_view method, std::string_view key,
                               std::int64_t expires, std::string_view name);
}
