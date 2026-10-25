// @implements SPEC-SY-signed-envelope SPEC-SY-lct-v1
#include "synergos/trust/crypto.h"
#include "synergos/trust/error.h"
#include <monocypher-ed25519.h>
#include <monocypher.h>
#include <utility>
#if defined(_WIN32)
#include <windows.h>
#include <bcrypt.h>
#else
#include <fstream>
#endif

namespace synergos::trust {

SigningKey::SigningKey(const Seed& seed) {
    Seed copy = seed; // crypto_ed25519_key_pair は seed を消去する。
    crypto_ed25519_key_pair(secret_.data(), public_.data(), copy.data());
}

SigningKey::~SigningKey() { crypto_wipe(secret_.data(), secret_.size()); }

SigningKey::SigningKey(SigningKey&& other) noexcept : secret_(other.secret_), public_(other.public_) {
    crypto_wipe(other.secret_.data(), other.secret_.size());
}

SigningKey& SigningKey::operator=(SigningKey&& other) noexcept {
    if (this != &other) {
        crypto_wipe(secret_.data(), secret_.size());
        secret_ = other.secret_;
        public_ = other.public_;
        crypto_wipe(other.secret_.data(), other.secret_.size());
    }
    return *this;
}

Seed random_seed() {
    Seed seed{};
#if defined(_WIN32)
    if (BCryptGenRandom(nullptr, seed.data(), static_cast<ULONG>(seed.size()), BCRYPT_USE_SYSTEM_PREFERRED_RNG) != 0)
        throw std::runtime_error("BCryptGenRandom failed");
#else
    std::ifstream urandom("/dev/urandom", std::ios::binary);
    if (!urandom.read(reinterpret_cast<char*>(seed.data()), static_cast<std::streamsize>(seed.size())))
        throw std::runtime_error("cannot read /dev/urandom");
#endif
    return seed;
}

bool ed25519_verify(const PublicKey& key, std::string_view message, const Signature& signature) {
    return crypto_ed25519_check(signature.data(), key.data(),
                                reinterpret_cast<const std::uint8_t*>(message.data()), message.size()) == 0;
}

namespace {
constexpr char alphabet[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
int index_of(char c) {
    if (c >= 'A' && c <= 'Z') return c - 'A';
    if (c >= 'a' && c <= 'z') return c - 'a' + 26;
    if (c >= '0' && c <= '9') return c - '0' + 52;
    if (c == '-') return 62;
    if (c == '_') return 63;
    return -1;
}
} // namespace

std::string base64url_encode(const std::uint8_t* data, std::size_t size) {
    std::string out;
    out.reserve((size * 4 + 2) / 3);
    std::size_t i = 0;
    for (; i + 3 <= size; i += 3) {
        const std::uint32_t v = (data[i] << 16) | (data[i + 1] << 8) | data[i + 2];
        out += alphabet[(v >> 18) & 63];
        out += alphabet[(v >> 12) & 63];
        out += alphabet[(v >> 6) & 63];
        out += alphabet[v & 63];
    }
    if (size - i == 1) {
        const std::uint32_t v = data[i] << 16;
        out += alphabet[(v >> 18) & 63];
        out += alphabet[(v >> 12) & 63];
    } else if (size - i == 2) {
        const std::uint32_t v = (data[i] << 16) | (data[i + 1] << 8);
        out += alphabet[(v >> 18) & 63];
        out += alphabet[(v >> 12) & 63];
        out += alphabet[(v >> 6) & 63];
    }
    return out;
}

std::vector<std::uint8_t> base64url_decode(std::string_view text) {
    if (text.size() % 4 == 1) throw TrustError(Code::encoding_invalid, "bad base64url length");
    std::vector<std::uint8_t> out;
    out.reserve(text.size() * 3 / 4);
    std::uint32_t acc = 0;
    int bits = 0;
    for (const char c : text) {
        const int v = index_of(c);
        if (v < 0) throw TrustError(Code::encoding_invalid, "bad base64url character");
        acc = (acc << 6) | static_cast<std::uint32_t>(v);
        bits += 6;
        if (bits >= 8) {
            bits -= 8;
            out.push_back(static_cast<std::uint8_t>((acc >> bits) & 0xff));
        }
    }
    // 余りビットは 0 でなければならない (正準形だけを受け付ける)。
    if (bits > 0 && (acc & ((1u << bits) - 1)) != 0) throw TrustError(Code::encoding_invalid, "non-canonical base64url");
    return out;
}

void throw_length_mismatch() { throw TrustError(Code::encoding_invalid, "unexpected decoded length"); }

} // namespace synergos::trust
