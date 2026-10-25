// 信頼の土台の単体テスト + golden vector 照合。
// `synergos_trust_test --write-vectors` で spec/vectors を作り直す (seed 固定なので差分は実装の変化だけ)。
#include "vectors.h"
#include "synergos/trust/envelope.h"
#include "synergos/trust/error.h"
#include "synergos/trust/jcs.h"
#include "synergos/trust/lct.h"
#include <cstdio>
#include <stdexcept>
#include <string>
#include <string_view>

using namespace synergos::trust;
namespace {
void require(bool value, const char* what) { if (!value) throw std::runtime_error(what); }
template<class F> void rejects(Code code, F action, const char* what) {
    try { action(); } catch (const TrustError& e) { if (e.code() == code) return; throw std::runtime_error(what); }
    throw std::runtime_error(what);
}
Seed seed_of(std::uint8_t b) { Seed s; s.fill(b); return s; }

void base64url_cases() {
    const std::string_view hello = "hello";
    const auto encoded = base64url_encode(reinterpret_cast<const std::uint8_t*>(hello.data()), hello.size());
    require(encoded == "aGVsbG8", "base64url encode");
    const auto decoded = base64url_decode(encoded);
    require(std::string(decoded.begin(), decoded.end()) == hello, "base64url round trip");
    rejects(Code::encoding_invalid, [] { base64url_decode("aGVsbG8="); }, "padding accepted");
    rejects(Code::encoding_invalid, [] { base64url_decode("aGVsbG9"); }, "non-canonical trailing bits accepted");
    rejects(Code::encoding_invalid, [] { base64url_decode("aGV+bG8"); }, "standard alphabet accepted");
}

void jcs_limits() {
    const std::string limit = std::string(128, '[') + std::string(128, ']');
    require(jcs::canonicalize(limit) == limit, "128 levels rejected");
    std::string deep(129, '[');
    deep += std::string(129, ']');
    rejects(Code::jcs_too_deep, [&] { jcs::canonicalize(deep); }, "129 levels accepted");
    std::string big = "\"" + std::string(jcs::max_bytes, 'a') + "\"";
    rejects(Code::jcs_too_large, [&] { jcs::canonicalize(big); }, "over 1 MiB accepted");
    rejects(Code::jcs_invalid_json, [] { jcs::canonicalize("\"\xff\""); }, "invalid UTF-8 accepted");
}

void envelope_signer_gate() {
    const SigningKey key(seed_of(0x41));
    rejects(Code::envelope_bad_signer, [&] { sign(Ctx::membership, 1, "{}", Signer::node, key); }, "node signed membership");
    rejects(Code::envelope_bad_signer, [&] { sign(Ctx::revocation, 1, "{}", Signer::custody, key); }, "custody signed revocation");
    const auto sig = sign(Ctx::gossip, 1, R"({"b":1,"a":2})", Signer::node, key);
    require(verify(Ctx::gossip, 1, R"({"a":2, "b":1})", Signer::node, key.public_key(), sig) == R"({"a":2,"b":1})",
            "verify returns canonical payload");
    auto bad = sig;
    bad[63] ^= 1;
    rejects(Code::envelope_bad_signature, [&] { verify(Ctx::gossip, 1, R"({"a":2,"b":1})", Signer::node, key.public_key(), bad); },
            "flipped signature accepted");
}

void lct_api_gates() {
    const SigningKey issuer(seed_of(0x51));
    const std::string delegation = R"({"exp":1800000200,"iss":"custody:hq","jti":"d-1","max_bytes_per_hour":100,)"
                                   R"("ops":["get","put"],"orgs":["o"],"sub":"curare:d1"})";
    rejects(Code::lct_profile_forbidden,
            [&] { lct::mint("k", issuer, lct::Profile::membership, 1800000000, delegation, seed_of(0x52)); },
            "Synergos minted a membership LCT");
    rejects(Code::lct_bad_caveat,
            [&] { lct::mint("k", issuer, lct::Profile::custody_delegation, 1800000000, R"({"ops":["get"]})", seed_of(0x52)); },
            "mint without exp accepted");
    rejects(Code::lct_profile_rule,
            [&] { lct::mint("k", issuer, lct::Profile::custody_delegation, 1800000000, R"({"exp":1800000200,"ops":["get"]})", seed_of(0x52)); },
            "delegation without iss/sub/orgs accepted");
    const auto minted = lct::mint("k", issuer, lct::Profile::custody_delegation, 1800000000, delegation, seed_of(0x52)).token;
    rejects(Code::lct_widened, [&] { lct::attenuate(minted, R"({"ops":["delete"]})", seed_of(0x53)); }, "attenuate widened ops");
    rejects(Code::lct_widened, [&] { lct::attenuate(minted, R"({"exp":1800000300})", seed_of(0x53)); }, "attenuate extended exp");
    rejects(Code::lct_widened, [&] { lct::attenuate(minted, R"({"sub":"curare:d10"})", seed_of(0x53)); }, "sub prefix without / accepted");
    const auto narrowed = lct::attenuate(
        minted, R"({"exp":1800000100,"jti":"c-1","object_keys":["k1"],"ops":["get"],"org":"o","project_ref":"p","session_id":"s","sub":"curare:d1/u"})",
        seed_of(0x53)).token;
    const auto sealed = lct::seal(narrowed);
    rejects(Code::lct_malformed, [&] { lct::attenuate(sealed, R"({"ops":["get"]})", seed_of(0x54)); }, "sealed token attenuated");
    rejects(Code::lct_malformed, [&] { lct::seal(sealed); }, "sealed twice");
    const std::map<std::string, lct::IssuerKey> issuers{{"k", {issuer.public_key(), lct::Profile::custody_delegation}}};
    const auto v = lct::verify(sealed, issuers, {}, 1800000010, {lct::Profile::custody_capability});
    require(v.exp == 1800000100 && v.profile == lct::Profile::custody_capability && v.issued_profile == lct::Profile::custody_delegation,
            "capability attenuated from a delegation");
    require(v.caveats_json == R"({"exp":1800000100,"iss":"custody:hq","jti":"c-1","max_bytes_per_hour":100,"object_keys":["k1"],"ops":["get"],)"
                              R"("org":"o","orgs":["o"],"project_ref":"p","session_id":"s","sub":"curare:d1/u"})",
            "attenuated intersection");
    // 鎖の長さの上限: max_blocks を超える attenuate は作らない
    auto chain = minted;
    for (std::size_t i = 1; i < lct::max_blocks; ++i) chain = lct::attenuate(chain, "{}", seed_of(static_cast<std::uint8_t>(0x60 + i))).token;
    rejects(Code::decode_too_large, [&] { lct::attenuate(chain, "{}", seed_of(0x7f)); }, "chain longer than max_blocks attenuated");
}
} // namespace

int main(int argc, char** argv) {
    try {
        if (argc == 3 && std::string_view(argv[1]) == "--write-vectors") {
            synergos::trust::test::write_vectors(argv[2]);
            std::puts("vectors written");
            return 0;
        }
        if (argc != 2) throw std::invalid_argument("usage: synergos_trust_test [--write-vectors] <vectors_dir>");
        base64url_cases();
        jcs_limits();
        envelope_signer_gate();
        lct_api_gates();
        synergos::trust::test::check_vectors(argv[1]);
    } catch (const std::exception& e) {
        std::fprintf(stderr, "FAIL: %s\n", e.what());
        return 1;
    }
    return 0;
}
