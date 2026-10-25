// @implements SPEC-SY-custody-receipt-v2
// custody の受領証 v2: JCS の本体バイトと、ctx receipt の分離署名。
// Curare は本体・署名・kid をそのまま保存し、後から何度でも検証し直せる (exp は保存の都合で現在時刻と比べない)。
#pragma once
#include "synergos/trust/crypto.h"
#include <cstddef>
#include <cstdint>
#include <map>
#include <string>
#include <string_view>
#include <vector>

namespace synergos::trust::receipt {

/// SPEC-SY-decode-bounds (decoder_id trust.receipt)。
inline constexpr std::size_t max_receipt_bytes = 64 * 1024;
inline constexpr std::size_t max_digests = 8;
inline constexpr std::size_t max_locations = 32;
/// 受け入れる発行時刻の未来方向のずれ (秒)。
inline constexpr std::int64_t max_future_skew = 120;

enum class Durability { durable_single, durable_replicated };

struct Digest {
    std::string alg;   // "sha256" | "blake3"
    std::string value; // 小文字 16 進 64 文字
};

struct Receipt {
    std::string kid;
    std::string iss; // custody_instance_id
    std::int64_t iat{};
    std::int64_t exp{};
    std::string nonce;
    std::string operation_id;
    std::string opaque_object_id;
    std::string org;
    std::string project_ref;
    std::vector<Digest> digests;
    std::int64_t byte_length{};
    std::vector<std::string> locations;
    Durability durability{Durability::durable_single};
};

struct Signed {
    std::string payload; // JCS 正規形の本体 (保存はこのバイト列のまま)
    Signature signature{};
};

/// 本体を JCS にして custody の鍵で署名する。形が規則に合わなければ receipt.malformed。
Signed sign(const Receipt& receipt, const SigningKey& custody_key);

/// pinned は SYNERGOS_RECEIPT_PUBKEYS ({kid: pub})。失敗は型付きの TrustError
/// (decode.too_large / receipt.malformed / receipt.unknown_kid / envelope.bad_signature / receipt.from_future)。
Receipt verify(std::string_view payload, const Signature& signature, const std::map<std::string, PublicKey>& pinned,
               std::int64_t now);

} // namespace synergos::trust::receipt
