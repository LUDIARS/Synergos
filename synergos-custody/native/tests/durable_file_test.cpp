#include "synergos/custody/durable_file.hpp"
#include "../partial_file.hpp"
#include <fstream>
#include <iterator>
#include <stdexcept>
#include <string>

using namespace synergos::custody;
namespace fs = std::filesystem;
namespace {
void require(bool value) { if (!value) throw std::runtime_error("Durable publication contract failed"); }
std::span<const std::byte> bytes(const std::string& value) {
    return std::as_bytes(std::span(value.data(), value.size()));
}
std::string read(const fs::path& path) {
    std::ifstream stream(path, std::ios::binary);
    return {std::istreambuf_iterator<char>(stream), std::istreambuf_iterator<char>()};
}
struct TestDirectory {
    fs::path path = detail::partial_name(fs::temp_directory_path());
    TestDirectory() { if (!fs::create_directory(path)) throw std::runtime_error("Test directory collision"); }
    ~TestDirectory() { std::error_code ignored; fs::remove_all(path, ignored); }
};
}
int main() {
    const TestDirectory temporary;
    const auto directory = temporary.path / fs::path(u8"custody-保存");
    fs::create_directory(directory);
    const auto destination = directory / "object.bin";
    const std::string original = std::string("a\0b", 3) + std::string(2 * 1024 * 1024 + 1, 'x');
    write_new(destination, bytes(original));
    require(read(destination) == original);
    bool refused = false;
    try { write_new(destination, bytes(std::string("replacement"))); }
    catch (const std::system_error&) { refused = true; }
    require(refused && read(destination) == original);
    write_new(directory / "empty", {});
    require(fs::file_size(directory / "empty") == 0);
    fs::create_directory(directory / "occupied");
    refused = false;
    try { write_new(directory / "occupied", bytes(std::string("data"))); }
    catch (const std::system_error&) { refused = true; }
    require(refused && fs::is_directory(directory / "occupied"));
    // Success and both failed publication paths must remove owned partials.
    for (const auto& entry : fs::directory_iterator(directory)) require(entry.path().extension() != ".part");
}
