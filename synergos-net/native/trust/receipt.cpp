// @implements SPEC-SY-custody-receipt-v2 SPEC-SY-decode-bounds
// @decoder trust.receipt
#include "synergos/trust/receipt.h"
#include "synergos/trust/envelope.h"
#include "synergos/trust/error.h"
#include "synergos/trust/jcs.h"
#include <yyjson.h>
#include <algorithm>
#include <array>
#include <memory>

namespace synergos::trust::receipt {
namespace {

using Doc = std::unique_ptr<yyjson_doc, decltype(&yyjson_doc_free)>;

constexpr std::array<std::string_view, 15> keys{
    "alg", "byteLength", "digests", "durability", "exp", "iat", "iss", "kid", "locations", "nonce",
    "opaqueObjectId", "operationId", "org", "project_ref", "v"};

[[noreturn]] void malformed(const std::string& detail) { throw TrustError(Code::receipt_malformed, detail); }

void bounded(std::size_t declared, std::size_t max, const char* what) {
    if (declared > max)
        throw TrustError(Code::decode_too_large, std::string("trust.receipt ") + what + ": " + std::to_string(declared) + " > " + std::to_string(max));
}

std::string_view durability_name(Durability d) {
    return d == Durability::durable_replicated ? "durable_replicated" : "durable_single";
}

bool lower_hex64(const std::string& value) {
    return value.size() == 64 && std::all_of(value.begin(), value.end(), [](char c) { return (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f'); });
}

/// 署名する側と検証する側で同じ規則を当てる。
void check(const Receipt& r) {
    for (const auto* field : {&r.kid, &r.iss, &r.nonce, &r.operation_id, &r.opaque_object_id, &r.org, &r.project_ref})
        if (field->empty()) malformed("text fields must be non-empty");
    if (r.iat < 0 || r.exp < r.iat) malformed("exp must not precede iat");
    if (r.byte_length < 0) malformed("byteLength must be non-negative");
    if (r.digests.empty()) malformed("digests must not be empty");
    bounded(r.digests.size(), max_digests, "digests");
    for (const auto& d : r.digests) {
        if (d.alg != "sha256" && d.alg != "blake3") malformed("digest alg must be sha256 or blake3");
        if (!lower_hex64(d.value)) malformed("digest value must be 64 lowercase hex digits");
    }
    if (r.locations.empty()) malformed("locations must not be empty");
    bounded(r.locations.size(), max_locations, "locations");
    for (const auto& l : r.locations) if (l.empty()) malformed("locations entries must be non-empty");
}

std::string to_json(const Receipt& r) {
    std::string json = "{\"alg\":\"Ed25519\",\"byteLength\":" + std::to_string(r.byte_length) + ",\"digests\":[";
    for (std::size_t i = 0; i < r.digests.size(); ++i) {
        if (i) json += ',';
        json += "{\"alg\":" + jcs::quote(r.digests[i].alg) + ",\"value\":" + jcs::quote(r.digests[i].value) + "}";
    }
    json += "],\"durability\":" + jcs::quote(durability_name(r.durability)) + ",\"exp\":" + std::to_string(r.exp)
        + ",\"iat\":" + std::to_string(r.iat) + ",\"iss\":" + jcs::quote(r.iss) + ",\"kid\":" + jcs::quote(r.kid) + ",\"locations\":[";
    for (std::size_t i = 0; i < r.locations.size(); ++i) {
        if (i) json += ',';
        json += jcs::quote(r.locations[i]);
    }
    json += "],\"nonce\":" + jcs::quote(r.nonce) + ",\"opaqueObjectId\":" + jcs::quote(r.opaque_object_id)
        + ",\"operationId\":" + jcs::quote(r.operation_id) + ",\"org\":" + jcs::quote(r.org)
        + ",\"project_ref\":" + jcs::quote(r.project_ref) + ",\"v\":2}";
    return jcs::canonicalize(json);
}

std::string text(yyjson_val* obj, const char* key) {
    auto* v = yyjson_obj_get(obj, key);
    if (!yyjson_is_str(v)) malformed(std::string(key) + " must be a string");
    return {yyjson_get_str(v), yyjson_get_len(v)};
}

std::int64_t integer(yyjson_val* obj, const char* key) {
    auto* v = yyjson_obj_get(obj, key);
    if (yyjson_is_uint(v)) return static_cast<std::int64_t>(yyjson_get_uint(v));
    if (yyjson_is_sint(v)) return yyjson_get_sint(v);
    malformed(std::string(key) + " must be an integer");
}

yyjson_val* array(yyjson_val* obj, const char* key, std::size_t max) {
    auto* v = yyjson_obj_get(obj, key);
    if (!yyjson_is_arr(v)) malformed(std::string(key) + " must be an array");
    bounded(yyjson_arr_size(v), max, key); // 要素を読む前に切る
    return v;
}

Receipt parse(const std::string& canonical) {
    Doc doc(yyjson_read(canonical.data(), canonical.size(), 0), yyjson_doc_free);
    auto* root = yyjson_doc_get_root(doc.get());
    if (!yyjson_is_obj(root) || yyjson_obj_size(root) != keys.size()) malformed("receipt must have exactly the v2 fields");
    for (const auto key : keys)
        if (!yyjson_obj_get(root, std::string(key).c_str())) malformed("missing field " + std::string(key));
    if (integer(root, "v") != 2) malformed("v must be 2");
    if (text(root, "alg") != "Ed25519") malformed("alg must be Ed25519");
    Receipt r;
    r.kid = text(root, "kid");
    r.iss = text(root, "iss");
    r.iat = integer(root, "iat");
    r.exp = integer(root, "exp");
    r.nonce = text(root, "nonce");
    r.operation_id = text(root, "operationId");
    r.opaque_object_id = text(root, "opaqueObjectId");
    r.org = text(root, "org");
    r.project_ref = text(root, "project_ref");
    r.byte_length = integer(root, "byteLength");
    const auto durability = text(root, "durability");
    if (durability == "durable_single") r.durability = Durability::durable_single;
    else if (durability == "durable_replicated") r.durability = Durability::durable_replicated;
    else malformed("unknown durability");
    auto it = yyjson_arr_iter_with(array(root, "digests", max_digests));
    while (auto* d = yyjson_arr_iter_next(&it)) {
        if (!yyjson_is_obj(d) || yyjson_obj_size(d) != 2) malformed("digest entries are {alg, value}");
        r.digests.push_back({text(d, "alg"), text(d, "value")});
    }
    auto lit = yyjson_arr_iter_with(array(root, "locations", max_locations));
    while (auto* l = yyjson_arr_iter_next(&lit)) {
        if (!yyjson_is_str(l)) malformed("locations entries must be strings");
        r.locations.emplace_back(yyjson_get_str(l), yyjson_get_len(l));
    }
    check(r);
    return r;
}

} // namespace

Signed sign(const Receipt& receipt, const SigningKey& custody_key) {
    check(receipt);
    Signed out;
    out.payload = to_json(receipt);
    bounded(out.payload.size(), max_receipt_bytes, "bytes");
    out.signature = trust::sign(Ctx::receipt, 1, out.payload, Signer::custody, custody_key);
    return out;
}

Receipt verify(std::string_view payload, const Signature& signature, const std::map<std::string, PublicKey>& pinned,
               std::int64_t now) {
    bounded(payload.size(), max_receipt_bytes, "bytes"); // 確保より前に長さで切る
    std::string canonical;
    try { canonical = jcs::canonicalize(payload); } catch (const TrustError& e) { malformed(e.what()); }
    if (canonical != payload) malformed("receipt bytes are not in JCS form");
    auto r = parse(canonical);
    const auto key = pinned.find(r.kid);
    if (key == pinned.end()) throw TrustError(Code::receipt_unknown_kid, "kid is not pinned");
    trust::verify(Ctx::receipt, 1, canonical, Signer::custody, key->second, signature);
    if (r.iat > now + max_future_skew) throw TrustError(Code::receipt_from_future, "iat is more than 120 s ahead");
    return r;
}

} // namespace synergos::trust::receipt
