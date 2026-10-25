// SPEC-SY-decode-bounds の fuzz target。決められた型のエラーで拒否するのは正常で、それ以外の例外・クラッシュは発見。
#include <cstddef>
#include <cstdint>
#include "synergos/ipc/frame.hpp"
#include <algorithm>
#include <span>

// 入力を不揃いな塊で流し、1 MiB 上限・途中切断・連結を通す。
extern "C" int LLVMFuzzerTestOneInput(const std::uint8_t* data, std::size_t size) {
    synergos::ipc::FrameDecoder decoder;
    const std::span<const std::uint8_t> bytes(data, size);
    std::size_t offset = 0;
    std::size_t step = 1;
    try {
        while (offset < bytes.size()) {
            const auto chunk = bytes.subspan(offset, std::min(step, bytes.size() - offset));
            const auto progress = decoder.consume(chunk);
            offset += progress.consumed;
            if (progress.complete) static_cast<void>(decoder.take());
            else if (progress.consumed == 0) break;
            step = step * 3 % 97 + 1;
        }
        decoder.finish();
    } catch (const synergos::ipc::FrameError&) {
    }
    return 0;
}