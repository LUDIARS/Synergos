// 攻撃ケースの vector を作るために LCT を分解・再組み立てする (テスト専用)。
// 公開 API (mint / attenuate) が拒否する形 (切り詰め・拡大・改ざん) をここで作る。
#pragma once
#include "synergos/trust/crypto.h"
#include <cstdint>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

namespace synergos::trust::test {

struct RawToken {
    std::vector<std::pair<std::string, std::string>> blocks; // (body の JCS 正規形, sig の b64url)
    std::string proof;                                        // proof オブジェクトの JSON
};

RawToken open_token(std::string_view token);
/// JCS 正規化してから base64url にする。
std::string close_token(const RawToken& token);
/// 正規化せずにそのまま base64url にする (非 JCS 形の拒否を試すため)。
std::string close_text(std::string_view json);

/// lct.cpp と同じ形の block body。kid が null なら block n (n>=1) の形。
std::string block_body(std::string_view caveats_json, const PublicKey& next_pub, const char* kid, const char* profile,
                       std::int64_t iat);
std::string unsealed_proof(const Seed& next_seed);

} // namespace synergos::trust::test
