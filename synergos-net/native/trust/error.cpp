// @implements SPEC-SY-signed-envelope SPEC-SY-lct-v1 SPEC-SY-custody-receipt-v2 SPEC-SY-decode-bounds
#include "synergos/trust/error.h"

namespace synergos::trust {

std::string_view code_name(Code code) noexcept {
    switch (code) {
    case Code::jcs_invalid_json: return "jcs.invalid_json";
    case Code::jcs_float: return "jcs.float";
    case Code::jcs_duplicate_key: return "jcs.duplicate_key";
    case Code::jcs_too_large: return "jcs.too_large";
    case Code::jcs_too_deep: return "jcs.too_deep";
    case Code::jcs_unsafe_integer: return "jcs.unsafe_integer";
    case Code::encoding_invalid: return "encoding.invalid";
    case Code::envelope_bad_ctx: return "envelope.bad_ctx";
    case Code::envelope_bad_signer: return "envelope.bad_signer";
    case Code::envelope_bad_signature: return "envelope.bad_signature";
    case Code::lct_malformed: return "lct.malformed";
    case Code::lct_unknown_caveat: return "lct.unknown_caveat";
    case Code::lct_bad_caveat: return "lct.bad_caveat";
    case Code::lct_widened: return "lct.widened";
    case Code::lct_expired: return "lct.expired";
    case Code::lct_not_yet_valid: return "lct.not_yet_valid";
    case Code::lct_ttl_exceeded: return "lct.ttl_exceeded";
    case Code::lct_revoked: return "lct.revoked";
    case Code::lct_unknown_issuer: return "lct.unknown_issuer";
    case Code::lct_profile_forbidden: return "lct.profile_forbidden";
    case Code::lct_profile_rule: return "lct.profile_rule";
    case Code::receipt_malformed: return "receipt.malformed";
    case Code::receipt_unknown_kid: return "receipt.unknown_kid";
    case Code::receipt_from_future: return "receipt.from_future";
    case Code::decode_too_large: return "decode.too_large";
    }
    return "unknown";
}

TrustError::TrustError(Code code, const std::string& detail)
    : std::runtime_error(std::string(code_name(code)) + ": " + detail), code_(code) {}

} // namespace synergos::trust
