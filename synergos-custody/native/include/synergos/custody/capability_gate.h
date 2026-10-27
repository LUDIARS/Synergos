// @implements SPEC-SY-native-custody-request-boundary
#pragma once
#include "synergos/trust/lct.h"
#include <cstdint>
#include <map>
#include <optional>
#include <set>
#include <stdexcept>
#include <string>

namespace synergos::custody {

enum class CapabilityOperation { unspecified, get, put };
struct ByteRange { std::uint64_t first = 0, last = 0; }; // Inclusive GET range.

/// Scope/object state comes from the admission/object owner, not self-reported headers.
struct CapabilityRequest {
    CapabilityOperation operation = CapabilityOperation::unspecified;
    std::string org, project_ref, object_key;
    std::optional<std::uint64_t> stored_object_bytes, stored_upload_bytes, declared_bytes;
    std::optional<ByteRange> range;
    std::optional<std::string> stored_digest, expected_subject, expected_session_id;
    bool url_form = false;
};

/// Only the future signature/sequence-checked cache adapter may supply this.
/// This type does not itself validate either signed part or fetch Cernere data.
struct VerifiedRevocationState {
    std::string org;
    std::int64_t local_issued_at = 0, local_expires_at = 0;
    std::int64_t roster_issued_at = 0, roster_expires_at = 0;
    trust::lct::RevocationView token_revocations;
    std::set<std::string> denied_subjects, denied_sessions, read_only_projects;
    bool organization_frozen = false;
};

enum class CapabilityFailure { invalid, out_of_scope, revocation_stale, range_denied, byte_limit };
class CapabilityError : public std::runtime_error {
public:
    explicit CapabilityError(CapabilityFailure failure);
    CapabilityFailure failure() const noexcept { return failure_; }
    int status() const noexcept { return failure_ == CapabilityFailure::revocation_stale ? 503 : 403; }
private:
    CapabilityFailure failure_;
};

/// Checked constraints only. A durable owner must reserve/consume budgets and
/// journal the redemption before reading bytes; this object is not an admission.
struct RedemptionConstraints {
    std::string subject, session_id, jti;
    std::optional<std::string> delegation_jti, required_digest, download_name;
    std::uint64_t request_byte_ceiling = 0;
    std::optional<std::uint64_t> max_uses, max_bytes_per_hour;
};

RedemptionConstraints check_capability_request(const std::string& token,
    const std::map<std::string, trust::lct::IssuerKey>& issuers,
    const std::optional<VerifiedRevocationState>& revocation,
    std::int64_t now, const CapabilityRequest& request);

} // namespace synergos::custody
