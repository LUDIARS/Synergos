#include "token_surgery.h"
#include "synergos/trust/jcs.h"
#include <yyjson.h>
#include <cstdlib>
#include <memory>
#include <stdexcept>

namespace synergos::trust::test {
namespace {

std::string write(yyjson_val* v) {
    std::size_t len{};
    std::unique_ptr<char, decltype(&std::free)> text(yyjson_val_write(v, 0, &len), std::free);
    if (!text) throw std::runtime_error("cannot write token part");
    return jcs::canonicalize({text.get(), len});
}

} // namespace

RawToken open_token(std::string_view token) {
    const auto raw = base64url_decode(token);
    const std::string json(raw.begin(), raw.end());
    std::unique_ptr<yyjson_doc, decltype(&yyjson_doc_free)> doc(yyjson_read(json.data(), json.size(), 0), yyjson_doc_free);
    auto* root = yyjson_doc_get_root(doc.get());
    if (!yyjson_is_obj(root)) throw std::runtime_error("token is not an object");
    RawToken out;
    auto it = yyjson_arr_iter_with(yyjson_obj_get(root, "blocks"));
    while (auto* block = yyjson_arr_iter_next(&it))
        out.blocks.emplace_back(write(yyjson_obj_get(block, "body")), yyjson_get_str(yyjson_obj_get(block, "sig")));
    out.proof = write(yyjson_obj_get(root, "proof"));
    return out;
}

std::string close_token(const RawToken& token) {
    std::string json = "{\"blocks\":[";
    for (std::size_t i = 0; i < token.blocks.size(); ++i) {
        if (i) json += ',';
        json += "{\"body\":" + token.blocks[i].first + ",\"sig\":" + jcs::quote(token.blocks[i].second) + "}";
    }
    json += "],\"proof\":" + token.proof + ",\"v\":1}";
    return close_text(jcs::canonicalize(json));
}

std::string close_text(std::string_view json) {
    return base64url_encode(reinterpret_cast<const std::uint8_t*>(json.data()), json.size());
}

std::string block_body(std::string_view caveats_json, const PublicKey& next_pub, const char* kid, const char* profile,
                       std::int64_t iat) {
    std::string json = "{\"caveats\":" + std::string(caveats_json);
    if (kid) json += ",\"iat\":" + std::to_string(iat) + ",\"kid\":" + jcs::quote(kid) + ",\"profile\":" + jcs::quote(profile);
    json += ",\"next_pub\":" + jcs::quote(base64url_encode(next_pub)) + ",\"v\":1}";
    return jcs::canonicalize(json);
}

std::string unsealed_proof(const Seed& next_seed) {
    return "{\"next_secret\":" + jcs::quote(base64url_encode(next_seed)) + "}";
}

} // namespace synergos::trust::test
