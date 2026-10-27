// @implements SPEC-SY-native-custody-journal
#include "synergos/custody/journal.hpp"
#include "journal_file.hpp"
#include "journal_row.hpp"
#include "synergos/trust/envelope.h"
#include "synergos/trust/jcs.h"
#include <array>
#include <mutex>
#include <stdexcept>
#include <utility>

namespace synergos::custody {
struct Journal::State {
    detail::JournalFile file;
    std::string stream;
    JournalHead head;
    std::uint64_t bytes{};
    bool poisoned = false;
    mutable std::mutex mutex;
    State(const std::filesystem::path& root, std::string name) : file(root), stream(std::move(name)) {
        detail::journal_identifier(stream);
        bytes = file.size();
        if (bytes > detail::journal_spool_bytes) throw std::runtime_error("Custody journal spool exceeds limit");
        std::array<char, 64*1024> buffer{};
        std::string row;
        std::uint64_t offset{};
        while (offset < bytes) {
            const auto count = file.read(offset, buffer);
            if (count == 0 || count > bytes-offset) throw std::runtime_error("Custody journal changed during recovery");
            offset += count;
            for (std::size_t i = 0; i < count; ++i) {
                if (buffer[i] == '\n') { head = detail::verify_journal_row(stream, head, row); row.clear(); }
                else {
                    if (row.size() >= detail::journal_row_bytes-1) throw std::runtime_error("Custody journal row exceeds limit");
                    row += buffer[i];
                }
            }
        }
        if (!row.empty() || file.size() != bytes) throw std::runtime_error("Custody journal incomplete recovery");
    }
    void require_healthy() const {
        if (poisoned) throw std::runtime_error("Custody journal requires recovery after uncertain write");
    }
};
Journal::Journal(const std::filesystem::path& root, std::string stream) {
    detail::journal_identifier(stream); // Validate config before opening/creating a file.
    state_ = std::make_unique<State>(root, std::move(stream));
}
Journal::~Journal() = default;
JournalHead Journal::head() const {
    const std::lock_guard lock(state_->mutex);
    state_->require_healthy();
    return state_->head;
}
JournalHead Journal::append(std::string_view input) {
    const auto event = detail::journal_event(input);
    const std::lock_guard lock(state_->mutex);
    state_->require_healthy();
    auto row = detail::journal_row(state_->stream, state_->head, event);
    auto next = detail::journal_row_head(row);
    row += '\n';
    if (row.size() > detail::journal_spool_bytes-state_->bytes) throw std::runtime_error("Custody journal spool full");
    try { state_->file.append_and_flush(row); }
    catch (...) {
        // There is no safe inference about whether a partial/full row reached
        // stable storage. Never attempt another append before recovery.
        state_->poisoned = true;
        throw;
    }
    state_->bytes += row.size();
    state_->head = std::move(next);
    return state_->head;
}
JournalCheckpoint Journal::checkpoint(std::string_view kid, const trust::SigningKey& key, std::int64_t ts) const {
    detail::journal_identifier(kid);
    if (ts < 0 || ts > trust::jcs::max_safe_integer) throw std::invalid_argument("Invalid custody checkpoint time");
    const std::lock_guard lock(state_->mutex);
    state_->require_healthy();
    const auto payload = trust::jcs::canonicalize("{\"stream\":" + trust::jcs::quote(state_->stream) +
        ",\"seq\":" + std::to_string(state_->head.seq) + ",\"root_hash\":" + trust::jcs::quote(state_->head.root_hash) +
        ",\"ts\":" + std::to_string(ts) + "}");
    return {payload, std::string(kid), trust::sign(trust::Ctx::journal_checkpoint, 1, payload, trust::Signer::custody, key)};
}
}
