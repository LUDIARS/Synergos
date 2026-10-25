// receipt.json の生成。正しい受領証は公開 API の sign で作り、規則に合わない本体は envelope の sign で直接署名する。
#include "json_writer.h"
#include "vector_gen.h"
#include "synergos/trust/envelope.h"
#include "synergos/trust/jcs.h"
#include "synergos/trust/receipt.h"
#include <vector>

namespace synergos::trust::test {
namespace {

constexpr std::uint8_t custody_seed = 0x61;
const char* const pinned_kid = "receipt-1";
const std::string hash_a(64, 'a');
const std::string hash_b(64, 'b');

struct ReceiptCase {
    std::string name;
    std::string payload;
    Signature signature;
    std::int64_t now;
    std::optional<Code> intended;
};

receipt::Receipt base(const char* kid = pinned_kid) {
    receipt::Receipt r;
    r.kid = kid;
    r.iss = "custody-hq";
    r.iat = iat;
    r.exp = iat + 86400;
    r.nonce = "n-1";
    r.operation_id = "op-1";
    r.opaque_object_id = "obj-1";
    r.org = "org-a";
    r.project_ref = "proj-a";
    r.digests = {{"sha256", hash_a}};
    r.byte_length = 1024;
    r.locations = {"hq/vol1"};
    return r;
}

ReceiptCase signed_case(std::string name, const receipt::Receipt& r, std::int64_t now = iat, std::optional<Code> intended = std::nullopt) {
    const auto s = receipt::sign(r, SigningKey(seed_of(custody_seed)));
    return {std::move(name), s.payload, s.signature, now, intended};
}

/// sign が拒否する本体を custody の鍵で直接署名する (署名は正しく、形だけが規則違反)。
ReceiptCase forged(std::string name, const std::string& payload, Code intended) {
    Signature sig{};
    try { sig = sign(Ctx::receipt, 1, payload, Signer::custody, SigningKey(seed_of(custody_seed))); } catch (const TrustError&) {}
    return {std::move(name), payload, sig, iat, intended};
}

std::string replace(std::string text, const std::string& from, const std::string& to) {
    text.replace(text.find(from), from.size(), to);
    return text;
}

std::vector<ReceiptCase> receipt_cases() {
    const auto ok = receipt::sign(base(), SigningKey(seed_of(custody_seed)));
    auto replicated = base();
    replicated.durability = receipt::Durability::durable_replicated;
    replicated.digests = {{"blake3", hash_b}, {"sha256", hash_a}};
    replicated.locations = {"hq/vol1", "mac/vol1"};
    auto future = base();
    future.iat = iat + receipt::max_future_skew + 1;
    future.exp = future.iat + 60;
    auto bad_sig = ok.signature;
    bad_sig[0] ^= 1;
    std::string many_locations;
    for (std::size_t i = 0; i <= receipt::max_locations; ++i) many_locations += (i ? ",\"l" : "\"l") + std::to_string(i) + "\"";

    std::vector<ReceiptCase> out;
    out.push_back(signed_case("receipt-single", base()));
    out.push_back(signed_case("receipt-replicated", replicated));
    out.push_back(signed_case("unknown-kid", base("receipt-9"), iat, Code::receipt_unknown_kid));
    out.push_back({"bad-signature", ok.payload, bad_sig, iat, Code::envelope_bad_signature});
    out.push_back(signed_case("from-future", future, iat, Code::receipt_from_future));
    out.push_back({"non-jcs", replace(ok.payload, "{\"alg\"", "{ \"alg\""), ok.signature, iat, Code::receipt_malformed});
    out.push_back(forged("extra-field", jcs::canonicalize(replace(ok.payload, "\"v\":2", "\"v\":2,\"zzz\":1")), Code::receipt_malformed));
    out.push_back(forged("wrong-version", replace(ok.payload, "\"v\":2", "\"v\":1"), Code::receipt_malformed));
    out.push_back(forged("uppercase-digest", replace(ok.payload, hash_a, std::string(64, 'A')), Code::receipt_malformed));
    out.push_back(forged("too-many-locations", replace(ok.payload, "[\"hq/vol1\"]", "[" + many_locations + "]"), Code::decode_too_large));
    out.push_back(forged("too-large", std::string(receipt::max_receipt_bytes + 1, ' '), Code::decode_too_large));
    out.push_back(forged("float-length", replace(ok.payload, "\"byteLength\":1024", "\"byteLength\":1024.5"), Code::receipt_malformed));
    return out;
}

} // namespace

void write_receipt(const std::string& dir) {
    JsonWriter w;
    auto* root = w.obj();
    const auto pub = SigningKey(seed_of(custody_seed)).public_key();
    auto* pinned = w.arr();
    auto* entry = w.obj();
    w.put(entry, "kid", w.str(pinned_kid));
    w.put(entry, "public_key", w.str(base64url_encode(pub)));
    w.push(pinned, entry);
    w.put(root, "pinned", pinned);
    const std::map<std::string, PublicKey> keys{{pinned_kid, pub}};
    auto* list = w.arr();
    for (const auto& c : receipt_cases()) {
        std::optional<Code> actual;
        std::optional<receipt::Receipt> verified;
        try { verified = receipt::verify(c.payload, c.signature, keys, c.now); } catch (const TrustError& e) { actual = e.code(); }
        expect_outcome(c.name, c.intended, actual);
        auto* item = w.obj();
        w.put(item, "name", w.str(c.name));
        w.put(item, "payload", w.str(c.payload));
        w.put(item, "signature", w.str(base64url_encode(c.signature)));
        w.put(item, "now", w.num(c.now));
        if (actual) {
            w.put(item, "error", w.str(code_name(*actual)));
        } else {
            auto* expect = w.obj();
            w.put(expect, "kid", w.str(verified->kid));
            w.put(expect, "opaqueObjectId", w.str(verified->opaque_object_id));
            w.put(expect, "byteLength", w.num(verified->byte_length));
            w.put(expect, "digests", w.num(static_cast<std::int64_t>(verified->digests.size())));
            w.put(item, "expect", expect);
        }
        w.push(list, item);
    }
    w.put(root, "cases", list);
    w.save(root, dir + "/receipt.json");
}

} // namespace synergos::trust::test
