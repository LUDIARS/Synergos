// @implements SPEC-SY-native-custody-journal
#include "synergos/custody/journal.hpp"
#include "synergos/trust/envelope.h"
#include "../partial_file.hpp"
#include <fstream>
#include <stdexcept>
#include <thread>
#include <vector>

namespace fs = std::filesystem;
using namespace synergos::custody;
namespace {
constexpr auto event = R"({"v":1,"ts":1800000000,"event_type":"custody.redeem","actor":{"principal_kind":"service_principal","service_principal":"curare:one"},"subject":{"org":"org-one","project_ref":"project-one","object_keys":["object-one"]},"change":{},"outcome":"success","reason_code":"capability.accepted","correlation":{"trace_id":"0123456789abcdef0123456789abcdef","cap_id":"jti-one"}})";
void require(bool value) { if (!value) throw std::runtime_error("Custody journal contract failed"); }
template<typename Action> void rejects(Action action) {
    bool denied = false;
    try { action(); } catch (const std::exception&) { denied = true; }
    require(denied);
}
struct Directory {
    fs::path path = detail::partial_name(fs::temp_directory_path());
    Directory() { require(fs::create_directory(path)); path = fs::canonical(path); }
    ~Directory() { std::error_code ignored; fs::remove_all(path, ignored); }
};
}
int main() {
    const Directory directory;
    JournalHead saved;
    {
        Journal journal(directory.path, "custody/org-one");
        require(journal.head().seq == 0);
        rejects([&] { Journal concurrent(directory.path, "custody/org-one"); });
        rejects([&] { journal.append("{}"); });
        require(journal.head().seq == 0);
        saved = journal.append(event);
        // Independently generated with Node's standard SHA-256 over zero32 ||
        // canonical row excluding row_hash, not the implementation under test.
        require(saved.seq == 1 && saved.root_hash == "4253d41e49a4a31cec3c2e1b30cd132aa7bc89b45e4082f8d7a2d87120881579");
        std::vector<std::thread> writers;
        for (unsigned i = 0; i < 4; ++i) writers.emplace_back([&] { journal.append(event); });
        for (auto& writer : writers) writer.join();
        require(journal.head().seq == 5);
        synergos::trust::Seed seed{};
        const synergos::trust::SigningKey key(seed); // Synthetic test-only key, never a runtime default.
        const auto checkpoint = journal.checkpoint("custody-test", key, 1800000000);
        require(synergos::trust::verify(synergos::trust::Ctx::journal_checkpoint, 1, checkpoint.payload,
            synergos::trust::Signer::custody, key.public_key(), checkpoint.signature) == checkpoint.payload);
        saved = journal.head();
    }
    {
        Journal recovered(directory.path, "custody/org-one");
        require(recovered.head().seq == saved.seq && recovered.head().root_hash == saved.root_hash);
    }
    rejects([&] { Journal wrong_stream(directory.path, "custody/org-two"); });
    // A torn append is evidence requiring explicit recovery, never silently
    // truncated. Opening twice must not mutate it or turn it into success.
    const auto path = directory.path / "custody-security.jsonl";
    {
        std::ofstream file(path, std::ios::binary | std::ios::app);
        file << "{\"seq\":";
    }
    const auto size = fs::file_size(path);
    rejects([&] { Journal torn(directory.path, "custody/org-one"); });
    rejects([&] { Journal torn(directory.path, "custody/org-one"); });
    require(fs::file_size(path) == size);
    const Directory altered;
    fs::copy_file(path, altered.path / "custody-security.jsonl");
    {
        std::fstream file(altered.path / "custody-security.jsonl", std::ios::binary | std::ios::in | std::ios::out);
        file.seekp(10); file.put('X');
    }
    rejects([&] { Journal corrupt(altered.path, "custody/org-one"); });
}
