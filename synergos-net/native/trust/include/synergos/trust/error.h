// @implements SPEC-SY-signed-envelope SPEC-SY-lct-v1
// 検証失敗は型付きのコードで返し、文字列で分類しない (redesign-security §4.1 失敗時)。
#pragma once
#include <stdexcept>
#include <string>
#include <string_view>

namespace synergos::trust {

enum class Code {
    jcs_invalid_json,
    jcs_float,
    jcs_duplicate_key,
    jcs_too_large,
    jcs_too_deep,
    jcs_unsafe_integer,
    encoding_invalid,
    envelope_bad_ctx,
    envelope_bad_signer,
    envelope_bad_signature,
    lct_malformed,
    lct_unknown_caveat,
    lct_bad_caveat,
    lct_widened,
    lct_expired,
    lct_not_yet_valid,
    lct_ttl_exceeded,
    lct_revoked,
    lct_unknown_issuer,
    lct_profile_forbidden,
    lct_profile_rule,    // profile ごとの規則 (custody-delegation / custody-capability) に合わない
    receipt_malformed,
    receipt_unknown_kid,
    receipt_from_future, // iat > now + 120 秒
    decode_too_large,    // SPEC-SY-decode-bounds: 確保より前に長さ・要素数の上限で拒否
};

/// 安定した外部名 (例 "lct.expired")。golden vector とイベントはこの名前で照合する。
std::string_view code_name(Code code) noexcept;

class TrustError : public std::runtime_error {
public:
    TrustError(Code code, const std::string& detail);
    Code code() const noexcept { return code_; }

private:
    Code code_;
};

} // namespace synergos::trust
