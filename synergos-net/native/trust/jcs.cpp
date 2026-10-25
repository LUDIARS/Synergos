// @implements SPEC-SY-signed-envelope
// @decoder trust.jcs
#include "synergos/trust/jcs.h"
#include "synergos/trust/error.h"
#include <yyjson.h>
#include <algorithm>
#include <memory>
#include <vector>

namespace synergos::trust::jcs {
namespace {

using Doc = std::unique_ptr<yyjson_doc, decltype(&yyjson_doc_free)>;

/// UTF-8 (yyjson が検証済み) を UTF-16 コード単位列に直す。JCS はキーを UTF-16 単位で並べる。
std::vector<std::uint16_t> utf16(std::string_view s) {
    std::vector<std::uint16_t> out;
    out.reserve(s.size());
    for (std::size_t i = 0; i < s.size();) {
        const auto b0 = static_cast<unsigned char>(s[i]);
        std::uint32_t cp{};
        std::size_t n{};
        if (b0 < 0x80) { cp = b0; n = 1; }
        else if ((b0 >> 5) == 0x6) { cp = b0 & 0x1f; n = 2; }
        else if ((b0 >> 4) == 0xe) { cp = b0 & 0x0f; n = 3; }
        else { cp = b0 & 0x07; n = 4; }
        if (i + n > s.size()) throw TrustError(Code::jcs_invalid_json, "truncated UTF-8");
        for (std::size_t k = 1; k < n; ++k) cp = (cp << 6) | (static_cast<unsigned char>(s[i + k]) & 0x3f);
        i += n;
        if (cp >= 0x10000) {
            cp -= 0x10000;
            out.push_back(static_cast<std::uint16_t>(0xd800 + (cp >> 10)));
            out.push_back(static_cast<std::uint16_t>(0xdc00 + (cp & 0x3ff)));
        } else {
            out.push_back(static_cast<std::uint16_t>(cp));
        }
    }
    return out;
}

void append_quoted(std::string_view s, std::string& out) {
    constexpr char hex[] = "0123456789abcdef";
    out += '"';
    for (const char ch : s) {
        const auto c = static_cast<unsigned char>(ch);
        switch (c) {
        case '"': out += "\\\""; break;
        case '\\': out += "\\\\"; break;
        case 0x08: out += "\\b"; break;
        case 0x09: out += "\\t"; break;
        case 0x0a: out += "\\n"; break;
        case 0x0c: out += "\\f"; break;
        case 0x0d: out += "\\r"; break;
        default:
            if (c < 0x20) {
                out += "\\u00";
                out += hex[c >> 4];
                out += hex[c & 15];
            } else {
                out += ch;
            }
        }
    }
    out += '"';
}

std::string_view str(yyjson_val* v) { return {yyjson_get_str(v), yyjson_get_len(v)}; }

void append(yyjson_val* v, std::string& out, unsigned depth) {
    // depth は 0 始まりなので段数は depth + 1。
    if (depth >= max_depth) throw TrustError(Code::jcs_too_deep, "nesting exceeds 128");
    if (yyjson_is_obj(v)) {
        struct Entry { yyjson_val* key; std::vector<std::uint16_t> order; };
        std::vector<Entry> entries;
        auto it = yyjson_obj_iter_with(v);
        while (auto* key = yyjson_obj_iter_next(&it)) entries.push_back({key, utf16(str(key))});
        std::sort(entries.begin(), entries.end(), [](const Entry& a, const Entry& b) { return a.order < b.order; });
        out += '{';
        for (std::size_t i = 0; i < entries.size(); ++i) {
            if (i && entries[i - 1].order == entries[i].order) throw TrustError(Code::jcs_duplicate_key, "duplicate object key");
            if (i) out += ',';
            append_quoted(str(entries[i].key), out);
            out += ':';
            append(yyjson_obj_iter_get_val(entries[i].key), out, depth + 1);
        }
        out += '}';
    } else if (yyjson_is_arr(v)) {
        out += '[';
        auto it = yyjson_arr_iter_with(v);
        bool first = true;
        while (auto* item = yyjson_arr_iter_next(&it)) {
            if (!first) out += ',';
            first = false;
            append(item, out, depth + 1);
        }
        out += ']';
    } else if (yyjson_is_str(v)) {
        append_quoted(str(v), out);
    } else if (yyjson_is_real(v)) {
        throw TrustError(Code::jcs_float, "floating point numbers are not allowed");
    } else if (yyjson_is_uint(v)) {
        const auto n = yyjson_get_uint(v);
        if (n > static_cast<std::uint64_t>(max_safe_integer)) throw TrustError(Code::jcs_unsafe_integer, "integer exceeds 2^53-1");
        out += std::to_string(n);
    } else if (yyjson_is_sint(v)) {
        const auto n = yyjson_get_sint(v);
        if (n < -max_safe_integer || n > max_safe_integer) throw TrustError(Code::jcs_unsafe_integer, "integer exceeds 2^53-1");
        out += std::to_string(n);
    } else if (yyjson_is_bool(v)) {
        out += yyjson_get_bool(v) ? "true" : "false";
    } else if (yyjson_is_null(v)) {
        out += "null";
    } else {
        throw TrustError(Code::jcs_invalid_json, "unsupported JSON value");
    }
}

} // namespace

std::string canonicalize(std::string_view json) {
    if (json.size() > max_bytes) throw TrustError(Code::jcs_too_large, "payload exceeds 1 MiB");
    Doc doc(yyjson_read(json.data(), json.size(), 0), yyjson_doc_free);
    if (!doc) throw TrustError(Code::jcs_invalid_json, "invalid JSON or UTF-8");
    std::string out;
    out.reserve(json.size());
    append(yyjson_doc_get_root(doc.get()), out, 0);
    return out;
}

std::string quote(std::string_view utf8) {
    // 不正な UTF-8 を通さないよう、yyjson の検証を経由する。
    std::string probe;
    append_quoted(utf8, probe);
    Doc doc(yyjson_read(probe.data(), probe.size(), 0), yyjson_doc_free);
    if (!doc) throw TrustError(Code::jcs_invalid_json, "invalid UTF-8 string");
    return probe;
}

} // namespace synergos::trust::jcs
