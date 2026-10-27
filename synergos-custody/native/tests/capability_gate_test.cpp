// @implements SPEC-SY-native-custody-request-boundary
#include "synergos/custody/capability_gate.h"
#include "synergos/trust/lct.h"
#include <cstdint>
#include <limits>
#include <stdexcept>
#include <string>

using namespace synergos;
namespace {
constexpr std::int64_t now = 1800000010;
void require(bool value, const char* message) { if (!value) throw std::runtime_error(message); }
trust::Seed seed(std::uint8_t value) { trust::Seed result; result.fill(value); return result; }
template<class F> void denied(custody::CapabilityFailure failure, F action) {
    try { action(); } catch (const custody::CapabilityError& error) {
        require(error.failure() == failure, "unexpected custody denial");
        require(error.status() == (failure == custody::CapabilityFailure::revocation_stale ? 503 : 403), "denial status");
        return;
    }
    throw std::runtime_error("request unexpectedly passed");
}
custody::VerifiedRevocationState fresh() {
    custody::VerifiedRevocationState state;
    state.org = "o";
    state.local_issued_at = state.roster_issued_at = now - 10;
    state.local_expires_at = state.roster_expires_at = now + 290;
    return state;
}
std::string capability(const trust::SigningKey& issuer, const char* fields) {
    const std::string body = std::string(R"({"exp":1800000200,"jti":"cap","org":"o","project_ref":"p","session_id":"session","sub":"user",)") + fields + "}";
    return trust::lct::mint("issuer", issuer, trust::lct::Profile::custody_capability, now - 10, body, seed(0x22)).token;
}
custody::CapabilityRequest get_request() {
    custody::CapabilityRequest request;
    request.operation = custody::CapabilityOperation::get;
    request.org = "o"; request.project_ref = "p"; request.object_key = "p/a";
    request.stored_object_bytes = 100;
    request.expected_subject = "user"; request.expected_session_id = "session";
    return request;
}
} // namespace

int main() {
    const trust::SigningKey issuer(seed(0x11));
    const std::map<std::string, trust::lct::IssuerKey> issuers{{"issuer", {issuer.public_key(), trust::lct::Profile::custody_capability}}};
    const auto unsealed = capability(issuer, R"("ops":["get"],"prefix":"p/","digest":"sha256:a","download_name":"a.bin")");
    const auto token = trust::lct::seal(unsealed);
    auto request = get_request();
    request.stored_digest = "sha256:a";
    const auto accepted = custody::check_capability_request(token, issuers, fresh(), now, request);
    require(accepted.request_byte_ceiling == 100 && accepted.download_name == "a.bin", "get constraints");
    require(accepted.subject == "user" && accepted.jti == "cap", "owned claims");
    denied(custody::CapabilityFailure::invalid, [&] { custody::check_capability_request(unsealed, issuers, fresh(), now, request); });
    denied(custody::CapabilityFailure::revocation_stale, [&] { custody::check_capability_request(token, issuers, std::nullopt, now, request); });
    auto state = fresh(); state.roster_expires_at = now;
    denied(custody::CapabilityFailure::revocation_stale, [&] { custody::check_capability_request(token, issuers, state, now, request); });
    state = fresh(); state.local_issued_at = std::numeric_limits<std::int64_t>::max(); state.local_expires_at = std::numeric_limits<std::int64_t>::min();
    denied(custody::CapabilityFailure::revocation_stale, [&] { custody::check_capability_request(token, issuers, state, now, request); });
    state = fresh(); state.denied_sessions.insert("session");
    denied(custody::CapabilityFailure::out_of_scope, [&] { custody::check_capability_request(token, issuers, state, now, request); });
    state = fresh(); state.token_revocations.revoked_jti.insert("cap");
    denied(custody::CapabilityFailure::invalid, [&] { custody::check_capability_request(token, issuers, state, now, request); });
    for (int field = 0; field < 4; ++field) {
        auto changed = request;
        if (field == 0) changed.org = "other";
        if (field == 1) changed.project_ref = "other";
        if (field == 2) changed.object_key = "else/a";
        if (field == 3) changed.stored_digest = "sha256:wrong";
        denied(custody::CapabilityFailure::out_of_scope, [&] { custody::check_capability_request(token, issuers, fresh(), now, changed); });
    }
    const auto ranged = trust::lct::seal(capability(issuer, R"("ops":["get"],"object_keys":["p/a"],"byte_range":[10,19],"max_bytes":10)"));
    auto range_request = get_request(); range_request.range = custody::ByteRange{10, 19};
    require(custody::check_capability_request(ranged, issuers, fresh(), now, range_request).request_byte_ceiling == 10, "inclusive range");
    range_request.range.reset();
    denied(custody::CapabilityFailure::range_denied, [&] { custody::check_capability_request(ranged, issuers, fresh(), now, range_request); });
    range_request.range = custody::ByteRange{10, std::numeric_limits<std::uint64_t>::max()};
    denied(custody::CapabilityFailure::range_denied, [&] { custody::check_capability_request(ranged, issuers, fresh(), now, range_request); });

    const auto put = trust::lct::seal(capability(issuer, R"("ops":["put"],"object_keys":["upload:u"],"max_bytes":100)"));
    auto upload = get_request(); upload.operation = custody::CapabilityOperation::put; upload.object_key = "upload:u";
    upload.stored_upload_bytes = 90; upload.declared_bytes = 10;
    require(custody::check_capability_request(put, issuers, fresh(), now, upload).request_byte_ceiling == 10, "remaining upload ceiling");
    upload.declared_bytes = 11;
    denied(custody::CapabilityFailure::byte_limit, [&] { custody::check_capability_request(put, issuers, fresh(), now, upload); });
    upload.declared_bytes = std::numeric_limits<std::uint64_t>::max();
    denied(custody::CapabilityFailure::byte_limit, [&] { custody::check_capability_request(put, issuers, fresh(), now, upload); });
    upload.declared_bytes = 10; state = fresh(); state.read_only_projects.insert("p");
    denied(custody::CapabilityFailure::out_of_scope, [&] { custody::check_capability_request(put, issuers, state, now, upload); });
    require(custody::check_capability_request(token, issuers, state, now, request).request_byte_ceiling == 100, "read-only permits get");

    const auto delegation = trust::lct::mint("delegation", issuer, trust::lct::Profile::custody_delegation, now - 10,
        R"({"exp":1800003000,"iss":"custody:one","sub":"curare:one","jti":"root","orgs":["o"],"ops":["get"],"max_bytes_per_hour":1000})", seed(0x23));
    const auto child = trust::lct::attenuate(delegation.token,
        R"({"exp":1800000200,"org":"o","project_ref":"p","jti":"child","sub":"curare:one/user","session_id":"session","object_keys":["p/a"]})", seed(0x24));
    const auto sealed_child = trust::lct::seal(child.token);
    const std::map<std::string, trust::lct::IssuerKey> delegation_issuers{{"delegation", {issuer.public_key(), trust::lct::Profile::custody_delegation}}};
    auto child_request = get_request(); child_request.expected_subject = "curare:one/user";
    const auto child_constraints = custody::check_capability_request(sealed_child, delegation_issuers, fresh(), now, child_request);
    require(child_constraints.delegation_jti == "root" && child_constraints.jti == "child", "root budget identity");
    return 0;
}
