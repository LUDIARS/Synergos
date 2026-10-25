// @implements SPEC-SY-signed-envelope
// @decoder trust.envelope
// Ed25519 の署名関数を呼んでよい唯一の翻訳単位 (scripts/gate/no-direct-sign.mjs が検査する)。
#include "synergos/trust/envelope.h"
#include "synergos/trust/error.h"
#include "synergos/trust/jcs.h"
#include <monocypher-ed25519.h>
#include <array>

namespace synergos::trust {
namespace {

struct Row {
    Ctx ctx;
    std::string_view name;
    std::array<Signer, 2> signers;
    std::size_t count;
};

// 正本: spec/vectors/ctx-registry.json。行の追加・変更は Synergos の Revisor 変更で golden vector と同時に行う。
constexpr std::array<Row, 15> registry{{
    {Ctx::peer_session, "synergos/peer-session", {Signer::node, Signer::node}, 1},
    {Ctx::membership, "synergos/membership", {Signer::cernere, Signer::cernere}, 1},
    {Ctx::gossip, "synergos/gossip", {Signer::node, Signer::node}, 1},
    {Ctx::chain, "synergos/chain", {Signer::node, Signer::node}, 1},
    {Ctx::invite, "synergos/invite", {Signer::node, Signer::node}, 1},
    {Ctx::node_card, "synergos/node-card", {Signer::cernere, Signer::cernere}, 1},
    {Ctx::presence, "synergos/presence", {Signer::node, Signer::node}, 1},
    {Ctx::project_roster, "synergos/project-roster", {Signer::cernere, Signer::cernere}, 1},
    {Ctx::roster_status, "synergos/roster-status", {Signer::cernere, Signer::cernere}, 1},
    {Ctx::heartbeat, "synergos/heartbeat", {Signer::node, Signer::node}, 1},
    {Ctx::enroll, "synergos/enroll", {Signer::node, Signer::node}, 1},
    {Ctx::revocation, "synergos/revocation", {Signer::control_deny, Signer::control_deny}, 1},
    {Ctx::receipt, "synergos/receipt", {Signer::custody, Signer::custody}, 1},
    // lct: block 0 は custody の kid、block n (n>=1) は前 block の next_pub に対応する holder 鍵。
    {Ctx::lct, "synergos/lct", {Signer::custody, Signer::holder}, 2},
    {Ctx::journal_checkpoint, "synergos/journal-checkpoint", {Signer::custody, Signer::curare}, 2},
}};

const Row& row(Ctx ctx) {
    for (const auto& r : registry) if (r.ctx == ctx) return r;
    throw TrustError(Code::envelope_bad_ctx, "ctx not in registry");
}

} // namespace

class EnvelopeSigner {
public:
    static Signature sign(const SigningKey& key, std::string_view message) {
        Signature sig{};
        crypto_ed25519_sign(sig.data(), key.secret_.data(),
                            reinterpret_cast<const std::uint8_t*>(message.data()), message.size());
        return sig;
    }
};

std::string_view ctx_name(Ctx ctx) noexcept {
    for (const auto& r : registry) if (r.ctx == ctx) return r.name;
    return {};
}

std::optional<Ctx> parse_ctx(std::string_view name) noexcept {
    for (const auto& r : registry) if (r.name == name) return r.ctx;
    return std::nullopt;
}

std::string_view signer_name(Signer signer) noexcept {
    switch (signer) {
    case Signer::cernere: return "cernere";
    case Signer::node: return "node";
    case Signer::control_deny: return "control-deny";
    case Signer::custody: return "custody";
    case Signer::curare: return "curare";
    case Signer::holder: return "holder";
    }
    return {};
}

std::optional<Signer> parse_signer(std::string_view name) noexcept {
    for (const auto s : {Signer::cernere, Signer::node, Signer::control_deny, Signer::custody, Signer::curare, Signer::holder})
        if (signer_name(s) == name) return s;
    return std::nullopt;
}

bool signer_allowed(Ctx ctx, Signer signer) noexcept {
    for (const auto& r : registry) {
        if (r.ctx != ctx) continue;
        for (std::size_t i = 0; i < r.count; ++i) if (r.signers[i] == signer) return true;
        return false;
    }
    return false;
}

std::string message(Ctx ctx, std::uint8_t ver, std::string_view payload_json) {
    const auto canonical = jcs::canonicalize(payload_json);
    const auto name = row(ctx).name;
    std::string out;
    out.reserve(name.size() + 2 + canonical.size());
    out.append(name);
    out.push_back('\0');
    out.push_back(static_cast<char>(ver));
    out.append(canonical);
    return out;
}

Signature sign(Ctx ctx, std::uint8_t ver, std::string_view payload_json, Signer signer, const SigningKey& key) {
    if (!signer_allowed(ctx, signer)) throw TrustError(Code::envelope_bad_signer, "signer not allowed for ctx");
    return EnvelopeSigner::sign(key, message(ctx, ver, payload_json));
}

std::string verify(Ctx ctx, std::uint8_t ver, std::string_view payload_json, Signer signer,
                   const PublicKey& key, const Signature& signature) {
    if (!signer_allowed(ctx, signer)) throw TrustError(Code::envelope_bad_signer, "signer not allowed for ctx");
    const auto msg = message(ctx, ver, payload_json);
    if (!ed25519_verify(key, msg, signature)) throw TrustError(Code::envelope_bad_signature, "signature mismatch");
    return msg.substr(row(ctx).name.size() + 2);
}

} // namespace synergos::trust
