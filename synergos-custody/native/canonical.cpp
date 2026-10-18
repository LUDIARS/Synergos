// @implements SPEC-SY-opaque-custody
#include "synergos/custody/signature.hpp"
#include <yyjson.h>
#include <algorithm>
#include <cstdlib>
#include <memory>
#include <stdexcept>
#include <vector>

namespace synergos::custody {
namespace {
std::string serde_escapes(std::string value) {
    // serde_json uses lowercase hex in Unicode escapes. Skip escaped
    // backslashes so literal text such as "\\u001B" remains unchanged.
    for (std::size_t i = 0; i + 1 < value.size(); ++i) {
        if (value[i] != '\\') continue;
        if (value[i + 1] == 'u' && i + 5 < value.size()) {
            for (std::size_t j = i + 2; j <= i + 5; ++j) {
                if (value[j] >= 'A' && value[j] <= 'F') value[j] = static_cast<char>(value[j] - 'A' + 'a');
            }
            i += 5;
        } else ++i;
    }
    return value;
}
std::string scalar(yyjson_val* value) {
    if (yyjson_is_real(value)) throw std::invalid_argument("Custody signatures require integer JSON numbers");
    std::size_t length{};
    std::unique_ptr<char, decltype(&std::free)> bytes(yyjson_val_write(value, 0, &length), std::free);
    if (!bytes) throw std::invalid_argument("Cannot encode custody value");
    return serde_escapes({bytes.get(), length});
}
void append(yyjson_val* value, std::string& output, unsigned depth) {
    if (depth > 128) throw std::invalid_argument("Custody signature JSON nesting exceeds 128");
    if (yyjson_is_obj(value)) {
        std::vector<yyjson_val*> keys;
        auto iter = yyjson_obj_iter_with(value);
        while (auto* key = yyjson_obj_iter_next(&iter)) keys.push_back(key);
        const auto name = [](yyjson_val* key) { return std::string_view(yyjson_get_str(key), yyjson_get_len(key)); };
        std::sort(keys.begin(), keys.end(), [&](auto* a, auto* b) { return name(a) < name(b); });
        output += '{';
        for (std::size_t i = 0; i < keys.size(); ++i) {
            if (i && name(keys[i - 1]) == name(keys[i])) throw std::invalid_argument("Duplicate custody JSON key");
            if (i) output += ',';
            output += scalar(keys[i]);
            output += ':';
            append(yyjson_obj_iter_get_val(keys[i]), output, depth + 1);
        }
        output += '}';
    } else if (yyjson_is_arr(value)) {
        output += '[';
        auto iter = yyjson_arr_iter_with(value);
        bool first = true;
        while (auto* item = yyjson_arr_iter_next(&iter)) {
            if (!first) output += ',';
            first = false;
            append(item, output, depth + 1);
        }
        output += ']';
    } else output += scalar(value);
}
std::string quote(std::string_view value) {
    std::unique_ptr<yyjson_mut_doc, decltype(&yyjson_mut_doc_free)> doc(yyjson_mut_doc_new(nullptr), yyjson_mut_doc_free);
    if (!doc) throw std::bad_alloc();
    auto* text = yyjson_mut_strncpy(doc.get(), value.data(), value.size());
    if (!text) throw std::bad_alloc();
    std::size_t length{};
    std::unique_ptr<char, decltype(&std::free)> bytes(yyjson_mut_val_write(text, 0, &length), std::free);
    if (!bytes) throw std::invalid_argument("Invalid capability string");
    return serde_escapes({bytes.get(), length});
}
}
std::string canonical(std::string_view json) {
    if (json.size() > 1024 * 1024) throw std::invalid_argument("Custody signature JSON exceeds 1 MiB");
    std::unique_ptr<yyjson_doc, decltype(&yyjson_doc_free)> doc(yyjson_read(json.data(), json.size(), 0), yyjson_doc_free);
    if (!doc) throw std::invalid_argument("Invalid custody JSON");
    std::string result;
    append(yyjson_doc_get_root(doc.get()), result, 0);
    return result;
}
std::string capability_message(std::string_view method, std::string_view key,
                               std::int64_t expires, std::string_view name) {
    return "{\"expires\":" + std::to_string(expires) + ",\"key\":" + quote(key)
        + ",\"method\":" + quote(method) + ",\"name\":" + quote(name) + "}";
}
}
