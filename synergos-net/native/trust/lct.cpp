// @implements SPEC-SY-lct-v1 SPEC-SY-decode-bounds
// @decoder trust.lct
#include "synergos/trust/lct.h"
#include "synergos/trust/envelope.h"
#include "synergos/trust/error.h"
#include "synergos/trust/jcs.h"
#include "caveats.h"
#include "json_read.h"
#include "profile_rules.h"
#include <yyjson.h>
#include <algorithm>
#include <array>
#include <memory>
#include <utility>
#include <vector>

namespace synergos::trust::lct {
namespace {

using namespace detail;

void bounded(std::size_t declared, std::size_t max, const char* what) {
    if (declared > max) fail(Code::decode_too_large, std::string("trust.lct ") + what + ": " + std::to_string(declared) + " > " + std::to_string(max));
}

std::optional<Profile> parse_profile(std::string_view name) {
    for (const auto p : {Profile::membership, Profile::custody_delegation, Profile::custody_capability})
        if (profile_name(p) == name) return p;
    return std::nullopt;
}

struct Block {
    std::string body_json; // JCS 正規形
    Caveats caveats;
    PublicKey next_pub{};
    Signature sig{};
    std::string kid;
    std::optional<Profile> profile;
    std::int64_t iat{};
};

std::string write(yyjson_val* v) {
    std::size_t len{};
    std::unique_ptr<char, decltype(&std::free)> text(yyjson_val_write(v, 0, &len), std::free);
    if (!text) fail(Code::lct_malformed, "cannot encode block body");
    return jcs::canonicalize({text.get(), len});
}

struct Proof {
    std::optional<Seed> next_secret;
    std::optional<Signature> seal;
};

struct Parsed {
    std::vector<Block> blocks;
    Proof proof;
};

std::vector<Block> parse_blocks(yyjson_val* blocks);

Parsed parse_token(std::string_view token, bool require_sealed = false) {
    bounded(token.size(), max_token_bytes, "token bytes");
    std::vector<std::uint8_t> raw;
    try { raw = base64url_decode(token); } catch (const TrustError&) { fail(Code::lct_malformed, "token is not base64url"); }
    const std::string json(raw.begin(), raw.end());
    std::string canonical;
    try { canonical = jcs::canonicalize(json); } catch (const TrustError& e) { fail(Code::lct_malformed, e.what()); }
    if (canonical != json) fail(Code::lct_malformed, "token JSON is not in JCS form");
    Doc doc(yyjson_read(json.data(), json.size(), 0), yyjson_doc_free);
    auto* root = yyjson_doc_get_root(doc.get());
    auto* v = yyjson_obj_get(root, "v");
    auto* blocks = yyjson_obj_get(root, "blocks");
    auto* proof = yyjson_obj_get(root, "proof");
    if (!yyjson_is_obj(root) || yyjson_obj_size(root) != 3 || !is_int(v) || as_int(v) != 1 || !yyjson_is_arr(blocks)
        || yyjson_arr_size(blocks) == 0 || !yyjson_is_obj(proof) || yyjson_obj_size(proof) != 1)
        fail(Code::lct_malformed, "token must be {v:1, blocks:[...], proof:{...}}");
    bounded(yyjson_arr_size(blocks), max_blocks, "blocks");
    Parsed parsed;
    parsed.blocks = parse_blocks(blocks);
    if (require_sealed && !yyjson_is_str(yyjson_obj_get(proof, "seal")))
        fail(Code::lct_profile_forbidden, "presentation requires a sealed token");
    try {
        if (auto* secret = yyjson_obj_get(proof, "next_secret"); yyjson_is_str(secret))
            parsed.proof.next_secret = base64url_decode_fixed<32>(sv(secret));
        else if (auto* seal = yyjson_obj_get(proof, "seal"); yyjson_is_str(seal))
            parsed.proof.seal = base64url_decode_fixed<64>(sv(seal));
        else
            fail(Code::lct_malformed, "proof must be next_secret or seal");
    } catch (const TrustError& e) {
        if (e.code() == Code::lct_malformed) throw;
        fail(Code::lct_malformed, "bad proof encoding");
    }
    return parsed;
}

std::vector<Block> parse_blocks(yyjson_val* blocks) {
    std::vector<Block> out;
    auto it = yyjson_arr_iter_with(blocks);
    while (auto* item = yyjson_arr_iter_next(&it)) {
        auto* body = yyjson_obj_get(item, "body");
        auto* sig = yyjson_obj_get(item, "sig");
        if (!yyjson_is_obj(item) || yyjson_obj_size(item) != 2 || !yyjson_is_obj(body) || !yyjson_is_str(sig))
            fail(Code::lct_malformed, "block must be {body, sig}");
        const bool first = out.empty();
        const std::size_t expected_keys = first ? 6 : 3;
        auto* bv = yyjson_obj_get(body, "v");
        auto* caveats = yyjson_obj_get(body, "caveats");
        auto* next = yyjson_obj_get(body, "next_pub");
        if (yyjson_obj_size(body) != expected_keys || !is_int(bv) || as_int(bv) != 1 || !yyjson_is_str(next))
            fail(Code::lct_malformed, "unexpected block body fields");
        Block b;
        b.body_json = write(body);
        b.caveats = parse_caveats(caveats);
        try {
            b.next_pub = base64url_decode_fixed<32>(sv(next));
            b.sig = base64url_decode_fixed<64>(sv(sig));
        } catch (const TrustError&) { fail(Code::lct_malformed, "bad key or signature encoding"); }
        if (first) {
            auto* kid = yyjson_obj_get(body, "kid");
            auto* profile = yyjson_obj_get(body, "profile");
            auto* iat = yyjson_obj_get(body, "iat");
            if (!yyjson_is_str(kid) || !yyjson_is_str(profile) || !is_int(iat)) fail(Code::lct_malformed, "block 0 needs kid, profile, iat");
            b.kid = std::string(sv(kid));
            b.profile = parse_profile(sv(profile));
            if (!b.profile) fail(Code::lct_malformed, "unknown profile");
            b.iat = as_int(iat);
        }
        out.push_back(std::move(b));
    }
    return out;
}

using Chain = std::vector<std::pair<std::string, Signature>>;

/// seal 署名の対象: {"seal":[各 block の sig (b64url)]}
std::string seal_payload(const Chain& blocks) {
    std::string json = "{\"seal\":[";
    for (std::size_t i = 0; i < blocks.size(); ++i) {
        if (i) json += ',';
        json += jcs::quote(base64url_encode(blocks[i].second));
    }
    return json + "]}";
}

std::string encode(const Chain& blocks, const std::string& proof_json) {
    std::string json = "{\"blocks\":[";
    for (std::size_t i = 0; i < blocks.size(); ++i) {
        if (i) json += ',';
        json += "{\"body\":" + blocks[i].first + ",\"sig\":" + jcs::quote(base64url_encode(blocks[i].second)) + "}";
    }
    json += "],\"proof\":" + proof_json + ",\"v\":1}";
    const auto canonical = jcs::canonicalize(json);
    auto token = base64url_encode(reinterpret_cast<const std::uint8_t*>(canonical.data()), canonical.size());
    bounded(token.size(), max_token_bytes, "token bytes"); // 受け手が読めない token は作らない
    return token;
}

std::string body(const Caveats& caveats, const PublicKey& next_pub, const std::string* kid, const Profile* profile, std::int64_t iat) {
    std::string json = "{\"caveats\":" + to_json(caveats);
    if (kid) json += ",\"iat\":" + std::to_string(iat) + ",\"kid\":" + jcs::quote(*kid) + ",\"profile\":" + jcs::quote(profile_name(*profile));
    json += ",\"next_pub\":" + jcs::quote(base64url_encode(next_pub)) + ",\"v\":1}";
    return jcs::canonicalize(json);
}

std::string unsealed_proof(const Seed& next_seed) {
    return "{\"next_secret\":" + jcs::quote(base64url_encode(next_seed)) + "}";
}

Chain chain_of(const std::vector<Block>& blocks) {
    Chain out;
    for (const auto& blk : blocks) out.emplace_back(blk.body_json, blk.sig);
    return out;
}

} // namespace

std::string_view profile_name(Profile profile) noexcept {
    switch (profile) {
    case Profile::membership: return "membership";
    case Profile::custody_delegation: return "custody-delegation";
    case Profile::custody_capability: return "custody-capability";
    }
    return {};
}

std::int64_t max_ttl(Profile profile, bool url_form) noexcept {
    switch (profile) {
    case Profile::membership: return 3600;
    case Profile::custody_delegation: return 3600;
    case Profile::custody_capability: return url_form ? 120 : 300;
    }
    return 0;
}

Minted mint(std::string_view kid, const SigningKey& issuer, Profile profile, std::int64_t iat,
            std::string_view caveats_json, const Seed& next_seed) {
    if (profile == Profile::membership) fail(Code::lct_profile_forbidden, "membership LCTs are issued only by Cernere");
    if (kid.empty()) fail(Code::lct_malformed, "kid is required");
    const auto caveats = parse_caveats_text(caveats_json);
    if (!caveats.exp) fail(Code::lct_bad_caveat, "exp is required");
    check_issued(profile, caveats);
    if (profile == Profile::custody_capability) check_presented(profile, caveats);
    const SigningKey next(next_seed);
    const std::string kid_text(kid);
    const auto b = body(caveats, next.public_key(), &kid_text, &profile, iat);
    const auto sig = sign(Ctx::lct, 1, b, Signer::custody, issuer);
    return {encode({{b, sig}}, unsealed_proof(next_seed))};
}

Minted attenuate(std::string_view token, std::string_view caveats_json, const Seed& next_seed) {
    const auto parsed = parse_token(token);
    const auto& blocks = parsed.blocks;
    if (!parsed.proof.next_secret) fail(Code::lct_malformed, "sealed token cannot be attenuated");
    const SigningKey holder(*parsed.proof.next_secret);
    if (holder.public_key() != blocks.back().next_pub) fail(Code::lct_malformed, "proof does not match the last block");
    bounded(blocks.size() + 1, max_blocks, "blocks");
    Caveats effective = blocks.front().caveats;
    for (std::size_t i = 1; i < blocks.size(); ++i) effective = narrow(effective, blocks[i].caveats);
    const auto added = parse_caveats_text(caveats_json);
    narrow(effective, added); // 広げる試みはここで lct.widened
    const SigningKey next(next_seed);
    const auto b = body(added, next.public_key(), nullptr, nullptr, 0);
    auto out = chain_of(blocks);
    out.emplace_back(b, sign(Ctx::lct, 1, b, Signer::holder, holder));
    return {encode(out, unsealed_proof(next_seed))};
}

std::string seal(std::string_view token) {
    const auto parsed = parse_token(token);
    if (!parsed.proof.next_secret) fail(Code::lct_malformed, "token is already sealed");
    const SigningKey holder(*parsed.proof.next_secret);
    if (holder.public_key() != parsed.blocks.back().next_pub) fail(Code::lct_malformed, "proof does not match the last block");
    const auto chain = chain_of(parsed.blocks);
    const auto sig = sign(Ctx::lct, 1, seal_payload(chain), Signer::holder, holder);
    return encode(chain, "{\"seal\":" + jcs::quote(base64url_encode(sig)) + "}");
}

Verified verify(std::string_view token, const std::map<std::string, IssuerKey>& issuers,
                const RevocationView& revocation, std::int64_t now, const Presented& presented) {
    const auto parsed = parse_token(token, presented.require_sealed);
    const auto& blocks = parsed.blocks;
    const auto& head = blocks.front();
    const auto issuer = issuers.find(head.kid);
    if (issuer == issuers.end()) fail(Code::lct_unknown_issuer, "kid is not a known issuer");
    if (issuer->second.profile != *head.profile) fail(Code::lct_profile_forbidden, "kid may not issue this profile");
    if (!presentable(*head.profile, presented.profile)) fail(Code::lct_profile_forbidden, "token cannot be presented as this profile");
    if (presented.url_form && presented.profile != Profile::custody_capability)
        fail(Code::lct_profile_forbidden, "the URL form is only for custody-capability");
    const bool membership = *head.profile == Profile::membership;
    trust::verify(membership ? Ctx::membership : Ctx::lct, 1, head.body_json, membership ? Signer::cernere : Signer::custody,
                  issuer->second.key, head.sig);
    Caveats effective = head.caveats;
    for (std::size_t i = 1; i < blocks.size(); ++i) {
        trust::verify(Ctx::lct, 1, blocks[i].body_json, Signer::holder, blocks[i - 1].next_pub, blocks[i].sig);
        effective = narrow(effective, blocks[i].caveats);
    }
    // 証明: 最後の next_pub の鍵を持つこと。無いと末尾 block を外した (広い) token が通ってしまう。
    if (parsed.proof.next_secret) {
        if (SigningKey(*parsed.proof.next_secret).public_key() != blocks.back().next_pub)
            fail(Code::lct_malformed, "proof does not match the last block");
    } else {
        trust::verify(Ctx::lct, 1, seal_payload(chain_of(blocks)), Signer::holder, blocks.back().next_pub, *parsed.proof.seal);
    }
    if (!effective.exp) fail(Code::lct_bad_caveat, "exp is required");
    if (now >= *effective.exp) fail(Code::lct_expired, "token expired");
    if (effective.nbf && now < *effective.nbf) fail(Code::lct_not_yet_valid, "token not yet valid");
    if (*effective.exp - now > max_ttl(presented.profile, presented.url_form)) fail(Code::lct_ttl_exceeded, "exp exceeds MAX_TTL");
    if (effective.strings.count("download_name") && (!effective.ops || *effective.ops != std::set<std::string>{"get"}))
        fail(Code::lct_bad_caveat, "download_name is valid only for GET");
    check_issued(*head.profile, head.caveats);
    check_presented(presented.profile, effective);
    for (const auto& jti : effective.chain_jti)
        if (revocation.revoked_jti.count(jti)) fail(Code::lct_revoked, "jti revoked");
    if (revocation.cap_issued_before && head.iat < *revocation.cap_issued_before) fail(Code::lct_revoked, "issued before cutoff");
    if (const auto ref = effective.strings.find("project_ref"); ref != effective.strings.end() && revocation.frozen_project_refs.count(ref->second))
        fail(Code::lct_revoked, "project frozen");
    if (effective.projects)
        for (const auto& [ref, role] : *effective.projects)
            if (revocation.frozen_project_refs.count(ref)) fail(Code::lct_revoked, "project frozen");
    const auto delegation_jti = *head.profile == Profile::custody_delegation
        ? std::optional<std::string>{head.caveats.strings.at("jti")} : std::nullopt;
    return {presented.profile, *head.profile, head.kid, head.iat, *effective.exp, to_json(effective), delegation_jti};
}

} // namespace synergos::trust::lct
