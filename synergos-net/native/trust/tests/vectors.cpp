// golden vector の照合。ファイルの入力だけから実装で結果を求め、期待値と比べる (生成コードは使わない)。
#include "vectors.h"
#include "synergos/trust/envelope.h"
#include "synergos/trust/error.h"
#include "synergos/trust/jcs.h"
#include "synergos/trust/lct.h"
#include "synergos/trust/receipt.h"
#include <yyjson.h>
#include <functional>
#include <memory>
#include <optional>
#include <stdexcept>

namespace synergos::trust::test {
namespace {

using Doc = std::unique_ptr<yyjson_doc, decltype(&yyjson_doc_free)>;

Doc load(const std::string& path) {
    yyjson_read_err err{};
    Doc doc(yyjson_read_file(path.c_str(), 0, nullptr, &err), yyjson_doc_free);
    if (!doc) throw std::runtime_error("cannot read " + path + ": " + (err.msg ? err.msg : "?"));
    return doc;
}

std::string text(yyjson_val* obj, const char* key) {
    auto* v = yyjson_obj_get(obj, key);
    if (!yyjson_is_str(v)) throw std::runtime_error(std::string("missing string ") + key);
    return {yyjson_get_str(v), yyjson_get_len(v)};
}

std::int64_t integer(yyjson_val* v) {
    return yyjson_is_uint(v) ? static_cast<std::int64_t>(yyjson_get_uint(v)) : yyjson_get_sint(v);
}

void each(yyjson_val* arr, const std::function<void(yyjson_val*)>& fn) {
    auto it = yyjson_arr_iter_with(arr);
    while (auto* item = yyjson_arr_iter_next(&it)) fn(item);
}

/// 期待: "error" があればそのコードで失敗、無ければ成功。
void compare(const std::string& file, yyjson_val* item, const std::function<void()>& run) {
    const auto name = text(item, "name");
    auto* error = yyjson_obj_get(item, "error");
    std::string got = "ok";
    try { run(); } catch (const TrustError& e) { got = std::string(code_name(e.code())); }
    const std::string want = error ? text(item, "error") : "ok";
    if (got != want) throw std::runtime_error(file + " " + name + ": expected " + want + " but got " + got);
}

void require(bool ok, const std::string& what) { if (!ok) throw std::runtime_error(what); }

void check_registry(const std::string& dir) {
    const auto doc = load(dir + "/ctx-registry.json");
    auto* rows = yyjson_obj_get(yyjson_doc_get_root(doc.get()), "rows");
    std::size_t count = 0;
    each(rows, [&](yyjson_val* row) {
        const auto name = text(row, "ctx");
        const auto ctx = parse_ctx(name);
        require(ctx.has_value(), "ctx-registry: unknown ctx " + name);
        std::set<std::string> listed;
        each(yyjson_obj_get(row, "signers"), [&](yyjson_val* s) { listed.emplace(yyjson_get_str(s)); });
        for (const auto s : {Signer::cernere, Signer::node, Signer::control_deny, Signer::custody, Signer::curare, Signer::holder})
            require(signer_allowed(*ctx, s) == (listed.count(std::string(signer_name(s))) == 1),
                    "ctx-registry: signers differ for " + name);
        ++count;
    });
    require(count == static_cast<std::size_t>(Ctx::journal_checkpoint) + 1, "ctx-registry: row count differs from the implementation");
}

void check_jcs(const std::string& dir) {
    const auto doc = load(dir + "/jcs.json");
    each(yyjson_obj_get(yyjson_doc_get_root(doc.get()), "cases"), [&](yyjson_val* item) {
        compare("jcs.json", item, [&] {
            const auto canonical = jcs::canonicalize(text(item, "input"));
            require(canonical == text(item, "canonical"), "jcs.json " + text(item, "name") + ": canonical form differs");
        });
    });
}

void check_envelope(const std::string& dir) {
    const auto doc = load(dir + "/envelope.json");
    each(yyjson_obj_get(yyjson_doc_get_root(doc.get()), "cases"), [&](yyjson_val* item) {
        const auto name = text(item, "name");
        const auto ctx = parse_ctx(text(item, "ctx"));
        const auto signer = parse_signer(text(item, "signer"));
        require(ctx && signer, "envelope.json " + name + ": unknown ctx or signer");
        const auto ver = static_cast<std::uint8_t>(integer(yyjson_obj_get(item, "ver")));
        const auto payload = text(item, "payload");
        const auto msg = message(*ctx, ver, payload);
        require(base64url_encode(reinterpret_cast<const std::uint8_t*>(msg.data()), msg.size()) == text(item, "message"),
                "envelope.json " + name + ": message bytes differ");
        compare("envelope.json", item, [&] {
            verify(*ctx, ver, payload, *signer, base64url_decode_fixed<32>(text(item, "public_key")),
                   base64url_decode_fixed<64>(text(item, "signature")));
        });
    });
}

std::optional<lct::Profile> profile_of(const std::string& name) {
    for (const auto p : {lct::Profile::membership, lct::Profile::custody_delegation, lct::Profile::custody_capability})
        if (lct::profile_name(p) == name) return p;
    return std::nullopt;
}

lct::RevocationView revocation_of(yyjson_val* rev) {
    lct::RevocationView out;
    each(yyjson_obj_get(rev, "revoked_jti"), [&](yyjson_val* s) { out.revoked_jti.emplace(yyjson_get_str(s)); });
    each(yyjson_obj_get(rev, "frozen_project_refs"), [&](yyjson_val* s) { out.frozen_project_refs.emplace(yyjson_get_str(s)); });
    if (auto* cut = yyjson_obj_get(rev, "cap_issued_before")) out.cap_issued_before = integer(cut);
    return out;
}

void check_lct(const std::string& dir) {
    const auto doc = load(dir + "/lct.json");
    auto* root = yyjson_doc_get_root(doc.get());
    std::map<std::string, lct::IssuerKey> issuers;
    each(yyjson_obj_get(root, "issuers"), [&](yyjson_val* i) {
        const auto profile = profile_of(text(i, "profile"));
        require(profile.has_value(), "lct.json: unknown issuer profile");
        issuers.emplace(text(i, "kid"), lct::IssuerKey{base64url_decode_fixed<32>(text(i, "public_key")), *profile});
    });
    each(yyjson_obj_get(root, "cases"), [&](yyjson_val* item) {
        const auto name = text(item, "name");
        compare("lct.json", item, [&] {
            const auto presented = profile_of(text(item, "presented"));
            require(presented.has_value(), "lct.json " + name + ": unknown presented profile");
            const auto v = lct::verify(text(item, "token"), issuers, revocation_of(yyjson_obj_get(item, "revocation")),
                                       integer(yyjson_obj_get(item, "now")),
                                       lct::Presented{*presented, yyjson_get_bool(yyjson_obj_get(item, "url_form"))});
            auto* expect = yyjson_obj_get(item, "expect");
            require(lct::profile_name(v.profile) == text(expect, "profile")
                        && lct::profile_name(v.issued_profile) == text(expect, "issued_profile") && v.kid == text(expect, "kid")
                        && v.iat == integer(yyjson_obj_get(expect, "iat")) && v.exp == integer(yyjson_obj_get(expect, "exp"))
                        && v.caveats_json == text(expect, "caveats"),
                    "lct.json " + name + ": verified result differs");
        });
    });
}

void check_receipt(const std::string& dir) {
    const auto doc = load(dir + "/receipt.json");
    auto* root = yyjson_doc_get_root(doc.get());
    std::map<std::string, PublicKey> pinned;
    each(yyjson_obj_get(root, "pinned"), [&](yyjson_val* p) {
        pinned.emplace(text(p, "kid"), base64url_decode_fixed<32>(text(p, "public_key")));
    });
    each(yyjson_obj_get(root, "cases"), [&](yyjson_val* item) {
        const auto name = text(item, "name");
        compare("receipt.json", item, [&] {
            const auto r = receipt::verify(text(item, "payload"), base64url_decode_fixed<64>(text(item, "signature")), pinned,
                                           integer(yyjson_obj_get(item, "now")));
            auto* expect = yyjson_obj_get(item, "expect");
            require(r.kid == text(expect, "kid") && r.opaque_object_id == text(expect, "opaqueObjectId")
                        && r.byte_length == integer(yyjson_obj_get(expect, "byteLength"))
                        && static_cast<std::int64_t>(r.digests.size()) == integer(yyjson_obj_get(expect, "digests")),
                    "receipt.json " + name + ": verified result differs");
        });
    });
}

} // namespace

void check_vectors(const std::string& dir) {
    check_registry(dir);
    check_jcs(dir);
    check_envelope(dir);
    check_lct(dir);
    check_receipt(dir);
}

} // namespace synergos::trust::test
