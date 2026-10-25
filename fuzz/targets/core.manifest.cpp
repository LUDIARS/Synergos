// SPEC-SY-decode-bounds の fuzz target。決められた型のエラーで拒否するのは正常で、それ以外の例外・クラッシュは発見。
#include <cstddef>
#include <cstdint>
#include "synergos/core/manifest_codec.hpp"
#include <stdexcept>
#include <string_view>

extern "C" int LLVMFuzzerTestOneInput(const std::uint8_t* data, std::size_t size) {
    const std::string_view text(reinterpret_cast<const char*>(data), size);
    try {
        const auto manifest = synergos::core::decode_manifest(text, "p");
        static_cast<void>(synergos::core::decode_manifest(synergos::core::encode_manifest(manifest, "p"), "p"));
    } catch (const synergos::core::DecodeTooLarge&) {
    } catch (const std::invalid_argument&) {
    }
    return 0;
}