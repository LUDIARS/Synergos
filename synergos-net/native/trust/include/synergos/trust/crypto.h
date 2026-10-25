// @implements SPEC-SY-signed-envelope SPEC-SY-lct-v1
// Ed25519 (RFC 8032) と base64url の最小ラッパ。Ed25519 の署名関数を直接呼んでよいのは
// envelope.cpp だけ (lint ゲート: scripts/gate/no-direct-sign.mjs)。
#pragma once
#include <array>
#include <cstdint>
#include <string>
#include <string_view>
#include <vector>

namespace synergos::trust {

using Seed = std::array<std::uint8_t, 32>;
using PublicKey = std::array<std::uint8_t, 32>;
using Signature = std::array<std::uint8_t, 64>;

/// 秘密鍵を保持し、破棄時に消去する。コピー不可。
class SigningKey {
public:
    explicit SigningKey(const Seed& seed);
    ~SigningKey();
    SigningKey(const SigningKey&) = delete;
    SigningKey& operator=(const SigningKey&) = delete;
    SigningKey(SigningKey&&) noexcept;
    SigningKey& operator=(SigningKey&&) noexcept;

    const PublicKey& public_key() const noexcept { return public_; }

private:
    friend class EnvelopeSigner;
    std::array<std::uint8_t, 64> secret_{};
    PublicKey public_{};
};

/// OS の乱数源から 32 byte の seed を作る。
Seed random_seed();

/// 署名の検証 (検証は誰が呼んでもよい)。
bool ed25519_verify(const PublicKey& key, std::string_view message, const Signature& signature);

std::string base64url_encode(const std::uint8_t* data, std::size_t size);
template <std::size_t N>
std::string base64url_encode(const std::array<std::uint8_t, N>& bytes) { return base64url_encode(bytes.data(), N); }
/// パディング無し・正準形だけを受け付ける。不正なら TrustError(encoding_invalid)。
std::vector<std::uint8_t> base64url_decode(std::string_view text);
[[noreturn]] void throw_length_mismatch();
template <std::size_t N>
std::array<std::uint8_t, N> base64url_decode_fixed(std::string_view text) {
    const auto bytes = base64url_decode(text);
    std::array<std::uint8_t, N> out{};
    if (bytes.size() != N) throw_length_mismatch();
    for (std::size_t i = 0; i < N; ++i) out[i] = bytes[i];
    return out;
}

} // namespace synergos::trust
