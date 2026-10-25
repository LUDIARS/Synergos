// SPEC-SY-decode-bounds の fuzz target。決められた型のエラーで拒否するのは正常で、それ以外の例外・クラッシュは発見。
#include <cstddef>
#include <cstdint>
#include "synergos/trust/error.h"
#include "synergos/trust/lct.h"
#include <map>
#include <string>
#include <string_view>

// 任意の token を固定の発行者で検証する。spec/vectors/lct.json と同じ発行鍵 (seed 0x11 / 0x12) を使う。
extern "C" int LLVMFuzzerTestOneInput(const std::uint8_t* data, std::size_t size) {
    using namespace synergos::trust;
    static const std::map<std::string, lct::IssuerKey> issuers = [] {
        Seed cap{}; cap.fill(0x11);
        Seed del{}; del.fill(0x12);
        return std::map<std::string, lct::IssuerKey>{
            {"custody-cap-1", {SigningKey(cap).public_key(), lct::Profile::custody_capability}},
            {"custody-del-1", {SigningKey(del).public_key(), lct::Profile::custody_delegation}}};
    }();
    const std::string_view token(reinterpret_cast<const char*>(data), size);
    for (const auto profile : {lct::Profile::custody_capability, lct::Profile::custody_delegation}) {
        try {
            static_cast<void>(lct::verify(token, issuers, {}, 1800000010, {profile}));
        } catch (const TrustError&) {
        }
    }
    return 0;
}
