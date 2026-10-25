// vector ファイルを書くための yyjson 可変文書の薄いラッパ (テスト専用)。
#pragma once
#include <yyjson.h>
#include <cstdint>
#include <stdexcept>
#include <string>
#include <string_view>

namespace synergos::trust::test {

class JsonWriter {
public:
    JsonWriter() : doc_(yyjson_mut_doc_new(nullptr)) {}
    ~JsonWriter() { yyjson_mut_doc_free(doc_); }
    JsonWriter(const JsonWriter&) = delete;
    JsonWriter& operator=(const JsonWriter&) = delete;

    yyjson_mut_val* obj() { return yyjson_mut_obj(doc_); }
    yyjson_mut_val* arr() { return yyjson_mut_arr(doc_); }
    yyjson_mut_val* str(std::string_view text) { return yyjson_mut_strncpy(doc_, text.data(), text.size()); }
    yyjson_mut_val* num(std::int64_t value) { return yyjson_mut_sint(doc_, value); }
    yyjson_mut_val* boolean(bool value) { return yyjson_mut_bool(doc_, value); }

    void put(yyjson_mut_val* object, std::string_view key, yyjson_mut_val* value) { yyjson_mut_obj_add(object, str(key), value); }
    void push(yyjson_mut_val* array, yyjson_mut_val* value) { yyjson_mut_arr_append(array, value); }

    void save(yyjson_mut_val* root, const std::string& path) {
        yyjson_mut_doc_set_root(doc_, root);
        yyjson_write_err err{};
        if (!yyjson_mut_write_file(path.c_str(), doc_, YYJSON_WRITE_PRETTY_TWO_SPACES | YYJSON_WRITE_NEWLINE_AT_END, nullptr, &err))
            throw std::runtime_error("cannot write " + path + ": " + (err.msg ? err.msg : "?"));
    }

private:
    yyjson_mut_doc* doc_;
};

} // namespace synergos::trust::test
