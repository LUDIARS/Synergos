// @implements SPEC-SY-opaque-custody
#if defined(_WIN32)
#include "digest.hpp"
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <bcrypt.h>
#include <algorithm>
#include <limits>
#include <stdexcept>
#include <vector>

namespace synergos::custody::detail {
namespace {
void checked(NTSTATUS status) {
    if (status < 0) throw std::runtime_error("Windows cryptography operation failed");
}
struct Provider {
    BCRYPT_ALG_HANDLE value{};
    explicit Provider(bool hmac) {
        checked(BCryptOpenAlgorithmProvider(&value, BCRYPT_SHA256_ALGORITHM, nullptr,
                                            hmac ? BCRYPT_ALG_HANDLE_HMAC_FLAG : 0));
    }
    ~Provider() { if (value) BCryptCloseAlgorithmProvider(value, 0); }
    Provider(const Provider&) = delete;
    Provider& operator=(const Provider&) = delete;
};
struct Hash {
    BCRYPT_HASH_HANDLE value{};
    std::vector<UCHAR> storage;
    Hash(Provider& provider, std::string_view secret) {
        if (secret.size() > std::numeric_limits<ULONG>::max()) throw std::length_error("HMAC key too large");
        ULONG size{}, copied{};
        checked(BCryptGetProperty(provider.value, BCRYPT_OBJECT_LENGTH,
                                  reinterpret_cast<PUCHAR>(&size), sizeof(size), &copied, 0));
        storage.resize(size);
        checked(BCryptCreateHash(provider.value, &value, storage.data(), size,
            reinterpret_cast<PUCHAR>(const_cast<char*>(secret.data())), static_cast<ULONG>(secret.size()), 0));
    }
    ~Hash() {
        if (value) BCryptDestroyHash(value);
        if (!storage.empty()) SecureZeroMemory(storage.data(), storage.size());
    }
    Hash(const Hash&) = delete;
    Hash& operator=(const Hash&) = delete;
};
Digest digest(std::string_view message, std::string_view secret, bool hmac) {
    Provider provider(hmac);
    Hash hash(provider, secret);
    while (!message.empty()) {
        const auto size = static_cast<ULONG>(std::min<std::size_t>(message.size(), 1024 * 1024));
        checked(BCryptHashData(hash.value, reinterpret_cast<PUCHAR>(const_cast<char*>(message.data())), size, 0));
        message.remove_prefix(size);
    }
    Digest result{};
    checked(BCryptFinishHash(hash.value, result.data(), static_cast<ULONG>(result.size()), 0));
    return result;
}
}
Digest sha256(std::string_view message) { return digest(message, {}, false); }
Digest hmac_sha256(std::string_view secret, std::string_view message) { return digest(message, secret, true); }
}
#endif
