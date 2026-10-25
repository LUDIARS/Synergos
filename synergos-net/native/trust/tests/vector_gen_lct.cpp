// lct.json の生成。発行・狭め・封印は公開 API で作り、API が拒否する形 (切り詰め・拡大・改ざん) は
// token_surgery で直接組み立てる。
#include "json_writer.h"
#include "token_surgery.h"
#include "vector_gen.h"
#include "synergos/trust/envelope.h"
#include "synergos/trust/lct.h"
#include <vector>

namespace synergos::trust::test {
namespace {

using lct::Profile;

struct Issuer { const char* kid; Profile profile; std::uint8_t seed; };
const std::vector<Issuer> issuers{
    {"custody-cap-1", Profile::custody_capability, 0x11},
    {"custody-del-1", Profile::custody_delegation, 0x12},
    {"cernere-mem-1", Profile::membership, 0x13},
};

struct LctCase {
    std::string name;
    std::string token;
    Profile presented;
    std::optional<Code> intended;
    std::int64_t now;
    bool url_form;
    lct::RevocationView revocation;
};

LctCase make(std::string name, std::string token, Profile presented, std::optional<Code> intended = std::nullopt,
             std::int64_t now = iat + 10, bool url_form = false, lct::RevocationView revocation = {}) {
    return LctCase{std::move(name), std::move(token), presented, intended, now, url_form, std::move(revocation)};
}

lct::RevocationView revoked(std::set<std::string> jti, std::optional<std::int64_t> cutoff, std::set<std::string> frozen) {
    lct::RevocationView v;
    v.revoked_jti = std::move(jti);
    v.cap_issued_before = cutoff;
    v.frozen_project_refs = std::move(frozen);
    return v;
}

/// custody が直接発行する GET capability の caveats。extra は先頭に ',' を付けて足す。
std::string cap_json(std::int64_t exp_offset, const std::string& extra = "") {
    return R"({"digest":"sha256:ab","download_name":"report.pdf","exp":)" + at(exp_offset)
        + R"(,"jti":"cap-1","object_keys":["proj-a/report"],"ops":["get"],"org":"org-a","project_ref":"proj-a","session_id":"s-1","sub":"user-1")"
        + extra + "}";
}

const std::string del_json = R"({"exp":)" + at(3000)
    + R"(,"iss":"custody:hq","jti":"del-1","max_bytes_per_hour":1000000000,"ops":["get","head","put"],"orgs":["org-a","org-b"],)"
      R"("projects":[{"project_ref":"proj-a","role":"publish"},{"project_ref":"proj-a","role":"read"}],"sub":"curare:d1"})";

const std::string get_from_del = R"({"exp":)" + at(200)
    + R"(,"jti":"cap-2","ops":["get"],"org":"org-a","prefix":"proj-a/assets/","project_ref":"proj-a","session_id":"s-1","sub":"curare:d1/user-1"})";

const std::string put_from_del = R"({"exp":)" + at(200)
    + R"(,"jti":"cap-3","max_bytes":1000000,"object_keys":["upload:u-1"],"ops":["put"],"org":"org-a","project_ref":"proj-a","session_id":"s-1","sub":"curare:d1/user-1"})";

const std::string membership_json = R"({"exp":)" + at(3600)
    + R"(,"iss":"cernere","org":"ludiars","sub_peer_id":"peer-1","node_id":"node-1",)"
      R"("principal":"principal-1","device_id":"device-1","roster_epoch":7,"jti":"membership-1",)"
      R"("projects":[{"project_ref":"proj-a","role":"read"}]})";

std::string mint_by(std::uint8_t issuer_seed, const char* kid, Profile profile, const std::string& caveats, std::uint8_t next) {
    return lct::mint(kid, SigningKey(seed_of(issuer_seed)), profile, iat, caveats, seed_of(next)).token;
}

/// 公開 API では作れない (mint が拒否する) block 0 を、発行鍵で直接署名して作る。
std::string forged_head(std::uint8_t issuer_seed, Ctx ctx, Signer signer, const char* kid, const char* profile,
                        const std::string& caveats, std::uint8_t next) {
    const auto body = block_body(caveats, SigningKey(seed_of(next)).public_key(), kid, profile, iat);
    RawToken t;
    t.blocks.emplace_back(body, base64url_encode(sign(ctx, 1, body, signer, SigningKey(seed_of(issuer_seed)))));
    t.proof = unsealed_proof(seed_of(next));
    return close_token(t);
}

std::string forged_cap(const std::string& caveats) {
    return forged_head(0x11, Ctx::lct, Signer::custody, "custody-cap-1", "custody-capability", caveats, 0x21);
}

/// 保持者の鍵で、狭めていない (広げた) block を足す。attenuate は拒否するので直接作る。
std::string append_raw(const std::string& token, std::uint8_t holder_seed, const std::string& caveats, std::uint8_t next) {
    auto t = open_token(token);
    const auto body = block_body(caveats, SigningKey(seed_of(next)).public_key(), nullptr, nullptr, 0);
    t.blocks.emplace_back(body, base64url_encode(sign(Ctx::lct, 1, body, Signer::holder, SigningKey(seed_of(holder_seed)))));
    t.proof = unsealed_proof(seed_of(next));
    return close_token(t);
}

std::string drop_last_block(const std::string& token) {
    auto t = open_token(token);
    t.blocks.pop_back();
    return close_token(t);
}

std::string replace_in_body(const std::string& token, const std::string& from, const std::string& to) {
    auto t = open_token(token);
    auto& body = t.blocks.front().first;
    body.replace(body.find(from), from.size(), to);
    return close_token(t);
}

/// 何も足さない block を重ねて max_blocks を超えた鎖を作る (attenuate は上限で止まるので直接)。
std::string too_many_blocks(const std::string& token, std::uint8_t holder_seed) {
    auto out = token;
    auto holder = holder_seed;
    for (std::size_t i = 1; i <= lct::max_blocks; ++i) {
        const auto next = static_cast<std::uint8_t>(0x40 + i);
        out = append_raw(out, holder, "{}", next);
        holder = next;
    }
    return out;
}

std::string many_ops() {
    std::string ops;
    for (std::size_t i = 0; i <= lct::max_caveat_items; ++i) ops += (i ? ",\"op" : "\"op") + std::to_string(1000 + i) + "\"";
    return R"({"exp":)" + at(200) + R"(,"ops":[)" + ops + "]}";
}

std::vector<LctCase> lct_cases() {
    const auto cap = mint_by(0x11, "custody-cap-1", Profile::custody_capability, cap_json(200), 0x21);
    const auto del = mint_by(0x12, "custody-del-1", Profile::custody_delegation, del_json, 0x22);
    const auto get2 = lct::attenuate(del, get_from_del, seed_of(0x23)).token;
    const auto put2 = lct::attenuate(del, put_from_del, seed_of(0x24)).token;
    const auto C = Profile::custody_capability;
    const auto D = Profile::custody_delegation;
    const auto M = Profile::membership;

    std::vector<LctCase> out;
    out.push_back(make("capability-unsealed", cap, C));
    out.push_back(make("capability-sealed", lct::seal(cap), C));
    out.push_back(make("delegation-sealed", lct::seal(del), D));
    out.push_back(make("capability-get-from-delegation", lct::seal(get2), C));
    out.push_back(make("capability-put-from-delegation", lct::seal(put2), C));
    out.push_back(make("membership-by-cernere",
                       forged_head(0x13, Ctx::membership, Signer::cernere, "cernere-mem-1", "membership", membership_json, 0x25), M));
    out.push_back(make("membership-missing-authority-bindings",
                       forged_head(0x13, Ctx::membership, Signer::cernere, "cernere-mem-1", "membership",
                                   R"({"exp":)" + at(3600) + R"(,"org":"ludiars","sub":"user-1"})", 0x25),
                       M, Code::lct_profile_rule));
    const auto membership = forged_head(0x13, Ctx::membership, Signer::cernere,
                                       "cernere-mem-1", "membership", membership_json, 0x25);
    out.push_back(make("membership-roster-epoch-replaced",
                       append_raw(membership, 0x25, R"({"roster_epoch":8})", 0x26), M, Code::lct_widened));
    out.push_back(make("membership-peer-replaced",
                       append_raw(membership, 0x25, R"({"sub_peer_id":"peer-2"})", 0x26), M, Code::lct_widened));
    out.push_back(make("truncated-chain-sealed", drop_last_block(lct::seal(get2)), C, Code::envelope_bad_signature));
    out.push_back(make("truncated-chain-unsealed", drop_last_block(get2), C, Code::lct_malformed));
    out.push_back(make("widened-ops", append_raw(del, 0x22, R"({"ops":["delete","get"]})", 0x26), D, Code::lct_widened));
    out.push_back(make("widened-exp", append_raw(del, 0x22, R"({"exp":)" + at(4000) + "}", 0x26), D, Code::lct_widened));
    out.push_back(make("widened-orgs", append_raw(del, 0x22, R"({"orgs":["org-a","org-c"]})", 0x26), D, Code::lct_widened));
    out.push_back(make("org-outside-orgs", append_raw(del, 0x22, R"({"org":"org-c"})", 0x26), D, Code::lct_widened));
    out.push_back(make("sub-not-extended", append_raw(del, 0x22, R"({"sub":"curare:d2/user-1"})", 0x26), D, Code::lct_widened));
    out.push_back(make("expired", lct::seal(cap), C, Code::lct_expired, iat + 200));
    out.push_back(make("not-yet-valid", mint_by(0x11, "custody-cap-1", C, cap_json(200, R"(,"nbf":)" + at(50)), 0x21), C,
                       Code::lct_not_yet_valid));
    out.push_back(make("ttl-exceeded", mint_by(0x11, "custody-cap-1", C, cap_json(400), 0x21), C, Code::lct_ttl_exceeded));
    out.push_back(make("url-form-ttl", lct::seal(cap), C, Code::lct_ttl_exceeded, iat + 10, true));
    out.push_back(make("delegation-presented-as-capability", lct::seal(del), C, Code::lct_ttl_exceeded));
    out.push_back(make("capability-presented-as-delegation", lct::seal(cap), D, Code::lct_profile_forbidden));
    out.push_back(make("url-form-for-delegation", lct::seal(del), D, Code::lct_profile_forbidden, iat + 10, true));
    out.push_back(make("download-name-traversal",
                       forged_cap(R"({"download_name":"../etc/passwd","exp":)" + at(200) + R"(,"ops":["get"]})"), C, Code::lct_bad_caveat));
    out.push_back(make("download-name-with-put",
                       forged_cap(R"({"download_name":"a.txt","exp":)" + at(200) + R"(,"ops":["get","put"]})"), C, Code::lct_bad_caveat));
    out.push_back(make("capability-put-without-upload-key",
                       forged_cap(R"({"exp":)" + at(200) + R"(,"jti":"cap-9","max_bytes":10,"object_keys":["proj-a/x"],"ops":["put"],)"
                                  R"("org":"org-a","project_ref":"proj-a","session_id":"s-1","sub":"user-1"})"),
                       C, Code::lct_profile_rule));
    out.push_back(make("capability-without-session",
                       forged_cap(R"({"exp":)" + at(200) + R"(,"jti":"cap-9","object_keys":["proj-a/x"],"ops":["get"],)"
                                  R"("org":"org-a","project_ref":"proj-a","sub":"user-1"})"),
                       C, Code::lct_profile_rule));
    out.push_back(make("delegation-without-orgs",
                       forged_head(0x12, Ctx::lct, Signer::custody, "custody-del-1", "custody-delegation",
                                   R"({"exp":)" + at(3000) + R"(,"iss":"custody:hq","jti":"del-9","max_bytes_per_hour":1,"ops":["get"],"sub":"curare:d1"})",
                                   0x22),
                       D, Code::lct_profile_rule));
    out.push_back(make("unknown-caveat", forged_cap(R"({"color":"red","exp":)" + at(200) + "}"), C, Code::lct_unknown_caveat));
    out.push_back(make("unknown-issuer", mint_by(0x11, "custody-cap-9", C, cap_json(200), 0x21), C, Code::lct_unknown_issuer));
    out.push_back(make("profile-not-allowed-for-kid", mint_by(0x12, "custody-del-1", C, cap_json(200), 0x21), C,
                       Code::lct_profile_forbidden));
    out.push_back(make("revoked-jti", lct::seal(cap), C, Code::lct_revoked, iat + 10, false, revoked({"cap-1"}, std::nullopt, {})));
    out.push_back(make("revoked-parent-jti", lct::seal(get2), C, Code::lct_revoked, iat + 10, false, revoked({"del-1"}, std::nullopt, {})));
    out.push_back(make("issued-before-cutoff", lct::seal(cap), C, Code::lct_revoked, iat + 10, false, revoked({}, iat + 1, {})));
    out.push_back(make("frozen-project", lct::seal(get2), C, Code::lct_revoked, iat + 10, false, revoked({}, std::nullopt, {"proj-a"})));
    out.push_back(make("membership-signed-as-lct",
                       forged_head(0x13, Ctx::lct, Signer::custody, "cernere-mem-1", "membership", membership_json, 0x25), M,
                       Code::envelope_bad_signature));
    out.push_back(make("tampered-caveat", replace_in_body(lct::seal(cap), "report.pdf", "secret.pdf"), C, Code::envelope_bad_signature));
    out.push_back(make("non-jcs-token", close_text(R"({"v":1, "blocks":[], "proof":{}})"), C, Code::lct_malformed));
    out.push_back(make("too-many-blocks", too_many_blocks(del, 0x22), D, Code::decode_too_large));
    out.push_back(make("token-too-large", std::string(lct::max_token_bytes + 1, 'A'), C, Code::decode_too_large));
    out.push_back(make("too-many-caveat-items", forged_cap(many_ops()), C, Code::decode_too_large));
    return out;
}

std::map<std::string, lct::IssuerKey> issuer_map() {
    std::map<std::string, lct::IssuerKey> out;
    for (const auto& i : issuers) out.emplace(i.kid, lct::IssuerKey{SigningKey(seed_of(i.seed)).public_key(), i.profile});
    return out;
}

yyjson_mut_val* string_array(JsonWriter& w, const std::set<std::string>& items) {
    auto* a = w.arr();
    for (const auto& s : items) w.push(a, w.str(s));
    return a;
}

} // namespace

void write_lct(const std::string& dir) {
    JsonWriter w;
    auto* root = w.obj();
    auto* issuer_list = w.arr();
    for (const auto& i : issuers) {
        auto* item = w.obj();
        w.put(item, "kid", w.str(i.kid));
        w.put(item, "profile", w.str(lct::profile_name(i.profile)));
        w.put(item, "public_key", w.str(base64url_encode(SigningKey(seed_of(i.seed)).public_key())));
        w.push(issuer_list, item);
    }
    w.put(root, "issuers", issuer_list);
    const auto keys = issuer_map();
    auto* list = w.arr();
    for (const auto& c : lct_cases()) {
        std::optional<Code> actual;
        std::optional<lct::Verified> verified;
        try {
            verified = lct::verify(c.token, keys, c.revocation, c.now, lct::Presented{c.presented, c.url_form});
        } catch (const TrustError& e) { actual = e.code(); }
        expect_outcome(c.name, c.intended, actual);
        auto* item = w.obj();
        w.put(item, "name", w.str(c.name));
        w.put(item, "token", w.str(c.token));
        w.put(item, "now", w.num(c.now));
        w.put(item, "presented", w.str(lct::profile_name(c.presented)));
        w.put(item, "url_form", w.boolean(c.url_form));
        auto* rev = w.obj();
        w.put(rev, "revoked_jti", string_array(w, c.revocation.revoked_jti));
        if (c.revocation.cap_issued_before) w.put(rev, "cap_issued_before", w.num(*c.revocation.cap_issued_before));
        w.put(rev, "frozen_project_refs", string_array(w, c.revocation.frozen_project_refs));
        w.put(item, "revocation", rev);
        if (actual) {
            w.put(item, "error", w.str(code_name(*actual)));
        } else {
            auto* expect = w.obj();
            w.put(expect, "profile", w.str(lct::profile_name(verified->profile)));
            w.put(expect, "issued_profile", w.str(lct::profile_name(verified->issued_profile)));
            w.put(expect, "kid", w.str(verified->kid));
            w.put(expect, "iat", w.num(verified->iat));
            w.put(expect, "exp", w.num(verified->exp));
            w.put(expect, "caveats", w.str(verified->caveats_json));
            w.put(item, "expect", expect);
        }
        w.push(list, item);
    }
    w.put(root, "cases", list);
    w.save(root, dir + "/lct.json");
}

} // namespace synergos::trust::test
