// @implements SPEC-SY-lct-v1
// LUDIARS Capability Token v1。block の鎖で、後の block は権限を狭めることしかできない (積集合)。
//
// 符号化: token = base64url(JCS({"v":1,"blocks":[{"body":<body>,"sig":<b64url>}...],"proof":<proof>}))
//   proof = {"next_secret":<b64url seed>} (未封印。受け取った者がさらに狭められる)
//         | {"seal":<b64url sig>}         (封印済み。最後の next_pub の鍵で {"seal":[各 block の sig]} に署名、ctx lct)
//   proof は最後の block の next_pub に対応する鍵を持つことの証明で、block の切り詰め (末尾を外して
//   権限を広げる攻撃) を検証で拒否する (Biscuit と同じ構成)。
//   body = {"v":1, "caveats":{...}, "next_pub":<b64url 32B>}
//          block 0 だけ "kid" (発行鍵 id)、"profile"、"iat" (発行時刻 epoch 秒) を持つ。
//   block 0 の署名: profile が membership なら ctx membership を Cernere の kid で、
//                   それ以外は ctx lct を custody の kid で (SPEC-SY-signed-envelope の登録表)。
//   block n (n>=1) の署名: ctx lct を、block n-1 の next_pub に対応する holder 鍵で。
// 保持者は最後の next_pub の seed だけを持つ。seal() でそれを消すと token は封じられる (それ以上狭められない)。
// PEP へ提示するときは封印済みにする。
#pragma once
#include "synergos/trust/crypto.h"
#include <cstddef>
#include <cstdint>
#include <map>
#include <optional>
#include <set>
#include <string>
#include <string_view>

namespace synergos::trust::lct {

enum class Profile { membership, custody_delegation, custody_capability };

std::string_view profile_name(Profile profile) noexcept;

/// MAX_TTL (秒): membership 3600、custody-delegation 3600、custody-capability 300 (URL 形式は 120)。
std::int64_t max_ttl(Profile profile, bool url_form) noexcept;

/// SPEC-SY-decode-bounds (decoder_id trust.lct): 確保より前に長さ、decode 中に要素数を切る。超えたら decode.too_large。
inline constexpr std::size_t max_token_bytes = 16 * 1024; // base64url の文字数
inline constexpr std::size_t max_blocks = 16;
inline constexpr std::size_t max_caveat_items = 256; // ops / orgs / object_keys / projects の各要素数

/// PEP が何として受け取るか。custody-capability は custody-delegation を狭めたものでもよく、
/// そのときも MAX_TTL と規則は提示 profile のものを使う。
struct Presented {
    Profile profile;
    bool url_form = false; // ブラウザ GET の URL 形式 (custody-capability だけ、TTL 120 秒)
    bool require_sealed = false; // Runtime PEPs opt in; codec/holder inspection still accepts unsealed proof.
};

struct Minted {
    std::string token; // 未封印 (proof に next_secret を含む)。保持者だけが持つ。
};

struct IssuerKey {
    PublicKey key;
    Profile profile; // この kid が発行してよい profile
};

/// 失効の参照。jti 単位、発行時刻のしきい値、凍結されたプロジェクト。
struct RevocationView {
    std::set<std::string> revoked_jti;
    std::optional<std::int64_t> cap_issued_before; // iat がこれより前の token は失効
    std::set<std::string> frozen_project_refs;
};

struct Verified {
    Profile profile;        // 提示 profile
    Profile issued_profile; // block 0 の profile
    std::string kid;
    std::int64_t iat;
    std::int64_t exp;
    std::string caveats_json; // 全 block の積集合 (JCS 正規形)
    std::optional<std::string> delegation_jti{}; // Root delegation label for the redemption budget owner.
};

/// custody-delegation / custody-capability を発行する。membership は lct.profile_forbidden
/// (Synergos のどの部品も membership LCT を発行しない、オーナー決定 8)。
/// caveats_json は閉じた語彙だけ。exp は必須。next_seed は検証用に固定できる (通常は random_seed())。
Minted mint(std::string_view kid, const SigningKey& issuer, Profile profile, std::int64_t iat,
            std::string_view caveats_json, const Seed& next_seed);

/// 保持者が caveats を足して狭める。広げようとすると lct.widened。封印済みの token は lct.malformed。
Minted attenuate(std::string_view token, std::string_view caveats_json, const Seed& next_seed);

/// 未封印の token を封印する (next_secret を捨て、鎖全体への seal 署名に置き換える)。
std::string seal(std::string_view token);

/// 署名の鎖・閉じた語彙・積集合・期限・MAX_TTL・profile の規則・失効を検査する。失敗は型付きの TrustError。
/// 提示 profile として受け取れない token は lct.profile_forbidden。
Verified verify(std::string_view token, const std::map<std::string, IssuerKey>& issuers,
                const RevocationView& revocation, std::int64_t now, const Presented& presented);

} // namespace synergos::trust::lct
