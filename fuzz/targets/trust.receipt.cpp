// SPEC-SY-decode-bounds の fuzz target。決められた型のエラーで拒否するのは正常で、それ以外の例外・クラッシュは発見。
#include <cstddef>
#include <cstdint>
#include "synergos/trust/error.h"
#include "synergos/trust/receipt.h"
#include <algorithm>
#include <map>
#include <string>
#include <string_view>

// 先頭 64 バイトを署名、残りを本体として検証する。spec/vectors/receipt.json と同じ custody 鍵 (seed 0x61)。
extern "C" int LLVMFuzzerTestOneInput(const std::uint8_t* data, std::size_t size) {
    using namespace synergos::trust;
    static const std::map<std::string, PublicKey> pinned = [] {
        Seed seed{}; seed.fill(0x61);
        return std::map<std::string, PublicKey>{{"receipt-1", SigningKey(seed).public_key()}};
    }();
    Signature signature{};
    const auto head = std::min(size, signature.size());
    std::copy(data, data + head, signature.begin());
    try {
        static_cast<void>(receipt::verify(std::string_view(reinterpret_cast<const char*>(data) + head, size - head), signature,
                                          pinned, 1800000000));
    } catch (const TrustError&) {
    }
    return 0;
}
