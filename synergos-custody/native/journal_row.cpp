// @implements SPEC-SY-native-custody-journal
// @decoder custody.journal
#include "journal_row.hpp"
#include "digest.hpp"
#include "synergos/trust/jcs.h"
#include <yyjson.h>
#include <algorithm>
#include <cstdlib>
#include <initializer_list>
#include <memory>
#include <stdexcept>

namespace synergos::custody::detail {
JournalRecordError::JournalRecordError() : std::runtime_error("Invalid custody journal record") {}
namespace {
using Doc = std::unique_ptr<yyjson_doc, decltype(&yyjson_doc_free)>;
[[noreturn]] void invalid() { throw JournalRecordError(); }
Doc parse(std::string_view text) {
    Doc doc(yyjson_read(text.data(), text.size(), 0), yyjson_doc_free);
    if (!doc || !yyjson_is_obj(yyjson_doc_get_root(doc.get()))) invalid();
    return doc;
}
std::string_view text(yyjson_val* value) {
    if (!yyjson_is_str(value)) invalid();
    return {yyjson_get_str(value), yyjson_get_len(value)};
}
bool one_of(std::string_view value, std::initializer_list<std::string_view> choices) {
    return std::find(choices.begin(), choices.end(), value) != choices.end();
}
void fields(yyjson_val* value, std::initializer_list<std::string_view> allowed) {
    if (!yyjson_is_obj(value)) invalid();
    yyjson_obj_iter it = yyjson_obj_iter_with(value);
    while (auto* key = yyjson_obj_iter_next(&it)) if (!one_of(text(key), allowed)) invalid();
}
std::string serialize(yyjson_val* value) {
    std::size_t count{};
    std::unique_ptr<char, decltype(&std::free)> bytes(yyjson_val_write(value, 0, &count), std::free);
    if (!bytes) throw std::bad_alloc();
    return trust::jcs::canonicalize({bytes.get(), count});
}
std::uint64_t seq(yyjson_val* object) {
    auto* value = yyjson_obj_get(object, "seq");
    if (!yyjson_is_uint(value) || yyjson_get_uint(value) == 0 ||
        yyjson_get_uint(value) > static_cast<std::uint64_t>(trust::jcs::max_safe_integer)) invalid();
    return yyjson_get_uint(value);
}
unsigned digit(char value) {
    if (value >= '0' && value <= '9') return static_cast<unsigned>(value - '0');
    if (value >= 'a' && value <= 'f') return static_cast<unsigned>(value - 'a' + 10);
    invalid();
}
std::string raw_hash(std::string_view hash) {
    if (hash.size() != 64) invalid();
    std::string bytes(32, '\0');
    for (std::size_t i = 0; i < 32; ++i) bytes[i] = static_cast<char>((digit(hash[2*i]) << 4) | digit(hash[2*i+1]));
    return bytes;
}
std::string hash(std::string_view previous, std::string_view row) {
    auto bytes = raw_hash(previous);
    bytes.append(row);
    const auto digest = sha256(bytes);
    constexpr std::string_view hex = "0123456789abcdef";
    std::string result;
    result.reserve(64);
    for (const auto byte : digest) { result += hex[byte >> 4]; result += hex[byte & 15]; }
    return result;
}
void ids(yyjson_val* object, std::initializer_list<const char*> keys) {
    for (const auto* key : keys) if (auto* value = yyjson_obj_get(object, key)) journal_identifier(text(value));
}
}
void journal_identifier(std::string_view value) {
    if (value.empty() || value.size() > 256 || !std::all_of(value.begin(), value.end(), [](unsigned char c) {
        return c >= 0x21 && c <= 0x7e;
    })) invalid();
}
std::string journal_event(std::string_view input) {
    if (input.size() > journal_row_bytes / 2) invalid();
    const auto canonical = trust::jcs::canonicalize(input);
    const auto doc = parse(canonical);
    auto* root = yyjson_doc_get_root(doc.get());
    fields(root, {"v", "ts", "event_type", "actor", "subject", "change", "outcome", "reason_code", "correlation"});
    auto* version = yyjson_obj_get(root, "v");
    auto* ts = yyjson_obj_get(root, "ts");
    if (!yyjson_is_uint(version) || yyjson_get_uint(version) != 1 || !yyjson_is_uint(ts)) invalid();
    if (!one_of(text(yyjson_obj_get(root, "event_type")),
        {"custody.admit", "custody.serve", "custody.redeem", "custody.deny", "custody.reclaim", "custody.purge", "custody.scrub"})) invalid();
    if (!one_of(text(yyjson_obj_get(root, "outcome")), {"success", "denied", "failure"})) invalid();
    journal_identifier(text(yyjson_obj_get(root, "reason_code")));
    auto* actor = yyjson_obj_get(root, "actor");
    fields(actor, {"principal_kind", "user_id", "service_principal", "node_id", "peer_id", "link_id", "session_id", "device_id", "client_kind"});
    const auto kind = text(yyjson_obj_get(actor, "principal_kind"));
    if (!one_of(kind, {"user", "service_principal", "node", "peer", "link"})) invalid();
    const auto* actor_key = kind == "user" ? "user_id" : kind == "node" ? "node_id" :
        kind == "peer" ? "peer_id" : kind == "link" ? "link_id" : "service_principal";
    journal_identifier(text(yyjson_obj_get(actor, actor_key)));
    ids(actor, {"user_id", "service_principal", "node_id", "peer_id", "link_id", "session_id", "device_id", "client_kind"});
    auto* subject = yyjson_obj_get(root, "subject");
    fields(subject, {"org", "project_ref", "space", "logical_asset", "version_ids", "object_keys"});
    journal_identifier(text(yyjson_obj_get(subject, "org")));
    ids(subject, {"project_ref", "space", "logical_asset"});
    for (const auto* key : {"version_ids", "object_keys"}) if (auto* list = yyjson_obj_get(subject, key)) {
        if (!yyjson_is_arr(list) || yyjson_arr_size(list) > 128) invalid();
        for (std::size_t i = 0; i < yyjson_arr_size(list); ++i) journal_identifier(text(yyjson_arr_get(list, i)));
    }
    auto* change = yyjson_obj_get(root, "change");
    fields(change, {"before_hash", "after_hash"}); // No arbitrary diff/body in this custody metadata writer.
    for (const auto* key : {"before_hash", "after_hash"}) if (auto* value = yyjson_obj_get(change, key)) raw_hash(text(value));
    auto* correlation = yyjson_obj_get(root, "correlation");
    fields(correlation, {"trace_id", "cap_id", "operation_id", "client_request_id"});
    const auto trace = text(yyjson_obj_get(correlation, "trace_id"));
    if (trace.size() != 32 || std::all_of(trace.begin(), trace.end(), [](char c) {return c == '0';})) invalid();
    for (const auto c : trace) digit(c);
    ids(correlation, {"cap_id", "operation_id", "client_request_id"});
    return canonical;
}
std::string journal_row(std::string_view stream, const JournalHead& previous, std::string_view event) {
    journal_identifier(stream);
    if (previous.seq >= static_cast<std::uint64_t>(trust::jcs::max_safe_integer)) invalid();
    const auto row = trust::jcs::canonicalize("{\"event\":" + std::string(event) + ",\"prev_hash\":" +
        trust::jcs::quote(previous.root_hash) + ",\"seq\":" + std::to_string(previous.seq+1) +
        ",\"stream\":" + trust::jcs::quote(stream) + "}");
    auto result = row;
    result.pop_back();
    result += ",\"row_hash\":" + trust::jcs::quote(hash(previous.root_hash, row)) + "}";
    result = trust::jcs::canonicalize(result);
    if (result.size() >= journal_row_bytes) invalid();
    return result;
}
JournalHead journal_row_head(std::string_view row) {
    const auto doc = parse(row);
    auto* root = yyjson_doc_get_root(doc.get());
    const auto root_hash = text(yyjson_obj_get(root, "row_hash"));
    raw_hash(root_hash);
    return {seq(root), std::string(root_hash)};
}
JournalHead verify_journal_row(std::string_view stream, const JournalHead& previous, std::string_view row) {
    if (row.size() >= journal_row_bytes || trust::jcs::canonicalize(row) != row) invalid();
    const auto doc = parse(row);
    auto* root = yyjson_doc_get_root(doc.get());
    fields(root, {"event", "prev_hash", "seq", "stream", "row_hash"});
    auto* event = yyjson_obj_get(root, "event");
    if (!event || journal_row(stream, previous, journal_event(serialize(event))) != row) invalid();
    return journal_row_head(row);
}
}
