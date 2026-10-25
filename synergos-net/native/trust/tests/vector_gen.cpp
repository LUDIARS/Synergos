// golden vector の生成。seed を固定するので Ed25519 (決定的署名) の出力は毎回同じになる。
// 各ケースは意図した結果を持ち、実装の結果と食い違えば生成を止める。
#include "json_writer.h"
#include "vector_gen.h"
#include "vectors.h"
#include "synergos/trust/envelope.h"
#include "synergos/trust/error.h"
#include "synergos/trust/jcs.h"
#include <functional>
#include <optional>
#include <stdexcept>

namespace synergos::trust::test {
namespace {


// --- ctx-registry.json -------------------------------------------------------

void write_registry(const std::string& dir) {
    JsonWriter w;
    auto* root = w.obj();
    w.put(root, "version", w.num(1));
    auto* rows = w.arr();
    for (int i = 0; i <= static_cast<int>(Ctx::journal_checkpoint); ++i) {
        const auto ctx = static_cast<Ctx>(i);
        auto* row = w.obj();
        w.put(row, "ctx", w.str(ctx_name(ctx)));
        auto* signers = w.arr();
        for (const auto s : {Signer::cernere, Signer::node, Signer::control_deny, Signer::custody, Signer::curare, Signer::holder})
            if (signer_allowed(ctx, s)) w.push(signers, w.str(signer_name(s)));
        w.put(row, "signers", signers);
        w.push(rows, row);
    }
    w.put(root, "rows", rows);
    w.save(root, dir + "/ctx-registry.json");
}

// --- jcs.json ----------------------------------------------------------------

void write_jcs(const std::string& dir) {
    struct Case { const char* name; std::string input; std::optional<Code> intended; };
    const std::vector<Case> cases{
        {"utf16-key-order", R"({"\ufb01":1,"\ud83d\ude00":2,"a":3})", std::nullopt},
        {"nested-sort", R"({ "b": {"d":1, "c":2}, "a": [ {"z":1,"y":2} ] })", std::nullopt},
        {"escapes", R"(["\u000f","\u2028","\/","\"","\\","\n","\u00e9"])", std::nullopt},
        {"literals", "[true, false, null]", std::nullopt},
        {"safe-integers", "[0, 9007199254740991, -9007199254740991]", std::nullopt},
        {"unsafe-integer", "[9007199254740992]", Code::jcs_unsafe_integer},
        {"float", "[1.5]", Code::jcs_float},
        {"exponent", "[1e2]", Code::jcs_float},
        {"duplicate-key", R"({"a":1,"a":2})", Code::jcs_duplicate_key},
        {"invalid-json", "{", Code::jcs_invalid_json},
    };
    JsonWriter w;
    auto* root = w.obj();
    auto* list = w.arr();
    for (const auto& c : cases) {
        auto* item = w.obj();
        w.put(item, "name", w.str(c.name));
        w.put(item, "input", w.str(c.input));
        std::optional<Code> actual;
        std::string canonical;
        try { canonical = jcs::canonicalize(c.input); } catch (const TrustError& e) { actual = e.code(); }
        expect_outcome(c.name, c.intended, actual);
        if (actual) w.put(item, "error", w.str(code_name(*actual)));
        else w.put(item, "canonical", w.str(canonical));
        w.push(list, item);
    }
    w.put(root, "cases", list);
    w.save(root, dir + "/jcs.json");
}

// --- envelope.json -----------------------------------------------------------

void write_envelope(const std::string& dir) {
    struct Case {
        const char* name;
        Ctx sign_ctx; Signer sign_as; std::uint8_t seed; std::string payload; // 署名した側
        Ctx check_ctx; Signer check_as;                                       // 検証で主張する側
        std::optional<Code> intended;
    };
    const std::vector<Case> cases{
        {"membership-by-cernere", Ctx::membership, Signer::cernere, 0x31, R"({"sub":"user-1", "org":"ludiars", "exp":1800003600})",
         Ctx::membership, Signer::cernere, std::nullopt},
        {"peer-session-non-bmp", Ctx::peer_session, Signer::node, 0x32, R"({"peer":"node-\ud83d\ude00","nonce":7})",
         Ctx::peer_session, Signer::node, std::nullopt},
        {"checkpoint-by-curare", Ctx::journal_checkpoint, Signer::curare, 0x33, R"({"seq":42,"head":"sha256:00"})",
         Ctx::journal_checkpoint, Signer::curare, std::nullopt},
        {"membership-claimed-by-node", Ctx::membership, Signer::cernere, 0x31, R"({"sub":"user-1","exp":1800003600})",
         Ctx::membership, Signer::node, Code::envelope_bad_signer},
        {"ctx-swapped", Ctx::membership, Signer::cernere, 0x31, R"({"sub":"user-1","exp":1800003600})",
         Ctx::node_card, Signer::cernere, Code::envelope_bad_signature},
    };
    JsonWriter w;
    auto* root = w.obj();
    auto* list = w.arr();
    for (const auto& c : cases) {
        const SigningKey key(seed_of(c.seed));
        const auto sig = sign(c.sign_ctx, 1, c.payload, c.sign_as, key);
        const auto msg = message(c.check_ctx, 1, c.payload);
        std::optional<Code> actual;
        try { verify(c.check_ctx, 1, c.payload, c.check_as, key.public_key(), sig); } catch (const TrustError& e) { actual = e.code(); }
        expect_outcome(c.name, c.intended, actual);
        auto* item = w.obj();
        w.put(item, "name", w.str(c.name));
        w.put(item, "ctx", w.str(ctx_name(c.check_ctx)));
        w.put(item, "ver", w.num(1));
        w.put(item, "signer", w.str(signer_name(c.check_as)));
        w.put(item, "public_key", w.str(base64url_encode(key.public_key())));
        w.put(item, "payload", w.str(c.payload));
        w.put(item, "message", w.str(base64url_encode(reinterpret_cast<const std::uint8_t*>(msg.data()), msg.size())));
        w.put(item, "signature", w.str(base64url_encode(sig)));
        if (actual) w.put(item, "error", w.str(code_name(*actual)));
        w.push(list, item);
    }
    w.put(root, "cases", list);
    w.save(root, dir + "/envelope.json");
}

} // namespace

void write_vectors(const std::string& dir) {
    write_registry(dir);
    write_jcs(dir);
    write_envelope(dir);
    write_lct(dir);
    write_receipt(dir);
}

} // namespace synergos::trust::test
