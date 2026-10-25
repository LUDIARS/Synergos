// @implements SPEC-SY-signed-envelope
// RFC 8785 (JCS) の正規化。署名対象の唯一の正規化関数 (SEC-CR-12 の三重正規化をここに畳む)。
#pragma once
#include <cstdint>
#include <string>
#include <string_view>

namespace synergos::trust::jcs {

inline constexpr std::size_t max_bytes = 1024 * 1024;
inline constexpr unsigned max_depth = 128;
/// JS (Curare / Cernere の TS 実装) と同値になる整数の範囲。超えた値は拒否する。
inline constexpr std::int64_t max_safe_integer = 9007199254740991; // 2^53 - 1

/// JSON テキストを JCS 正規形に変換する。
/// 浮動小数・重複キー・1 MiB 超・128 段超・安全範囲外の整数・不正な JSON/UTF-8 は TrustError。
std::string canonicalize(std::string_view json);

/// 文字列 1 つを JCS の規則で引用符付きの JSON 文字列にする。
std::string quote(std::string_view utf8);

} // namespace synergos::trust::jcs
