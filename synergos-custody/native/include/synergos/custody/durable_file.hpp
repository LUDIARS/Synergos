#pragma once
#include <cstddef>
#include <filesystem>
#include <span>

namespace synergos::custody {
// Caller owns and locks the canonical storage directory. Destination must be a
// generated object key, never an untrusted request path. Does not replace an
// existing file. A durability failure after publication may leave destination
// present: callers must reconcile it rather than retry with replacement.
void write_new(const std::filesystem::path& destination, std::span<const std::byte> bytes);
}
