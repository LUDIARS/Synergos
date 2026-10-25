// LCT の caveat: 閉じた語彙の読み取り・積集合 (狭めるだけ)・JCS 正規形への書き出し (翻訳単位の内部用)。
#pragma once
#include <yyjson.h>
#include <cstdint>
#include <map>
#include <optional>
#include <set>
#include <string>
#include <string_view>
#include <utility>

namespace synergos::trust::lct::detail {

struct Caveats {
    std::map<std::string, std::string> strings;
    std::optional<std::int64_t> nbf, exp, max_uses, max_bytes, max_bytes_per_hour;
    std::optional<std::int64_t> roster_epoch;
    std::optional<std::set<std::string>> ops, orgs;
    std::optional<std::set<std::pair<std::string, std::string>>> projects;
    std::optional<std::set<std::string>> object_keys;
    std::optional<std::pair<std::int64_t, std::int64_t>> byte_range;
    /// 鎖のすべての block の jti。どれか 1 つが失効していれば token 全体が失効 (to_json には出さない)。
    std::set<std::string> chain_jti;
};

/// 閉じた語彙で読む。未知の名前は lct.unknown_caveat、値の形が違えば lct.bad_caveat。
Caveats parse_caveats(yyjson_val* obj);
Caveats parse_caveats_text(std::string_view caveats_json);

/// e (これまでの積集合) に n (新しい block) を重ねる。広げようとしたら lct.widened。
Caveats narrow(Caveats e, const Caveats& n);

/// JCS 正規形の JSON。
std::string to_json(const Caveats& c);

} // namespace synergos::trust::lct::detail
