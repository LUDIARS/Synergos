#pragma once
#include <string_view>
namespace synergos::core {
// Lexical validation only. Filesystem traversal requires separate handle-based
// symlink/reparse-point checks immediately before opening or replacing a file.
[[nodiscard]] bool is_safe_relative_path(std::string_view path) noexcept;
}
