// LCT の JSON 読み取りで共通に使う小さな補助 (翻訳単位の内部用、公開しない)。
#pragma once
#include "synergos/trust/error.h"
#include <yyjson.h>
#include <cstdint>
#include <memory>
#include <string>
#include <string_view>

namespace synergos::trust::lct::detail {

using Doc = std::unique_ptr<yyjson_doc, decltype(&yyjson_doc_free)>;

[[noreturn]] inline void fail(Code code, const std::string& detail) { throw TrustError(code, detail); }

inline std::string_view sv(yyjson_val* v) { return {yyjson_get_str(v), yyjson_get_len(v)}; }

inline bool is_int(yyjson_val* v) { return yyjson_is_sint(v) || yyjson_is_uint(v); }
inline std::int64_t as_int(yyjson_val* v) {
    return yyjson_is_uint(v) ? static_cast<std::int64_t>(yyjson_get_uint(v)) : yyjson_get_sint(v);
}

} // namespace synergos::trust::lct::detail
