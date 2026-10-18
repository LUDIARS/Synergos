#pragma once
#include <filesystem>

namespace synergos::custody::detail {
std::filesystem::path partial_name(const std::filesystem::path& parent);
// Arm only AFTER exclusive creation succeeds, so a collision cannot remove
// someone else's file. Allocate the path before opening the OS handle.
class PartialFile {
public:
    explicit PartialFile(std::filesystem::path path) : path_(std::move(path)) {}
    ~PartialFile() { if (owned_) { std::error_code ignored; std::filesystem::remove(path_, ignored); } }
    void own() noexcept { owned_ = true; }
    PartialFile(const PartialFile&) = delete;
    PartialFile& operator=(const PartialFile&) = delete;
private:
    std::filesystem::path path_;
    bool owned_ = false;
};
}
