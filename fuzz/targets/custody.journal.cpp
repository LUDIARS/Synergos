// @implements SPEC-SY-native-custody-journal SPEC-SY-decode-bounds
#include "../../synergos-custody/native/journal_row.hpp"
#include "synergos/trust/error.h"
#include <cstddef>
#include <cstdint>
#include <string_view>

extern "C" int LLVMFuzzerTestOneInput(const std::uint8_t* data, std::size_t size) {
    using namespace synergos::custody;
    if (size >= detail::journal_row_bytes) return 0;
    try {
        static_cast<void>(detail::verify_journal_row("custody/org-one", {},
            std::string_view(reinterpret_cast<const char*>(data), size)));
    } catch (const detail::JournalRecordError&) {
    } catch (const synergos::trust::TrustError&) {
    }
    return 0;
}
