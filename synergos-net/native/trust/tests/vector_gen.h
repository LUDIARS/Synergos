// golden vector 生成器の共通部品 (テスト専用)。
#pragma once
#include "synergos/trust/crypto.h"
#include "synergos/trust/error.h"
#include <cstdint>
#include <optional>
#include <stdexcept>
#include <string>

namespace synergos::trust::test {

inline constexpr std::int64_t iat = 1800000000;

inline Seed seed_of(std::uint8_t b) { Seed s; s.fill(b); return s; }

inline std::string at(std::int64_t offset) { return std::to_string(iat + offset); }

/// 意図した結果 (成功 / エラーコード) と実装の結果が違えば生成を止める。
inline void expect_outcome(const std::string& name, std::optional<Code> intended, std::optional<Code> actual) {
    if (intended == actual) return;
    const auto show = [](std::optional<Code> c) { return c ? std::string(code_name(*c)) : std::string("ok"); };
    throw std::runtime_error("vector " + name + ": intended " + show(intended) + " but got " + show(actual));
}

void write_lct(const std::string& dir);
void write_receipt(const std::string& dir);

} // namespace synergos::trust::test
