// SPEC-SY-decode-bounds の fuzz target。決められた型のエラーで拒否するのは正常で、それ以外の例外・クラッシュは発見。
#include <cstddef>
#include <cstdint>
#include "synergos/trust/error.h"
#include "synergos/trust/jcs.h"
#include <cstdio>
#include <cstdlib>
#include <string>
#include <string_view>

// 正規形は冪等でなければならない (canonicalize(canonicalize(x)) == canonicalize(x))。
extern "C" int LLVMFuzzerTestOneInput(const std::uint8_t* data, std::size_t size) {
    try {
        const auto once = synergos::trust::jcs::canonicalize(std::string_view(reinterpret_cast<const char*>(data), size));
        if (synergos::trust::jcs::canonicalize(once) != once) {
            std::fputs("JCS is not idempotent\n", stderr);
            std::abort();
        }
    } catch (const synergos::trust::TrustError&) {
    }
    return 0;
}
