// @implements SPEC-SY-native-custody-journal
#pragma once
#include "synergos/custody/journal.hpp"
#include <string>
#include <string_view>
#include <stdexcept>
namespace synergos::custody::detail {
// @implements SPEC-SY-native-custody-journal
class JournalRecordError final : public std::runtime_error {
public:
    // @implements SPEC-SY-native-custody-journal
    JournalRecordError();
};
inline constexpr std::uint64_t journal_spool_bytes = 1024ULL * 1024 * 1024;
inline constexpr std::size_t journal_row_bytes = 1024 * 1024;
std::string journal_event(std::string_view input);
std::string journal_row(std::string_view stream, const JournalHead& previous, std::string_view event);
JournalHead verify_journal_row(std::string_view stream, const JournalHead& previous, std::string_view row);
JournalHead journal_row_head(std::string_view row);
void journal_identifier(std::string_view value);
}
