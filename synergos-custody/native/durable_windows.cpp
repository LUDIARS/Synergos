// @implements SPEC-SY-custody-durability
#include "synergos/custody/durable_file.hpp"
#include "partial_file.hpp"
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <algorithm>
#include <system_error>

namespace synergos::custody {
namespace {
[[noreturn]] void fail(const char* operation) {
    throw std::system_error(static_cast<int>(GetLastError()), std::system_category(), operation);
}
class File {
public:
    explicit File(HANDLE value) : value_(value) {}
    ~File() { if (value_ != INVALID_HANDLE_VALUE) CloseHandle(value_); }
    File(const File&) = delete;
    File& operator=(const File&) = delete;
    HANDLE get() const { return value_; }
    void close() {
        if (!CloseHandle(value_)) fail("Close durable file");
        value_ = INVALID_HANDLE_VALUE;
    }
private:
    HANDLE value_;
};
}
void write_new(const std::filesystem::path& destination, std::span<const std::byte> bytes) {
    const auto partial = detail::partial_name(destination.parent_path());
    detail::PartialFile cleanup(partial);
    const auto handle = CreateFileW(partial.c_str(), GENERIC_WRITE, 0, nullptr,
                                    CREATE_NEW, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (handle == INVALID_HANDLE_VALUE) fail("Create exclusive partial file");
    // File closes before PartialFile removes the path on exception.
    cleanup.own();
    File file(handle);
    while (!bytes.empty()) {
        const auto size = static_cast<DWORD>(std::min<std::size_t>(bytes.size(), 1024 * 1024));
        DWORD written{};
        if (!WriteFile(file.get(), bytes.data(), size, &written, nullptr)) fail("Write durable file");
        if (written == 0) throw std::runtime_error("Zero-byte durable write");
        bytes = bytes.subspan(written);
    }
    if (!FlushFileBuffers(file.get())) fail("Flush durable file");
    file.close();
    // Deliberately omit REPLACE_EXISTING: published custody objects are immutable.
    if (!MoveFileExW(partial.c_str(), destination.c_str(), MOVEFILE_WRITE_THROUGH)) fail("Publish durable file");
}
}
