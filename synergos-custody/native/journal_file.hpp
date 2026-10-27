#pragma once
#include <cstdint>
#include <filesystem>
#include <memory>
#include <span>
#include <string_view>

namespace synergos::custody::detail {
// Sole owner of the locked journal descriptor. An exception never establishes
// that retry is safe: the higher owner poisons itself on append/flush failure.
class JournalFile final {
public:
    explicit JournalFile(const std::filesystem::path& root);
    ~JournalFile();
    JournalFile(const JournalFile&) = delete;
    JournalFile& operator=(const JournalFile&) = delete;
    std::uint64_t size() const;
    std::size_t read(std::uint64_t offset, std::span<char> destination);
    void append_and_flush(std::string_view row);
private:
    struct Handle;
    std::unique_ptr<Handle> handle_;
};
}
