// @implements SPEC-SY-signed-envelope
// 署名は Ed25519(ctx ‖ 0x00 ‖ ver:u8 ‖ JCS(payload)) だけ。ctx は閉じた登録表で、行ごとに署名してよい主体を持つ。
// 登録表の正本は spec/vectors/ctx-registry.json (テストがこの表と一致することを検査する)。
#pragma once
#include "synergos/trust/crypto.h"
#include <cstdint>
#include <optional>
#include <string>
#include <string_view>

namespace synergos::trust {

enum class Ctx {
    peer_session, membership, gossip, chain, invite, node_card, presence, project_roster,
    roster_status, heartbeat, enroll, revocation, receipt, lct, journal_checkpoint,
};

/// 署名主体の種類。membership・node-card・project-roster・roster-status は cernere だけ (オーナー決定 8)。
enum class Signer { cernere, node, control_deny, custody, curare, holder };

std::string_view ctx_name(Ctx ctx) noexcept;        // 例 "synergos/membership"
std::optional<Ctx> parse_ctx(std::string_view name) noexcept;
std::string_view signer_name(Signer signer) noexcept; // 例 "cernere"
std::optional<Signer> parse_signer(std::string_view name) noexcept;
bool signer_allowed(Ctx ctx, Signer signer) noexcept;

/// 署名対象のバイト列。payload_json は JCS で正規化してから連結する。
std::string message(Ctx ctx, std::uint8_t ver, std::string_view payload_json);

/// ctx の行に signer が無ければ envelope.bad_signer。
Signature sign(Ctx ctx, std::uint8_t ver, std::string_view payload_json, Signer signer, const SigningKey& key);

/// 検証に成功したら JCS 正規化済みの payload を返す。失敗は envelope.bad_signer / envelope.bad_signature。
std::string verify(Ctx ctx, std::uint8_t ver, std::string_view payload_json, Signer signer,
                   const PublicKey& key, const Signature& signature);

} // namespace synergos::trust
