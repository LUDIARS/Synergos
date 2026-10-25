// @implements SPEC-SY-opaque-custody
#if !defined(_WIN32)
#include "digest.hpp"
#include <openssl/evp.h>
#include <stdexcept>

namespace synergos::custody::detail {
Digest sha256(std::string_view message) {
    Digest result{};
    std::size_t length{};
    if (EVP_Q_digest(nullptr, "SHA256", nullptr, message.data(), message.size(), result.data(), &length) != 1
        || length != result.size()) throw std::runtime_error("SHA256 failed");
    return result;
}
Digest hmac_sha256(std::string_view secret, std::string_view message) {
    Digest result{};
    std::size_t length{};
    // OpenSSL needs a non-null key pointer even for an empty key.
    const char empty = 0;
    if (!EVP_Q_mac(nullptr, "HMAC", nullptr, "SHA256", nullptr,
                   secret.empty() ? &empty : secret.data(), secret.size(),
                   reinterpret_cast<const unsigned char*>(message.data()), message.size(),
                   result.data(), result.size(), &length) || length != result.size()) {
        throw std::runtime_error("HMAC SHA256 failed");
    }
    return result;
}
}
#endif
