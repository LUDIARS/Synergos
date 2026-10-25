// @implements SPEC-SY-lct-v1
#include "caveats.h"
#include "json_read.h"
#include "synergos/trust/jcs.h"
#include "synergos/trust/lct.h"
#include <algorithm>
#include <array>

namespace synergos::trust::lct::detail {
namespace {

constexpr std::array<std::string_view, 15> string_caveats{
    "iss", "org", "project_ref", "sub", "session_id", "device_pub", "peer_id", "jti", "digest", "download_name", "prefix",
    "sub_peer_id", "node_id", "principal", "device_id"};
constexpr std::array<std::string_view, 3> roles{"read", "publish", "history"};

bool safe_name(std::string_view name) {
    if (name.empty() || name.size() > 255 || name == "." || name == "..") return false;
    return std::all_of(name.begin(), name.end(), [](char c) {
        return (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '.' || c == '_' || c == '-';
    });
}

bool storage_key(std::string_view key) {
    if (key.empty() || key.size() > 1024) return false;
    if (key.substr(0, 7) == "upload:" && key.size() == 7) return false;
    return std::all_of(key.begin(), key.end(), [](char c) { return c > 0x20 && c < 0x7f; });
}

std::int64_t non_negative(yyjson_val* v, std::string_view name) {
    if (!is_int(v) || as_int(v) < 0) fail(Code::lct_bad_caveat, std::string(name) + " must be a non-negative integer");
    return as_int(v);
}

template <class T>
bool subset(const std::set<T>& narrower, const std::set<T>& wider) {
    return std::includes(wider.begin(), wider.end(), narrower.begin(), narrower.end());
}

/// SPEC-SY-decode-bounds: 要素を読む前に要素数を上限で切る。
void items_bounded(yyjson_val* array, std::string_view name) {
    const auto size = yyjson_arr_size(array);
    if (size > max_caveat_items)
        fail(Code::decode_too_large, "trust.lct " + std::string(name) + ": " + std::to_string(size) + " > " + std::to_string(max_caveat_items));
}

} // namespace

Caveats parse_caveats(yyjson_val* obj) {
    if (!yyjson_is_obj(obj)) fail(Code::lct_malformed, "caveats must be an object");
    Caveats c;
    auto it = yyjson_obj_iter_with(obj);
    while (auto* key = yyjson_obj_iter_next(&it)) {
        const auto name = sv(key);
        auto* v = yyjson_obj_iter_get_val(key);
        if (std::find(string_caveats.begin(), string_caveats.end(), name) != string_caveats.end()) {
            if (!yyjson_is_str(v) || yyjson_get_len(v) == 0) fail(Code::lct_bad_caveat, std::string(name) + " must be a non-empty string");
            if (name == "download_name" && !safe_name(sv(v))) fail(Code::lct_bad_caveat, "download_name must be a safe ASCII basename");
            if (name == "prefix" && !storage_key(sv(v))) fail(Code::lct_bad_caveat, "prefix must be a storage key prefix");
            c.strings.emplace(std::string(name), std::string(sv(v)));
            if (name == "jti") c.chain_jti.emplace(sv(v));
        } else if (name == "nbf") c.nbf = non_negative(v, name);
        else if (name == "exp") c.exp = non_negative(v, name);
        else if (name == "roster_epoch") c.roster_epoch = non_negative(v, name);
        else if (name == "max_uses") c.max_uses = non_negative(v, name);
        else if (name == "max_bytes") c.max_bytes = non_negative(v, name);
        else if (name == "max_bytes_per_hour") c.max_bytes_per_hour = non_negative(v, name);
        else if (name == "ops" || name == "object_keys" || name == "orgs") {
            if (!yyjson_is_arr(v)) fail(Code::lct_bad_caveat, std::string(name) + " must be an array");
            items_bounded(v, name);
            std::set<std::string> items;
            auto ai = yyjson_arr_iter_with(v);
            while (auto* item = yyjson_arr_iter_next(&ai)) {
                if (!yyjson_is_str(item) || yyjson_get_len(item) == 0) fail(Code::lct_bad_caveat, std::string(name) + " entries must be strings");
                if (name == "object_keys" && !storage_key(sv(item))) fail(Code::lct_bad_caveat, "object_keys entry is not a storage key");
                items.emplace(sv(item));
            }
            (name == "ops" ? c.ops : name == "orgs" ? c.orgs : c.object_keys) = std::move(items);
        } else if (name == "projects") {
            if (!yyjson_is_arr(v)) fail(Code::lct_bad_caveat, "projects must be an array");
            items_bounded(v, name);
            std::set<std::pair<std::string, std::string>> items;
            auto ai = yyjson_arr_iter_with(v);
            while (auto* item = yyjson_arr_iter_next(&ai)) {
                auto* ref = yyjson_obj_get(item, "project_ref");
                auto* role = yyjson_obj_get(item, "role");
                if (!yyjson_is_obj(item) || yyjson_obj_size(item) != 2 || !yyjson_is_str(ref) || !yyjson_is_str(role)
                    || std::find(roles.begin(), roles.end(), sv(role)) == roles.end())
                    fail(Code::lct_bad_caveat, "projects entries are {project_ref, role}");
                items.emplace(std::string(sv(ref)), std::string(sv(role)));
            }
            c.projects = std::move(items);
        } else if (name == "byte_range") {
            if (!yyjson_is_arr(v) || yyjson_arr_size(v) != 2) fail(Code::lct_bad_caveat, "byte_range is [start, end]");
            const auto start = non_negative(yyjson_arr_get(v, 0), name);
            const auto end = non_negative(yyjson_arr_get(v, 1), name);
            if (start > end) fail(Code::lct_bad_caveat, "byte_range start exceeds end");
            c.byte_range = std::make_pair(start, end);
        } else {
            fail(Code::lct_unknown_caveat, "unknown caveat " + std::string(name));
        }
    }
    if (c.object_keys && c.strings.count("prefix")) fail(Code::lct_bad_caveat, "object_keys and prefix are exclusive");
    if (const auto org = c.strings.find("org"); org != c.strings.end() && c.orgs && !c.orgs->count(org->second))
        fail(Code::lct_bad_caveat, "org must be one of orgs");
    return c;
}

Caveats narrow(Caveats e, const Caveats& n) {
    const auto widened = [](const char* what) { fail(Code::lct_widened, what); };
    e.chain_jti.insert(n.chain_jti.begin(), n.chain_jti.end());
    for (const auto& [k, v] : n.strings) {
        if (k == "jti") { e.strings[k] = v; continue; } // jti は block ごとの名札 (失効は chain_jti の全部で見る)
        if (k == "prefix") {
            if (e.object_keys) widened("prefix cannot replace object_keys");
            const auto it = e.strings.find("prefix");
            if (it != e.strings.end() && v.rfind(it->second, 0) != 0) widened("prefix must extend the previous prefix");
            e.strings[k] = v;
            continue;
        }
        const auto it = e.strings.find(k);
        // sub はパスとして狭める: "curare:d1" → "curare:d1/<subject>" (delegation から capability へ)
        if (k == "sub" && it != e.strings.end() && v != it->second && v.rfind(it->second + "/", 0) != 0)
            widened("sub must extend the previous sub with /<child>");
        if (k != "sub" && it != e.strings.end() && it->second != v) widened("string caveat changed");
        e.strings[k] = v;
    }
    if (n.nbf) { if (e.nbf && *n.nbf < *e.nbf) widened("nbf moved earlier"); e.nbf = n.nbf; }
    // A roster epoch identifies the authority snapshot; attenuation cannot
    // replace it with a newer or older snapshot signed by the token holder.
    if (n.roster_epoch) {
        if (e.roster_epoch && *n.roster_epoch != *e.roster_epoch) widened("roster_epoch changed");
        e.roster_epoch = n.roster_epoch;
    }
    for (auto [mine, theirs] : {std::pair{&e.exp, &n.exp}, std::pair{&e.max_uses, &n.max_uses}, std::pair{&e.max_bytes, &n.max_bytes},
                                std::pair{&e.max_bytes_per_hour, &n.max_bytes_per_hour}}) {
        if (*theirs) { if (*mine && **theirs > **mine) widened("limit increased"); *mine = *theirs; }
    }
    if (n.ops) { if (e.ops && !subset(*n.ops, *e.ops)) widened("ops not a subset"); e.ops = n.ops; }
    if (n.orgs) { if (e.orgs && !subset(*n.orgs, *e.orgs)) widened("orgs not a subset"); e.orgs = n.orgs; }
    // org は orgs の 1 つに絞ることだけ許す (delegation の orgs[] から capability の org へ)
    if (const auto org = e.strings.find("org"); org != e.strings.end() && e.orgs && !e.orgs->count(org->second))
        widened("org outside orgs");
    if (n.projects) { if (e.projects && !subset(*n.projects, *e.projects)) widened("projects not a subset"); e.projects = n.projects; }
    if (n.object_keys) {
        if (e.object_keys && !subset(*n.object_keys, *e.object_keys)) widened("object_keys not a subset");
        if (const auto it = e.strings.find("prefix"); it != e.strings.end()) {
            for (const auto& key : *n.object_keys) if (key.rfind(it->second, 0) != 0) widened("object key outside prefix");
            e.strings.erase(it);
        }
        e.object_keys = n.object_keys;
    }
    if (n.byte_range) {
        if (e.byte_range && (n.byte_range->first < e.byte_range->first || n.byte_range->second > e.byte_range->second))
            widened("byte_range outside previous range");
        e.byte_range = n.byte_range;
    }
    return e;
}

std::string to_json(const Caveats& c) {
    std::string out = "{";
    bool first = true;
    const auto field = [&](std::string_view name) { if (!first) out += ','; first = false; out += jcs::quote(name); out += ':'; };
    for (const auto& [k, v] : c.strings) { field(k); out += jcs::quote(v); }
    for (auto [name, value] : {std::pair{"nbf", &c.nbf}, std::pair{"exp", &c.exp}, std::pair{"max_uses", &c.max_uses}, std::pair{"max_bytes", &c.max_bytes},
                               std::pair{"max_bytes_per_hour", &c.max_bytes_per_hour}, std::pair{"roster_epoch", &c.roster_epoch}})
        if (*value) { field(name); out += std::to_string(**value); }
    for (auto [name, value] : {std::pair{"ops", &c.ops}, std::pair{"object_keys", &c.object_keys}, std::pair{"orgs", &c.orgs}}) {
        if (!*value) continue;
        field(name);
        out += '[';
        bool f = true;
        for (const auto& item : **value) { if (!f) out += ','; f = false; out += jcs::quote(item); }
        out += ']';
    }
    if (c.projects) {
        field("projects");
        out += '[';
        bool f = true;
        for (const auto& [ref, role] : *c.projects) {
            if (!f) out += ',';
            f = false;
            out += "{\"project_ref\":" + jcs::quote(ref) + ",\"role\":" + jcs::quote(role) + "}";
        }
        out += ']';
    }
    if (c.byte_range) { field("byte_range"); out += '[' + std::to_string(c.byte_range->first) + ',' + std::to_string(c.byte_range->second) + ']'; }
    out += '}';
    return jcs::canonicalize(out);
}

Caveats parse_caveats_text(std::string_view caveats_json) {
    const auto canonical = jcs::canonicalize(caveats_json);
    Doc doc(yyjson_read(canonical.data(), canonical.size(), 0), yyjson_doc_free);
    return parse_caveats(yyjson_doc_get_root(doc.get()));
}

} // namespace synergos::trust::lct::detail
