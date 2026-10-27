// @implements SPEC-SY-native-custody-request-boundary SPEC-SY-custody-capability-v2
#include "synergos/custody/capability_gate.h"
#include "synergos/trust/error.h"
#include <yyjson.h>
#include <algorithm>
#include <memory>
#include <string_view>

namespace synergos::custody {
namespace {
using Doc = std::unique_ptr<yyjson_doc, decltype(&yyjson_doc_free)>;

[[noreturn]] void deny(CapabilityFailure failure) { throw CapabilityError(failure); }

bool valid_window(std::int64_t issued, std::int64_t expires, std::int64_t now) noexcept {
    // Subtraction only after nonnegative ordering: no signed overflow at int64 extremes.
    return issued >= 0 && issued <= now && expires >= issued && expires - issued == 300 && now < expires;
}

std::optional<std::string> text(yyjson_val* object, const char* name) {
    auto* value = yyjson_obj_get(object, name);
    if (!value) return std::nullopt;
    if (!yyjson_is_str(value)) deny(CapabilityFailure::invalid);
    return std::string(yyjson_get_str(value), yyjson_get_len(value));
}

std::string required(yyjson_val* object, const char* name) {
    auto value = text(object, name);
    if (!value || value->empty()) deny(CapabilityFailure::invalid);
    return *value;
}

std::optional<std::uint64_t> number(yyjson_val* object, const char* name) {
    auto* value = yyjson_obj_get(object, name);
    if (!value) return std::nullopt;
    if (!yyjson_is_int(value) || yyjson_get_sint(value) < 0) deny(CapabilityFailure::invalid);
    return static_cast<std::uint64_t>(yyjson_get_sint(value));
}

bool storage_key(const std::string& key) noexcept {
    return !key.empty() && key.size() <= 1024 && key != "upload:"
        && std::all_of(key.begin(), key.end(), [](unsigned char c) { return c > 0x20 && c < 0x7f; });
}

bool object_matches(yyjson_val* caveats, const std::string& key) {
    if (auto* keys = yyjson_obj_get(caveats, "object_keys")) {
        if (!yyjson_is_arr(keys)) deny(CapabilityFailure::invalid);
        auto iter = yyjson_arr_iter_with(keys);
        while (auto* item = yyjson_arr_iter_next(&iter)) {
            if (!yyjson_is_str(item)) deny(CapabilityFailure::invalid);
            if (std::string_view(yyjson_get_str(item), yyjson_get_len(item)) == key) return true;
        }
        return false;
    }
    const auto prefix = text(caveats, "prefix");
    return prefix && key.compare(0, prefix->size(), *prefix) == 0;
}

std::uint64_t get_bytes(yyjson_val* caveats, const CapabilityRequest& request) {
    if (!request.stored_object_bytes) deny(CapabilityFailure::out_of_scope);
    const auto size = *request.stored_object_bytes;
    auto* range = yyjson_obj_get(caveats, "byte_range");
    if (range && !request.range) deny(CapabilityFailure::range_denied);
    if (!request.range) return size;
    const auto asked = *request.range;
    if (asked.first > asked.last || asked.last >= size) deny(CapabilityFailure::range_denied);
    if (range) {
        if (!yyjson_is_arr(range) || yyjson_arr_size(range) != 2) deny(CapabilityFailure::invalid);
        const auto first = yyjson_get_sint(yyjson_arr_get(range, 0));
        const auto last = yyjson_get_sint(yyjson_arr_get(range, 1));
        if (first < 0 || last < first || asked.first < static_cast<std::uint64_t>(first)
            || asked.last > static_cast<std::uint64_t>(last)) deny(CapabilityFailure::range_denied);
    }
    // last < stored size <= uint64 max, so inclusive length cannot overflow.
    return asked.last - asked.first + 1;
}
} // namespace

CapabilityError::CapabilityError(CapabilityFailure failure)
    : std::runtime_error(failure == CapabilityFailure::revocation_stale ? "revocation_stale" : "custody_capability_denied"), failure_(failure) {}

RedemptionConstraints check_capability_request(const std::string& token,
    const std::map<std::string, trust::lct::IssuerKey>& issuers,
    const std::optional<VerifiedRevocationState>& revocation,
    std::int64_t now, const CapabilityRequest& request) {
    if (!revocation || now < 0 || !valid_window(revocation->local_issued_at, revocation->local_expires_at, now)
        || !valid_window(revocation->roster_issued_at, revocation->roster_expires_at, now)) deny(CapabilityFailure::revocation_stale);
    const auto& state = *revocation;
    if (request.org.empty() || request.project_ref.empty() || !storage_key(request.object_key)
        || state.org != request.org || state.organization_frozen) deny(CapabilityFailure::out_of_scope);
    if (request.operation != CapabilityOperation::get && request.operation != CapabilityOperation::put) deny(CapabilityFailure::invalid);
    const bool put = request.operation == CapabilityOperation::put;
    if ((put && (request.url_form || request.range))
        || (put && state.read_only_projects.count(request.project_ref))) deny(CapabilityFailure::out_of_scope);

    trust::lct::Verified verified;
    try {
        verified = trust::lct::verify(token, issuers, state.token_revocations, now,
            {trust::lct::Profile::custody_capability, request.url_form, true});
    } catch (const trust::TrustError&) { deny(CapabilityFailure::invalid); }
    Doc doc(yyjson_read(verified.caveats_json.data(), verified.caveats_json.size(), 0), yyjson_doc_free);
    auto* caveats = doc ? yyjson_doc_get_root(doc.get()) : nullptr;
    if (!yyjson_is_obj(caveats)) deny(CapabilityFailure::invalid);
    if (required(caveats, "org") != request.org || required(caveats, "project_ref") != request.project_ref
        || !object_matches(caveats, request.object_key)) deny(CapabilityFailure::out_of_scope);
    auto* ops = yyjson_obj_get(caveats, "ops");
    auto* op = yyjson_is_arr(ops) && yyjson_arr_size(ops) == 1 ? yyjson_arr_get(ops, 0) : nullptr;
    if (!yyjson_is_str(op) || std::string_view(yyjson_get_str(op), yyjson_get_len(op)) != (put ? "put" : "get")) deny(CapabilityFailure::out_of_scope);
    const auto subject = required(caveats, "sub");
    const auto session = required(caveats, "session_id");
    if ((request.expected_subject && *request.expected_subject != subject)
        || (request.expected_session_id && *request.expected_session_id != session)
        || state.denied_subjects.count(subject) || state.denied_sessions.count(session)) deny(CapabilityFailure::out_of_scope);
    const auto digest = text(caveats, "digest");
    if (!put && digest && (!request.stored_digest || *digest != *request.stored_digest)) deny(CapabilityFailure::out_of_scope);
    const auto maximum = number(caveats, "max_bytes");
    std::uint64_t ceiling;
    if (put) {
        if (!maximum || !request.stored_upload_bytes) deny(CapabilityFailure::out_of_scope);
        if (*request.stored_upload_bytes > *maximum) deny(CapabilityFailure::byte_limit);
        ceiling = *maximum - *request.stored_upload_bytes;
        if (request.declared_bytes) {
            if (*request.declared_bytes > ceiling) deny(CapabilityFailure::byte_limit);
            ceiling = *request.declared_bytes;
        }
    } else {
        ceiling = get_bytes(caveats, request);
        if (maximum && ceiling > *maximum) deny(CapabilityFailure::byte_limit);
    }
    return {subject, session, required(caveats, "jti"), verified.delegation_jti, digest,
        put ? std::nullopt : text(caveats, "download_name"), ceiling,
        number(caveats, "max_uses"), number(caveats, "max_bytes_per_hour")};
}
} // namespace synergos::custody
