// @implements SPEC-SY-custody-service-auth SPEC-SY-custody-capability-v2
#include "profile_rules.h"
#include "json_read.h"
#include <algorithm>
#include <array>
#include <initializer_list>
#include <string_view>

namespace synergos::trust::lct::detail {
namespace {

constexpr std::array<std::string_view, 7> delegation_ops{"admit", "commit", "get", "head", "put", "reclaim_request", "verify"};

[[noreturn]] void broken(const std::string& rule) { fail(Code::lct_profile_rule, rule); }

bool starts_with(const std::string& text, std::string_view head) {
    return text.size() > head.size() && text.compare(0, head.size(), head) == 0;
}

const std::string* text(const Caveats& c, const char* name) {
    const auto it = c.strings.find(name);
    return it == c.strings.end() ? nullptr : &it->second;
}

void require(const Caveats& c, std::initializer_list<const char*> names) {
    for (const auto* name : names)
        if (!text(c, name)) broken(std::string(name) + " is required");
}

void check_delegation(const Caveats& c) {
    require(c, {"iss", "sub", "jti"});
    if (!starts_with(*text(c, "iss"), "custody:")) broken("iss must be custody:<instance>");
    if (!starts_with(*text(c, "sub"), "curare:")) broken("sub must be curare:<deployment>");
    if (!c.orgs || c.orgs->empty()) broken("orgs is required");
    if (!c.ops || c.ops->empty()) broken("ops is required");
    for (const auto& op : *c.ops)
        if (std::find(delegation_ops.begin(), delegation_ops.end(), op) == delegation_ops.end()) broken("unknown delegation op " + op);
    if (!c.max_bytes_per_hour) broken("max_bytes_per_hour is required");
}

void check_capability(const Caveats& c) {
    require(c, {"org", "project_ref", "jti", "sub", "session_id"});
    const bool get = c.ops == std::set<std::string>{"get"};
    const bool put = c.ops == std::set<std::string>{"put"};
    if (!get && !put) broken("ops must be exactly [get] or [put]");
    if (put) {
        if (!c.object_keys || c.object_keys->empty()) broken("PUT names upload:<upload_id> object keys");
        for (const auto& key : *c.object_keys)
            if (!starts_with(key, "upload:")) broken("PUT object keys must be upload:<upload_id>");
        if (!c.max_bytes) broken("PUT requires max_bytes");
        if (c.byte_range) broken("byte_range is valid only for GET");
    } else if (!c.object_keys && !c.strings.count("prefix")) {
        broken("GET names object_keys or prefix");
    }
}

// @implements SPEC-SY-membership-token
void check_membership(const Caveats& c) {
    require(c, {"iss", "sub_peer_id", "node_id", "principal", "device_id", "org", "jti"});
    if (*text(c, "iss") != "cernere") broken("membership iss must be cernere");
    if (!c.roster_epoch) broken("membership requires roster_epoch");
    if (!c.projects || c.projects->empty()) broken("membership requires explicit projects");
    for (const auto& [project, role] : *c.projects) {
        (void)role;
        if (project.empty() || project == "*") broken("membership project must be explicit");
    }
}

} // namespace

bool presentable(Profile issued, Profile presented) noexcept {
    return issued == presented || (issued == Profile::custody_delegation && presented == Profile::custody_capability);
}

void check_issued(Profile issued, const Caveats& head) {
    if (issued == Profile::custody_delegation) check_delegation(head);
    if (issued == Profile::membership) check_membership(head);
}

void check_presented(Profile presented, const Caveats& effective) {
    switch (presented) {
    case Profile::membership: check_membership(effective); break;
    case Profile::custody_delegation: check_delegation(effective); break;
    case Profile::custody_capability: check_capability(effective); break;
    }
}

} // namespace synergos::trust::lct::detail
