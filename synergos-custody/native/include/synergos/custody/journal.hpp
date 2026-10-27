#pragma once
#include "synergos/trust/crypto.h"
#include <cstdint>
#include <filesystem>
#include <memory>
#include <string>
#include <string_view>

namespace synergos::custody {
struct JournalHead {
    std::uint64_t seq{};
    std::string root_hash = std::string(64, '0');
};
struct JournalCheckpoint {
    std::string payload, kid;
    trust::Signature signature;
};
// An exclusive, bounded append/flush owner. This is audit storage, never a
// capability authorization, budget reservation or object commit transaction.
class Journal final {
public:
    Journal(const std::filesystem::path& private_root, std::string stream);
    ~Journal();
    Journal(const Journal&) = delete;
    Journal& operator=(const Journal&) = delete;
    JournalHead head() const;
    JournalHead append(std::string_view redacted_event_json);
    JournalCheckpoint checkpoint(std::string_view kid, const trust::SigningKey& key, std::int64_t ts) const;
private:
    struct State;
    std::unique_ptr<State> state_;
};
}
