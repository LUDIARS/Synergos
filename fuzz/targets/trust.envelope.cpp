// SPEC-SY-decode-bounds の fuzz target。決められた型のエラーで拒否するのは正常で、それ以外の例外・クラッシュは発見。
#include <cstddef>
#include <cstdint>
#include "synergos/trust/envelope.h"
#include "synergos/trust/error.h"
#include <string_view>

// 任意の payload を固定の鍵・署名で検証する。正規化と署名検証の両方を通す。
extern "C" int LLVMFuzzerTestOneInput(const std::uint8_t* data, std::size_t size) {
    using namespace synergos::trust;
    static const SigningKey key(Seed{});
    static const Signature signature = sign(Ctx::gossip, 1, "{}", Signer::node, key);
    try {
        static_cast<void>(verify(Ctx::gossip, 1, std::string_view(reinterpret_cast<const char*>(data), size), Signer::node,
                                 key.public_key(), signature));
    } catch (const TrustError&) {
    }
    return 0;
}
