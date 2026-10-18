// @implements SPEC-SY-custody-durability
#include "synergos/custody/durable_file.hpp"
#include "partial_file.hpp"
#include <algorithm>
#include <cerrno>
#include <fcntl.h>
#include <system_error>
#include <unistd.h>

namespace synergos::custody {
namespace {
[[noreturn]] void fail(const char* operation) { throw std::system_error(errno, std::generic_category(), operation); }
class File {
public:
    explicit File(int value) : value_(value) {}
    ~File() { if (value_ >= 0) ::close(value_); }
    File(const File&) = delete;
    File& operator=(const File&) = delete;
    int get() const { return value_; }
    void close() {
        const auto fd = value_;
        value_ = -1; // Never retry close: another thread may reuse this descriptor.
        if (::close(fd) != 0) fail("Close durable file");
    }
private:
    int value_;
};
void sync(int fd) {
    int result;
    do { result = ::fsync(fd); } while (result != 0 && errno == EINTR);
    if (result != 0) fail("Sync durable file");
}
}
void write_new(const std::filesystem::path& destination, std::span<const std::byte> bytes) {
    const auto parent = destination.parent_path();
    const auto partial = detail::partial_name(parent);
    detail::PartialFile cleanup(partial);
    const auto fd = ::open(partial.c_str(), O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC | O_NOFOLLOW, 0600);
    if (fd < 0) fail("Create exclusive partial file");
    cleanup.own();
    File file(fd);
    while (!bytes.empty()) {
        const auto size = std::min<std::size_t>(bytes.size(), 1024 * 1024);
        const auto written = ::write(file.get(), bytes.data(), size);
        if (written < 0) { if (errno == EINTR) continue; fail("Write durable file"); }
        if (written == 0) throw std::runtime_error("Zero-byte durable write");
        bytes = bytes.subspan(static_cast<std::size_t>(written));
    }
    sync(file.get());
    file.close();
    const auto directory_fd = ::open(parent.c_str(), O_RDONLY | O_DIRECTORY | O_CLOEXEC);
    if (directory_fd < 0) fail("Open durable directory");
    File directory(directory_fd);
    if (::link(partial.c_str(), destination.c_str()) != 0) fail("Publish durable file");
    sync(directory.get());
}
}
