// SPEC-SY-decode-bounds の fuzz target。決められた型のエラーで拒否するのは正常で、それ以外の例外・クラッシュは発見。
#include <cstddef>
#include <cstdint>
#include "synergos/custody/signature.hpp"
#include <stdexcept>
#include <string_view>

extern "C" int LLVMFuzzerTestOneInput(const std::uint8_t* data, std::size_t size) {
    try {
        static_cast<void>(synergos::custody::canonical(std::string_view(reinterpret_cast<const char*>(data), size)));
    } catch (const std::invalid_argument&) {
    }
    return 0;
}