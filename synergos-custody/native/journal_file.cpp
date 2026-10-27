// @implements SPEC-SY-native-custody-journal
#include "journal_file.hpp"
#include <algorithm>
#include <limits>
#include <stdexcept>
#include <system_error>
#if defined(_WIN32)
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#else
#include <cerrno>
#include <fcntl.h>
#include <sys/file.h>
#include <sys/stat.h>
#include <unistd.h>
#endif

namespace synergos::custody::detail {
namespace {
[[noreturn]] void fail(const char* operation) {
#if defined(_WIN32)
    throw std::system_error(static_cast<int>(GetLastError()), std::system_category(), operation);
#else
    throw std::system_error(errno, std::generic_category(), operation);
#endif
}
}
struct JournalFile::Handle {
#if defined(_WIN32)
    HANDLE file = INVALID_HANDLE_VALUE;
    ~Handle() { if (file != INVALID_HANDLE_VALUE) CloseHandle(file); }
#else
    int file = -1;
    ~Handle() { if (file >= 0) close(file); }
#endif
};

JournalFile::JournalFile(const std::filesystem::path& root) : handle_(std::make_unique<Handle>()) {
    if (!root.is_absolute() || std::filesystem::canonical(root) != root)
        throw std::invalid_argument("Journal root must be absolute and canonical");
    const auto path = root / "custody-security.jsonl";
#if defined(_WIN32)
    handle_->file = CreateFileW(path.c_str(), GENERIC_READ | GENERIC_WRITE, 0, nullptr,
        OPEN_ALWAYS, FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_WRITE_THROUGH, nullptr);
    if (handle_->file == INVALID_HANDLE_VALUE) fail("Open exclusive custody journal");
    BY_HANDLE_FILE_INFORMATION info{};
    if (!GetFileInformationByHandle(handle_->file, &info)) fail("Inspect custody journal");
    if ((info.dwFileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY)) ||
        info.nNumberOfLinks != 1 || GetFileType(handle_->file) != FILE_TYPE_DISK)
        throw std::runtime_error("Custody journal must be a single regular file");
    // Share mode zero excludes readers and writers in other processes. The
    // private parent directory remains a required service-account boundary.
#else
    handle_->file = open(path.c_str(), O_RDWR | O_CREAT | O_CLOEXEC | O_NOFOLLOW, 0600);
    if (handle_->file < 0) fail("Open custody journal");
    if (flock(handle_->file, LOCK_EX | LOCK_NB) != 0) fail("Lock custody journal");
    struct stat info{};
    if (fstat(handle_->file, &info) != 0) fail("Inspect custody journal");
    if (!S_ISREG(info.st_mode) || info.st_nlink != 1 || (info.st_mode & 0077) != 0 || info.st_uid != geteuid())
        throw std::runtime_error("Custody journal must be a private owned regular file");
    // Persist creation before accepting any operation. Closing always releases
    // the lock, including exceptions during this constructor.
    const auto directory = open(root.c_str(), O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW);
    if (directory < 0) fail("Open custody journal directory");
    const auto result = fsync(directory);
    const auto saved_errno = errno;
    close(directory);
    if (result != 0) { errno = saved_errno; fail("Flush custody journal directory"); }
#endif
}
JournalFile::~JournalFile() = default;

std::uint64_t JournalFile::size() const {
#if defined(_WIN32)
    LARGE_INTEGER value{};
    if (!GetFileSizeEx(handle_->file, &value)) fail("Size custody journal");
    if (value.QuadPart < 0) throw std::runtime_error("Invalid custody journal size");
    return static_cast<std::uint64_t>(value.QuadPart);
#else
    struct stat value{};
    if (fstat(handle_->file, &value) != 0) fail("Size custody journal");
    if (value.st_size < 0) throw std::runtime_error("Invalid custody journal size");
    return static_cast<std::uint64_t>(value.st_size);
#endif
}
std::size_t JournalFile::read(std::uint64_t offset, std::span<char> destination) {
    if (offset > static_cast<std::uint64_t>(std::numeric_limits<std::int64_t>::max()))
        throw std::length_error("Custody journal offset exceeds file bounds");
    const auto limit = std::min<std::size_t>(destination.size(), 64 * 1024);
#if defined(_WIN32)
    LARGE_INTEGER where{};
    where.QuadPart = static_cast<LONGLONG>(offset);
    if (!SetFilePointerEx(handle_->file, where, nullptr, FILE_BEGIN)) fail("Seek custody journal");
    DWORD count{};
    if (!ReadFile(handle_->file, destination.data(), static_cast<DWORD>(limit), &count, nullptr)) fail("Read custody journal");
    return count;
#else
    ssize_t count;
    do { count = pread(handle_->file, destination.data(), limit, static_cast<off_t>(offset)); }
    while (count < 0 && errno == EINTR);
    if (count < 0) fail("Read custody journal");
    return static_cast<std::size_t>(count);
#endif
}
void JournalFile::append_and_flush(std::string_view row) {
#if defined(_WIN32)
    LARGE_INTEGER end{};
    if (!SetFilePointerEx(handle_->file, end, nullptr, FILE_END)) fail("Seek custody journal end");
#else
    if (lseek(handle_->file, 0, SEEK_END) < 0) fail("Seek custody journal end");
#endif
    while (!row.empty()) {
        const auto size = std::min<std::size_t>(row.size(), 64 * 1024);
#if defined(_WIN32)
        DWORD count{};
        if (!WriteFile(handle_->file, row.data(), static_cast<DWORD>(size), &count, nullptr)) fail("Write custody journal");
#else
        ssize_t count;
        do { count = write(handle_->file, row.data(), size); } while (count < 0 && errno == EINTR);
        if (count < 0) fail("Write custody journal");
#endif
        if (count == 0) throw std::runtime_error("Zero-byte custody journal write");
        row.remove_prefix(static_cast<std::size_t>(count));
    }
#if defined(_WIN32)
    if (!FlushFileBuffers(handle_->file)) fail("Flush custody journal");
#else
    if (fsync(handle_->file) != 0) fail("Flush custody journal");
#endif
}
}
