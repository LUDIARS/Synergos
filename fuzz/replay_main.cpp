// fuzz target の再生ドライバ (libFuzzer を使わない毎回の試験)。
// 見本入力 (fuzz/<decoder_id>/) を、そのままと決まった変形 (切り詰め・ビット反転・重複) で target に流す。
// target が決められた型以外の例外を投げたら、入力の名前と変形を出して失敗する。
#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <exception>
#include <filesystem>
#include <fstream>
#include <iterator>
#include <stdexcept>
#include <string>
#include <vector>

extern "C" int LLVMFuzzerTestOneInput(const std::uint8_t* data, std::size_t size);

namespace {

using Bytes = std::vector<std::uint8_t>;

Bytes read_file(const std::filesystem::path& path) {
    std::ifstream in(path, std::ios::binary);
    return Bytes(std::istreambuf_iterator<char>(in), std::istreambuf_iterator<char>());
}

void run(const Bytes& input, const std::string& label) {
    try {
        LLVMFuzzerTestOneInput(input.data(), input.size());
    } catch (const std::exception& e) {
        throw std::runtime_error(label + ": " + e.what());
    }
}

/// 決まった変形。乱数を使わないので毎回同じ入力になる。
std::size_t replay(const Bytes& seed, const std::string& name) {
    std::size_t count = 0;
    run(seed, name);
    ++count;
    for (std::size_t cut = 0; cut < seed.size(); cut += seed.size() / 8 + 1) {
        run(Bytes(seed.begin(), seed.begin() + static_cast<std::ptrdiff_t>(cut)), name + " truncated@" + std::to_string(cut));
        ++count;
    }
    const std::size_t step = seed.size() / 64 + 1;
    for (std::size_t at = 0; at < seed.size(); at += step) {
        for (const std::uint8_t mask : {std::uint8_t{0x01}, std::uint8_t{0x80}}) {
            auto flipped = seed;
            flipped[at] = static_cast<std::uint8_t>(flipped[at] ^ mask);
            run(flipped, name + " flip@" + std::to_string(at));
            ++count;
        }
    }
    auto doubled = seed;
    doubled.insert(doubled.end(), seed.begin(), seed.end());
    run(doubled, name + " doubled");
    return count + 1;
}

} // namespace

int main(int argc, char** argv) {
    if (argc != 2) {
        std::fprintf(stderr, "usage: %s <corpus_dir>\n", argv[0]);
        return 2;
    }
    try {
        std::vector<std::filesystem::path> seeds;
        for (const auto& entry : std::filesystem::directory_iterator(argv[1]))
            if (entry.is_regular_file()) seeds.push_back(entry.path());
        if (seeds.empty()) throw std::runtime_error("corpus is empty");
        std::size_t inputs = 0;
        for (const auto& path : seeds) inputs += replay(read_file(path), path.filename().string());
        std::printf("replayed %zu inputs from %zu seeds\n", inputs, seeds.size());
    } catch (const std::exception& e) {
        std::fprintf(stderr, "FAIL: %s\n", e.what());
        return 1;
    }
    return 0;
}
