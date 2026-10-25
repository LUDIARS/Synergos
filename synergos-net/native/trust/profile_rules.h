// profile ごとの規則 (翻訳単位の内部用)。違反は lct.profile_rule。
//   custody-delegation: SPEC-SY-custody-service-auth
//   custody-capability: SPEC-SY-custody-capability-v2
//   membership: SPEC-SY-membership-token (Cernere authority bindings)
#pragma once
#include "caveats.h"
#include "synergos/trust/lct.h"

namespace synergos::trust::lct::detail {

/// 提示 profile として受け取ってよい発行 profile か。capability は delegation を狭めたものでもよい。
bool presentable(Profile issued, Profile presented) noexcept;

/// 発行時 (block 0) の caveats に対する規則。
void check_issued(Profile issued, const Caveats& head);

/// 提示 profile の規則を、全 block の積集合に対して検査する。
void check_presented(Profile presented, const Caveats& effective);

} // namespace synergos::trust::lct::detail
