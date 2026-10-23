# Contract amendment: membership and node-roster authority, relay landing, Curare's role

- Date: 2026-10-23 (evening)
- Applies to: SY-SEC = Synergos `spec/plan/redesign-security.md`, SY-TRN = Synergos `spec/plan/redesign-transfer-efficiency.md`, SY-AG = Synergos `spec/plan/redesign-ai-game-dev.md`, CU-SEC / CU-TRN / CU-AG = the Curare documents with the same names.
- Source: `spec/plan/redesign-analysis/HANDOFF.md` §7.2 (owner decisions 7, 8, 9; neco; binding).
- How to apply: editors apply these blocks verbatim.
  - A quoted English block (`>`) replaces the whole normative block of the same id in that id's 正本 (owner document).
  - Documents that are not the 正本 quote the block verbatim or keep id + summary, following their own §4.0 rule.
  - Blocks marked "Insert (ja)" are inserted verbatim at the named place.
  - Every id not named here keeps its id and definition.

## 0. Decisions as constraints

| # | Decision | Constraint |
|---|---|---|
| 7 | DERP-style outbound relay only in C++ | SPEC-SY-relay-auth and SPEC-SY-relay-v2 are C++ migration contracts only. Nothing is added to the current Rust code. Until the C++ relay passes its gate, sites and external environments (for example Codex cloud) connect over Tailscale, with `synergos-node` bound to the tailnet IP only (Synergos `docs/site-operations.md`). |
| 8 | Cernere is the source of truth for project membership and the node roster | Cernere signs membership LCTs, node cards, ProjectRoster, RosterStatus and OfflineLease. synergos-control only distributes, caches and verifies them. It never decides membership, ownership or reachability, and it holds no membership or roster issuer key at any phase, including as a stopgap. Enrollment is approved in Cernere with step-up. Revocation flows from Cernere through SSF/CAEP. Requested as Actio tasks `actio:2a7af0fa` and `actio:fd0e4de8`. |
| 9 | Curare is designer-facing version control, a box around Synergos | Transfer and sync are Synergos-only. Curare acts only where versioning, review or approval is needed. Curare is not a byte path and not a membership authority. |

Consequences for existing decisions:

- **D-02:** option C stays for the custody-delegation, receipt and revocation (deny-only) keys. Membership and roster use Cernere's own keys. The Vault-pin interim is withdrawn for membership.
- **D-19:** the authority is Cernere for every project. `project_policy` moves to Cernere. The [変更提案] D-19 (i) against `opaque-custody.md:3-5` is withdrawn, and (ii) is moot.
- **D-21:** the OfflineLease is signed by Cernere. G11 (control-node multiplexing) stays.
- **D-23:** unchanged (DHT deleted). Discovery is the control-distributed directory of Cernere-signed cards.
- **D-A02:** re-opened (§4.3).

## 1. Replacement normative definitions

### 1.1 SPEC-SY-signed-envelope (正本: SY-SEC §4.1)
> Every signature is `Ed25519(ctx ‖ 0x00 ‖ ver:u8 ‖ JCS(payload))`. `ctx` comes from a closed registry: `synergos/{peer-session, membership, gossip, chain, invite, node-card, presence, project-roster, roster-status, heartbeat, enroll, revocation, receipt, lct, journal-checkpoint}`. Each registry row names its allowed signers: `membership`, `node-card`, `project-roster` and `roster-status` are signed only by Cernere kids published under SPEC-CR-capability-signing-keys; `enroll`, `heartbeat` and `presence` only by node identity keys; `revocation` only by the control-node deny-only kid; `receipt` and `lct` only by custody kids. A signature under a ctx by a signer not listed in its row is invalid. Payloads are RFC 8785 JCS with integers only, no floats, duplicate keys rejected, ≤1 MiB, ≤128 nesting levels. Golden vectors live in Synergos `spec/vectors/` and are imported by a Curare Revisor case at a pinned commit and by a Cernere conformance case at a pinned commit. Direct `sign(` calls outside the envelope module fail a lint gate. This removes cross-protocol signature reuse (SEC-CR-10) and the triple canonicalization (SEC-CR-12).

(D-28 `curare/{…}` stays open as Q-CU-9.)

### 1.2 SPEC-SY-lct-v1 (正本: SY-SEC §4.1)
> LUDIARS Capability Token. It is a chain of blocks `{v:1, kid?, caveats, next_pub}`. Block 0 is signed by the issuer key under ctx `lct`, except in the membership profile, whose block 0 is signed by a Cernere membership kid under ctx `membership` (SPEC-CR-membership-issuer). Block n is signed with the secret of block n−1's `next_pub`; the holder keeps only the last secret, and deleting it seals the token. The caveat vocabulary is closed: `iss, org, project_ref, projects[{project_ref, role}], ops[], object_keys[] \| prefix, digest, byte_range, download_name, nbf, exp, sub, session_id, device_pub, peer_id, jti, max_uses, max_bytes`. An `object_keys` entry is a storage key or `upload:<upload_id>`. `download_name` is a safe ASCII basename, valid only for GET, and is the only source of the `Content-Disposition` filename. Verification intersects all blocks (a later block can only narrow), checks `exp − now ≤ MAX_TTL(profile)`, and checks the revocation bundle (`jti`, `cap_issued_before`, frozen scopes) and, for the membership profile, the embedded Cernere roster status. There are three profiles: **membership**, **custody-delegation** and **custody-capability**. Transport is `Authorization: Capability <b64url>`; the URL form is allowed only for browser GET, with TTL ≤120 s.

### 1.3 SPEC-SY-membership-token (正本: SY-SEC §4.3)
> The membership profile of LCT v1. Block 0 is issued and signed only by Cernere (SPEC-CR-membership-issuer) under ctx `membership`: `{iss:"cernere", kid, sub_peer_id, node_id, principal (Cernere UUID of the owning human or service principal), device_id, org, projects[{project_ref, role: read\|publish\|history}], roster_epoch, exp ≤ now+3600, jti}`. `projects` is exactly the Cernere membership of `principal` intersected with the project scope of the node's Cernere-signed card. A membership LCT whose block 0 is not signed by a Cernere membership kid is rejected; synergos-control holds no membership issuer key and never mints, widens or re-signs a membership LCT. Renewal starts at 50 % of the lifetime: the node's signed heartbeat carries the renewal request, control-node relays it verbatim to Cernere and relays the new token back, and control may cache an issued token only to deliver it to the same node. `project_ref = blake3(org_id ‖ host_pubkey ‖ local_id)` for P2P-only projects; Curare-bound projects use SPEC-CU-project-descriptor; every `project_ref` is registered in Cernere before any member exists. Each connection installs a CapSet after SESS. Every stream (TXF2, SYNC, PLAN, RLY, GSP1) peeks `(magic, project_ref)` and checks `may(peer, project, op)` **before reading the body**; a deny sends `RESET 0x5303 forbidden` and a `peer.denied` event. A session whose SESS carries no membership LCT may open only `JOIN1` (SPEC-SY-invite-v2). TXFR v1, BSW1, DHT and FileWant are not accepted on `synergos/2`; history and archive nodes serve through TXF2 under the `history` or `read` role. Gossip fans out only to authorized peers and binds `origin = signer = sender`. History nodes need an explicit project list in their Cernere-signed card; `"*"` is refused.

### 1.4 SPEC-SY-node-enrollment (正本: SY-SEC §4.4)
> Enrollment is approved in Cernere, never in control. A Cernere org admin (or a principal that Cernere policy allows to own nodes) creates a single-use claim code in Cernere after SPEC-CR-step-up-assertion; it lasts 15 min and names the owning principal, the allowed roles and the project scope (SPEC-CR-node-roster). The node calls control-node `POST /v1/enroll {claim_code, node_pubkey, requested{roles, site_id, endpoints[]}, proof = Sig(enroll, claim_code ‖ node_id ‖ pubkey ‖ nonce)}`; control forwards the body verbatim to Cernere and relays the answer `{node_card (Cernere-signed, SPEC-SY-node-card), membership_lct (Cernere-signed)}`. Control does not evaluate, approve or record the claim code beyond forwarding. Node keys and connector tokens never appear in JSON. Heartbeat to control-node is `Sig(heartbeat, {node_id, peer_id, mesh_ip, version, ts, counter, card_hash, renew_membership?})`; it reports liveness and feeds the directory only and never changes the roster. Control rejects a counter regression, skew over 120 s, or a `peer_id`/`card_hash` that does not match the node's current Cernere-signed card. A key change is a dual-signed `KeyRotation{old_pub, new_pub, ts}` submitted through control to Cernere, which re-issues the card; changes of roles, project scope or declared endpoints are likewise Cernere card re-issues (roles and project scope need step-up). The identity key lives in the OS keystore by default; the file fallback is v2 with AAD-bound argon2 parameters, written atomically 0600.

### 1.5 SPEC-SY-node-card (正本: SY-SEC §4.5)
> A node card is the Cernere roster entry of one node (SPEC-CR-node-roster), signed only by Cernere under ctx `node-card`: `{v:2, org, node_id, peer_id, pubkey, principal (Cernere UUID of the owning human or service principal), device_id?, site_id?, projects_scope[] (explicit project_refs; required for history, archive and seed roles; "*" refused), endpoints[] (declared stable reachability, such as the tailnet address), protocol:"synergos/2", topo? (SPEC-SY-node-roles), roster_epoch, iat, exp ≤ 24 h}`. It replaces `/peer-info`. Reachability measured at run time is not in the card: the node publishes `Presence{node_id, peer_id, card_hash, candidates[], iat, exp ≤ 10 min}` signed by the node key named in the card under ctx `presence`, and a presence whose key or `card_hash` does not match a valid card is dropped. Control and peers distribute cards and presences but never sign or alter them. Every bootstrap names the expected `peer_id` (`bootstrap=[{url, peer_id}]`, `--peer`) and verifies the Cernere signature, so there is no TOFU. http is allowed only to loopback or the mesh/tailnet range. Every daemon-initiated fetch goes through the SPEC-CU-egress-policy classifier.

### 1.6 SPEC-SY-node-roles (正本: SY-TRN §4.10)
> **SPEC-SY-node-roles** (Cernere-signed card → Sy control, Sy daemons): The `topo` member inside the Cernere-signed SPEC-SY-node-card payload: `{roles ⊆ {member, seed, archive, relay, edge, custody-transfer, chunk-store}, site_id, relays[{url, region}], capacity{egress_mbps, cache_bytes}, caps}`, approved at enrollment or card re-issue (SPEC-CR-node-roster). Measured reachability `candidates[{type: host | srflx | mapped | relay, addr, prio, exp}]` is carried in the node-signed `Presence` of SPEC-SY-node-card, never in the card. Config `[node] modes = [...]`, `site_id`; a node whose configured modes are not a subset of its card's `roles` refuses to start. Start-up validation rejects forbidden co-residence (custody-admin or control-admin with any Internet-facing mode), per SPEC-SY-custody-topology. Reachability (`candidates`) is measured, never guessed from a global IPv6 address, an answering UPnP gateway or a `cloudflared` binary on PATH. The `relay` role is served only by the C++ relay (SPEC-SY-relay-v2).

### 1.7 SPEC-SY-directory (正本: SY-TRN §4.10)
> **SPEC-SY-directory** (Sy control-node → Sy daemons, relays, edges): `GET /v1/nodes/self/directory?project=<ref>&etag=` authenticated by a membership LCT returns `{project_ref, roster (the Cernere-signed ProjectRoster of SPEC-CR-membership-issuer), seeds[card], relays[card], edges[card], members_sample[card ≤ 32, chosen by site and RTT], presences[], site_map{site_id: [cidr]} (taken from the cards), revocation_seq, roster_epoch, exp}`, where every card is Cernere-signed (SPEC-SY-node-card) and every presence is node-signed. Control does not sign the directory: it selects, caches, orders and may omit entries, but it cannot add a node, a member or an endpoint, because each element verifies on its own; a node drops any card absent from the ProjectRoster's `nodes[]`. `exp` = the smallest `exp` of the roster and the listed cards, at most 24 h. Refresh every 5 min; the signed heartbeat response carries `{directory_etag, revocation_seq, roster_epoch}`. Membership is derived only from the Cernere ProjectRoster and membership LCTs (SPEC-CR-membership-issuer). Nodes persist the cards next to `projects.json`, dial seeds and relays first (8 in parallel, exponential backoff with full jitter capped at 5 min, immediate retry on network change), and fall back to an mDNS beacon `{peer_id, card_hash}` on the same site. After `exp` a node may only keep already-authenticated sessions. DHT1 is not ported.

### 1.8 SPEC-SY-invite-v2 (正本: SY-SEC §4.5)
> `{project_ref, host_peer_id, host_pubkey, invitee?, role, nonce16, iat, exp (24 h default, 7 d max), uses (default 1)}`, signed under ctx `invite`. It is consumed through a `JOIN1` stream over an authenticated session against the host's ledger; the joining node must already hold a Cernere-signed node card. The host then forwards a membership *request* `{invite, invite_proof, joiner_node_id, joiner_principal}` through control-node, which relays it verbatim to Cernere (SPEC-CR-membership-issuer `member-requests`). Cernere decides with the project's Cernere policy (`invite_max_role`, `require_approval`, owner approval with step-up when required) for every project, Curare-bound or P2P-only; control and the host never decide. On approval Cernere issues the membership LCT and a new ProjectRoster epoch. Includes `invite ls` and `invite revoke`.

### 1.9 SPEC-SY-revocation-bundle (正本: SY-SEC §4.7)
> `GET /v1/orgs/{org}/revocation?since=seq` returns a bundle of two independently signed parts. (1) `roster_status`: the latest Cernere-signed `RosterStatus` (SPEC-CR-node-roster, ctx `roster-status`), embedded verbatim; it is the only source of `nodes_revoked[]`, `principals_revoked[]`, `memberships_revoked[{principal, project_ref}]` and membership `sessions_revoked[]`. (2) `local`: `Sig(revocation){seq (monotonic), issued_at, exp = issued_at+300, jtis_revoked[], delegations_revoked[], cap_issued_before{org\|principal: ts}, sessions_revoked[], principals_frozen[], projects_frozen[{project_ref, mode: read_only\|full}], orgs_frozen[], quarantined_nodes[], invites_revoked[]}` signed by the control-node deny-only kid. Every entry of `local` can only deny; nothing in this bundle grants, restores or widens access, and a PEP denies when either part denies. PEPs poll every 30 s and also on a Cernere `roster-changed` SSF event. If either part is past its `exp`, a PEP denies new sessions and redemptions; streams on an already-authenticated session continue only until that session's membership LCT `exp`. Listed peers and sessions close immediately. A project may hold a Cernere-signed `OfflineLease{project_ref, site_id, site_cidrs[], peers[{peer_id, role}], issued_at, exp ≤ 72 h, max_stale ≤ 24 h}` (ctx `project-roster`, issued by Cernere on the project owner's step-up; every listed peer must be a Cernere member with at least that role); while it is valid, PEPs on that site keep granting CapSets to the listed peers from `site_cidrs` for up to `max_stale` past the bundle `exp`. Inputs to `local` are (1) SPEC-CR-ssf-caep-transmitter events, applied at once as session closes and confirmed by the next `RosterStatus`; (2) Curare's SSF receiver through `POST /v1/orgs/{org}/revocation/inputs` on `control-admin`, body `{source:"curare", idempotency_key, jtis[], cap_issued_before{<principal>: ts}}`, authenticated by the SPKI-pinned mTLS client cert of SPEC-SY-custody-service-auth plus a SPEC-CR-service-principal JWT once available, answering `202 {input_id, effective_from_seq}` and returning the first result for a repeated `idempotency_key`; and (3) the operator incident API `POST /v1/incident/{freeze\|quarantine\|revoke}`, which requires SPEC-CR-step-up-assertion, plus a second approver for org-wide scope. Incident freezes and quarantines are emergency, deny-only local entries; removing a principal or node from membership or the roster is done in Cernere, and control never lifts a Cernere revocation. The pinned verification keys are `SYNERGOS_REVOCATION_PUBKEYS` (control deny-only kids) and the Cernere `roster` kids of SPEC-CR-capability-signing-keys.

### 1.10 SPEC-SY-control-admin-auth (正本: SY-SEC §4.10)
> Operators sign in with Cernere OIDC code+PKCE and get an HttpOnly, SameSite=Strict cookie (30 min idle, 8 h absolute). Roles come from Cernere org roles: `synergos.node_admin`, `synergos.mesh_operator`, `synergos.account_admin`. Control-admin has no route that creates, approves, changes or removes project membership, node enrollment, node ownership, node keys or declared node reachability; such requests answer `409 authority_is_cernere` with a link to the Cernere roster or membership page. `revoke_dark` and connector issuance need step-up (`auth_time` < 5 min); `revoke_dark` also needs an approved dry-run diff and a second person. Curare uses a service principal scoped to `org:<id>:nodes.read` only. Cloudflare uses a dedicated least-scope token whose scope is checked at boot and fails closed; there is no request-scoped token path, and `revoke_dark` uses a JIT token from the Excubitor Cloudflare broker. The break-glass token is single-use with forced rotation and cannot reach any membership or roster function. Every route declares `{principal_kind, scope, step_up}`.

### 1.11 SPEC-CU-membership-export (正本: CU-SEC §4.5)
> **SPEC-CU-membership-export** — Retired as an authority channel. Curare never exports, mints, decides or caches-as-authority Synergos project membership or node roster entries; both are owned by Cernere (SPEC-CR-membership-issuer, SPEC-CR-node-roster). For a Curare-bound scope, Curare (a) registers the scope's `project_ref` (SPEC-CU-project-descriptor) with Cernere at scope creation, acting as the creating human with that human's Cernere authority; (b) forwards a membership change made in Curare's UI to Cernere as the acting human, so Cernere's own authorization and step-up apply, and shows Cernere's answer; and (c) reads Cernere's signed ProjectRoster to display members and to bound Curare's own permissions (SPEC-CU-authority-model). Curare holds no Cernere credential that can write membership on its own, and synergos-control receives nothing from Curare about membership.

### 1.12 SPEC-CU-authority-model (正本: CU-SEC §4.2)
> **SPEC-CU-authority-model** — Curare permissions apply only to principals that Cernere lists as members of the org (org spaces) or of the Cernere project of a Curare-bound scope; Curare owners, grants and roles narrow inside that membership and never add org membership, project membership, Synergos data-plane roles or node roster entries. An explicit `owner` with audited transfer replaces creator omnipotence. Org owner and admin get manage, review and release on org spaces. Separation of duties: `decision.createdBy ≠ submission.createdBy`. Grants are validated against Cernere and gain `granted_by, granted_at, expires_at, reason`; only an owner or org admin may grant `manage`. Pins require `release`, and pin events are append-only. Unauthorized reads return a uniform 404. Deleting dependencies validates `targetId`. Project delete goes through SPEC-CU-destructive-op.

### 1.13 SPEC-CU-principal-model (正本: CU-AG §4.18; becomes an English quoted block)
> **SPEC-CU-principal-model** — `principals{id, kind: human\|service\|agent\|guest, operator_user_id, org_id, scopes[], expires_at}` is Curare's local projection of Cernere principals: `id` is the Cernere UUID for human, service and agent principals, and `kind` comes from Cernere claims (SPEC-CR-service-principal once available; until then the `agent_accounts` registration of CU-SEC D-27). `created_by_principal` is recorded on changesets, versions, runs, scans and audit rows. The idempotency key is (principal, operation_id). An `agent` may commit only to `agent/*` refs, only the `working` class, only with an open run and `digital_source_type`. Quotas are per principal. Codex tokens come from the per-task broker, not from Codex's secret store (`codex-cloud-access.md` is revised accordingly). Interim Curare-issued agent tokens (CU-SEC D-27 P2, ctx `curare/agent-token`) and link-scoped guest principals (CU-SEC D-23) authorize Curare API calls only: they never confer Cernere membership, Synergos membership LCTs or node roster entries, and an agent or service that runs `synergos-node` must be a Cernere service principal enrolled through SPEC-CR-node-roster. Interim issuance ends when SPEC-CR-service-principal / SPEC-CR-guest-identity ship (CU-AG §4.18 exit condition).

### 1.14 SPEC-CU-synergos-control (正本: Curare `spec/feature/production-lifecycle-contract.md`; replacement proposed in CU-SEC §2.3 CP-05 and §4.5)
> **SPEC-CU-synergos-control** — Curare's Synergos panel is display-only. It uses a `read` IPC token (SPEC-SY-ipc-capability) for `Status`, `NetworkStatus`, `ProjectList`, `PeerList` and `TransferList`, and control's `org:<id>:nodes.read` scope (or a Cernere roster read) for node listings. Curare does not change sync mode, peers, transfers, node names, membership or the roster: sync and transfer settings are changed in the Synergos daemon's CLI or GUI, and membership and roster in Cernere. Transfer and sync run in Synergos alone; Curare takes part only in designer versioning, review and approval flows, where it authorizes byte movement by issuing custody capabilities (SPEC-SY-custody-capability-v2) and never relays bytes.

## 2. New Cernere contracts (requests to Cernere; 正本: SY-SEC §4.34)

Sent as Actio `actio:fd0e4de8` (SPEC-CR-membership-issuer and SPEC-CR-node-roster); the signing-key publication, SSF/CAEP and step-up requests are `actio:2a7af0fa`. Rules shared by both contracts:
- Dedicated Cernere Ed25519 kids with `purpose: membership` and `purpose: roster`, published in the JWKS with rotation and kept separate from the user-JWT keys.
- Cernere runs the Synergos golden-vector conformance case at a pinned commit.
- Every change emits the SSF events `project-membership-changed`, `node-revoked` and `roster-changed`.
- No email addresses appear in any signed object.

### 2.1 SPEC-CR-membership-issuer
> **SPEC-CR-membership-issuer** (request to Cernere; 正本: Synergos `spec/plan/redesign-security.md` §4.34) — Cernere is the source of truth for Synergos project membership and the only issuer of membership tokens. **Projects:** `synergos_projects{org, project_ref, kind: curare_bound\|p2p_only, owner_principal, policy{invite_max_role, require_approval, offline_lease_allowed}, created_at}`; a project is registered by its owner (for a Curare-bound scope, Curare forwards the creating human's request). **Members:** `synergos_memberships{org, project_ref, principal, role: read\|publish\|history, granted_by, granted_at, expires_at?}`; grants of `publish` or `history` and policy changes require SPEC-CR-step-up-assertion; removals take effect at once. **Member requests:** `POST /v1/synergos/projects/{ref}/member-requests {invite, invite_proof, joiner_node_id, joiner_principal}` decides an invite (SPEC-SY-invite-v2) under the project policy, with owner approval when `require_approval`. **Tokens:** `POST /v1/synergos/membership-tokens` takes the node's signed heartbeat (ctx `heartbeat`, relayed verbatim by synergos-control), verifies it against the node's current card, and returns an LCT v1 membership-profile token whose block 0, under ctx `membership`, is `{iss:"cernere", kid, sub_peer_id, node_id, principal, device_id, org, projects[{project_ref, role}], roster_epoch, exp ≤ now+3600, jti}` with `projects` = memberships of `principal` ∩ the card's `projects_scope`; p99 latency < 1 s. **Roster view:** `GET /v1/synergos/projects/{ref}/roster` returns `ProjectRoster{org, project_ref, epoch, members[{principal, role}], nodes[{node_id, peer_id, card_hash}], issued_at, exp ≤ 24 h}` under ctx `project-roster`, readable by members and by synergos-control's service principal. **Offline lease:** on the owner's step-up and only if `offline_lease_allowed`, Cernere issues `OfflineLease{project_ref, site_id, site_cidrs[], peers[{peer_id, role}], issued_at, exp ≤ 72 h, max_stale ≤ 24 h}` under ctx `project-roster`, listing only current members at no more than their role. Synergos components verify these objects and never produce them.

### 2.2 SPEC-CR-node-roster
> **SPEC-CR-node-roster** (request to Cernere; 正本: Synergos `spec/plan/redesign-security.md` §4.34) — Cernere is the source of truth for which node belongs to which principal and where it is reachable, and the only signer of node cards. **Claim codes:** an org admin, or a principal Cernere policy allows to own nodes, creates after SPEC-CR-step-up-assertion a single-use claim code (128-bit, shown once, stored as SHA-256, 15 min) naming `{owner_principal, roles_allowed, projects_scope, site_id?}`. **Enrollment:** `POST /v1/synergos/enroll` takes the node's enrollment body (SPEC-SY-node-enrollment) relayed verbatim by synergos-control, checks the claim code and the proof under ctx `enroll`, consumes the code, and returns a SPEC-SY-node-card signed under ctx `node-card` (`exp ≤ 24 h`, re-signed before expiry while the node stays enrolled) plus the first membership token (SPEC-CR-membership-issuer). **Changes:** a dual-signed `KeyRotation{old_pub, new_pub, ts}`, a change of declared `endpoints[]`, and (with step-up) a change of roles or project scope each produce a re-issued card and a new roster epoch. **Removal:** removing a node or its owner's org membership revokes the node at once. **Status:** `GET /v1/synergos/orgs/{org}/roster-status` returns `RosterStatus{org, epoch (monotonic), issued_at, exp = issued_at+300, nodes_revoked[], principals_revoked[], memberships_revoked[{principal, project_ref}], sessions_revoked[]}` under ctx `roster-status`, re-issued at least every 60 s and on every change; `GET /v1/synergos/orgs/{org}/roster?since=epoch` returns the changed cards for synergos-control's service principal. A revocation is visible in `RosterStatus` within 10 s and pushed as `node-revoked` / `roster-changed` SSF events, so that, together with the 30 s PEP poll, G1 (p99 ≤60 s) holds.

### 2.3 Existing SPEC-CR summary changes (SY-SEC §4.34, SY-TRN §4.27, SY-AG §4.24, CU-SEC §4.19, CU-AG §4.4)
- **SPEC-CR-capability-signing-keys:** the delegation-record purpose drops `membership` and keeps custody-delegation, receipt and revocation (deny-only). The `membership` and `roster` kids go in the JWKS.
- **SPEC-CR-ssf-caep-transmitter:** add the events `project-membership-changed`, `node-revoked` and `roster-changed`.
- **SPEC-CR-step-up-assertion:** add these uses: claim codes, role or scope re-issue, publish/history grants, OfflineLease.
- **Stopgap before the two new contracts ship: none in Synergos.** P3 (membership and enrollment parts) and P4 wait for Cernere. Until then sites run the current Rust runtime with M34 over Tailscale. No Synergos component may hold a membership or roster key.

## 3. Decision 7 wording

- **R7-A (ja).** Add as the first bullet of SY-SEC §4.6, SY-TRN §3.9 and §4.12, and the CU-TRN D-T04〜D-T10 summary:
  「- **着地 (オーナー決定 7、2026-10-23 夕)。** 外向きのセキュアな中継 (DERP 型 relay: 両端が 443 へ outbound で接続し、relay は end-to-end 暗号化された QUIC を中継するだけ) は C++ 版でだけ作る。SPEC-SY-relay-auth と SPEC-SY-relay-v2 は C++ 移行の契約で、現行 Rust には何も足さない (`synergos-relay` の Rust 実装は改修も配備もしない)。C++ relay が受入ゲートを通るまで、拠点と外部環境 (Codex クラウドなど) は Tailscale でつなぎ、`synergos-node` は tailnet IP にだけ bind する (Synergos `docs/site-operations.md`)。」
- **R7-B (ja).** Add after the relay-auth quote (SY-SEC §4.6) and the relay-v2 quote (SY-TRN §4.12); the definitions themselves do not change:
  「本契約は C++ でだけ実装する (オーナー決定 7)。`native-contracts.md` の `[id:]` 節として移し、Rust 実行時には入れない。暫定の到達経路は Tailscale (tailnet IP への bind) で、relay の代替ではなく、C++ relay が無い間の唯一の経路である。」
- **R7-C (ja).** Add to SY-SEC §4.31 (M34), SY-TRN §4.24 (M33), and the CGNAT and vendor/agent rows of SY-TRN §2.2 and CU-TRN §2.2:
  「暫定 (C++ relay まで): Tailscale で到達させる。`synergos-node` の QUIC は tailnet IP にだけ bind し、`0.0.0.0` や mesh 外のアドレスに bind しない。外部環境 (Codex クラウドなど) は tag 付き・ephemeral の Tailscale ノードとして参加し、到達範囲は Tailscale ACL で決める (ACL の変更は拠点の運用判断で、`docs/site-operations.md` に記録する)。」
- **SY-SEC §4.31 (M34) table:**
  - relay row → 「Rust の relay は起動しない (オーナー決定 7: relay は C++ だけ)。到達は Tailscale で行う」
  - QUIC row title → 「QUIC を tailnet IP に限る」; value 「`quic.listen_addr` に tailnet IP を明示する (Synergos `docs/site-operations.md`)。`[::]` や `0.0.0.0` にしない」
  - `/peer-info` row: append 「公開する場合も tailnet IP だけに bind する」
- **SY-SEC §4.6 変更対象:** 「C++ relay (`synergos-relay/native/`) が受入ゲートを通るまで relay を配備しない。到達は Tailscale (M34、オーナー決定 7)」
- **SY-TRN:**
  - TG-10: add 「(C++ relay でだけ測る。Rust には relay の経路を作らない、オーナー決定 7)」
  - §4.11: 「overlay は任意の下層にする。C++ relay までは Tailscale が唯一の NAT 越え経路で、`synergos-node` は tailnet IP にだけ bind する (オーナー決定 7)」
  - §4.24 TRN-TP-05: 「overlay は Tailscale。bootstrap 一覧は tailnet IP を書き、同一拠点では LAN アドレスを先に書く」
  - §7 Q7: append 「relay は C++ 版でだけ配備する (オーナー決定 7)。それまでは Tailscale」
- **SY-SEC §5.1 P4 and SY-TRN §5.1 T3:** add to the dependencies 「C++ relay だけ (オーナー決定 7)」.

## 4. Decision 9 wording

- **R9-A (ja).** Add rows 7–9 to the §1.1 decision table of all six documents:

  | # | 決定 | 本書での扱い |
  |---|---|---|
  | 7 | 外向きの DERP 型 relay は C++ 版でだけ作る。それまで拠点と外部環境は Tailscale (`synergos-node` は tailnet IP にだけ bind) | SPEC-SY-relay-auth / SPEC-SY-relay-v2 は C++ 移行の契約。現行 Rust には何も足さない (`design/authority-amendment.md` §3) |
  | 8 | プロジェクト会員とノード名簿の正本は Cernere。synergos-control は Cernere 署名の会員証・名簿を配布・キャッシュ・検証するだけ | 会員証 (membership LCT)・node card・ProjectRoster・RosterStatus・OfflineLease は Cernere が署名する。control は会員を決めず、その発行鍵を持たない。enrollment の承認は Cernere (step-up)。失効は Cernere から SSF/CAEP で流れる (SPEC-CR-membership-issuer、SPEC-CR-node-roster。Actio `actio:2a7af0fa`、`actio:fd0e4de8`) |
  | 9 | Curare はデザイナー向けの版管理で、Synergos のハコ | 転送・同期の本体は Synergos 単体。Curare が関わるのは版・レビュー・承認が要る場面だけ。Curare はバイトの経路でも会員資格の権威でもない |

- **R9-B (ja).** Add after the 2026-09-08 boundary bullet in §1.1 of all six documents:
  「2026-10-23 夕の決定 8・9 で、この境界を次のように締める。Cernere は本人・組織・所属に加えて、**Synergos のプロジェクト会員 (誰がどのプロジェクトで read / publish / history か) とノード名簿 (どのノードが誰のもので、どこに到達できるか) の正本**である。Synergos は転送と同期の本体で、Curare が無くても動く。Curare はデザイナー向けの版・レビュー・承認を持ち、それが要る場面でだけ capability を出してバイトの移動を認可する。Curare はバイトを中継せず、会員資格を輸出・発行・判定しない。」
- **R9-C (ja).** Add once to each section where Curare looks like the transfer path (fetch session, changeset, magic link, gateway, acquisition, LFS, vendor link):
  「Curare は「どの版を、誰に、どの用途で渡してよいか」を判定して capability を出すだけで、バイトは常に Synergos (custody / edge / seed / P2P) が運ぶ。版・レビュー・承認を伴わない同期 (メンバー間の作業ツリーの同期) は Curare を通らない。」
- **R9-D (ja).** Add wherever Curare looks like the membership authority (SPEC-CU-membership-export, D-19 summaries):
  「Synergos のプロジェクト会員の正本は Cernere (SPEC-CR-membership-issuer)。Curare は会員を輸出・発行・判定しない。Curare 紐付けの scope では、作成者の権限で `project_ref` を Cernere に登録し、Curare の UI での会員変更は操作者本人として Cernere へ取り次ぐだけ (SPEC-CU-membership-export の改訂後の定義)。」
- **Specific replacements:**
  - CU-SEC §2.2 a → 「(Sy が正本) 会員資格の正本は Cernere。Curare は関与しない」
  - CU-SEC §2.2 c → `org:<id>:nodes.read`
  - CU-SEC §3.19 D-19 body → 「採用: 会員資格の正本は Cernere (オーナー決定 8)。Curare 紐付けかどうかによらず、Cernere が会員証を発行し、synergos-control は配布・キャッシュ・検証だけを行う。Curare への帰結: Curare は会員資格の入力元にも判定点にもならない。scope 作成時の `project_ref` の登録と、UI での会員変更の取り次ぎだけを行う (SPEC-CU-membership-export、§4.5)。正本: Synergos 側 §3.16 の D-19 行。」
  - CU-SEC §4.5:
    - Title → 「…会員資格の取り次ぎ」
    - Membership bullet → 「新設 src/synergos/membership-export.ts は、Curare 紐付け scope の作成時に作成者本人の資格で `project_ref` を Cernere へ登録し、UI での会員変更を操作者本人として Cernere の SPEC-CR-membership-issuer へ取り次ぐ。control へは何も送らない。Cernere の ProjectRoster を読んで表示と Curare の権限の上限に使う」
    - Control-client scope → `nodes.read` only
    - Invariant → 「Curare は会員資格を作らない。Curare の権限は Cernere の会員の内側でだけ効く」
    - Failure → 「Cernere 不達のとき会員変更は 503 で拒否し、再送待ちにしない」
  - CU-SEC §4.0 M10 role → 「消費 (クライアント)。SPEC-CU-membership-export は取り次ぎだけの契約」
  - CU-TRN §5.1 T3 → 「Curare の変更なし (会員資格は Cernere。Curare は取り次ぎだけ)」
  - CU-TRN §0 → 「Curare は変更しない」
  - CU-TRN 既存決定まとめ 2026-09-08 row: append 「会員とノード名簿は Cernere (決定 8)。転送・同期は Synergos 単体で、Curare は版・承認が要る場面だけ (決定 9)」
  - CU-AG §4.7 ref-log readers → 「…その `project_ref` の Cernere 署名 membership LCT を持つノード」
  - X-AG5 rows (CU-AG ×2, SY-AG §4.23): target → SPEC-CR-membership-issuer; text 「…`project_ref` 導出は SPEC-CU-project-descriptor が固定し、その `project_ref` で Cernere に登録する」
  - SY-AG §4.22 → 「会員資格は Cernere が発行し、control は配布・検証だけを行う (security D-19、オーナー決定 8)」
- **R9-E (ja), D-A02.** Add after the adoption sentence in SY-AG §3.2 and at the matching place in CU-AG; also extend Q-A2:
  「**オーナー決定 9 との衝突 → 決定 10 (2026-10-23 neco) で解消。** 採用 C は Curare の無いプロジェクトが C++ で同期できなくなる。決定 9 は「転送・同期の本体は Synergos 単体」とするので、Curare の ref 権威は **Curare 紐付けプロジェクトの版・レビュー・承認** に限り、P2P だけのプロジェクトは Cernere の会員 (SPEC-CR-membership-issuer) が署名する publish 役割の Head (SPEC-SY-replication-stream の member-signed head、X-AG3 の代替) で同期する案とする (決定済み)。オブジェクトモデル (SPEC-SY-object-model) と changeset は両方で共通で、`acceptor_sig` は Curare 紐付けのときだけ要る。」

## 5. Change table

**SY-SEC:**
- §0: add Cernere signing and the relay landing.
- §1.1: R9-A and R9-B.
- §1.2 G2: the claim code is issued by Cernere.
- §1.2 G11: add a note on Cernere availability.
- §1.3: Cernere row (membership issuer, roster).
- §2.8 b/c/l: no ownership change via control; 409; R7-C.
- §3.2 D-02: narrowed.
- §3.16 D-19: replaced; (i) withdrawn.
- §3.17 D-21: the lease is Cernere-signed.
- §3.19 D-23: wording.
- §3.22: delete the D-19 (i) row.
- §4.0.1: M04, M05, M07, M10 roles.
- §4.0.2 X2: topo/Presence split.
- §4.1: §1.1 and §1.2 plus `signers[]`.
- §4.3: §1.3.
- §4.4: §1.4; claim-code bullets move to Cernere; the registry becomes a copy.
- §4.5: §1.5, §1.8; invariant "issuer is Cernere".
- §4.6: R7-A, R7-B, 変更対象.
- §4.7: §1.9; the known gap becomes "P3 membership and P4 wait for Cernere".
- §4.10: §1.10.
- §4.17: remove the `synergos-control-membership` key.
- §4.31: §3 table and R7-C.
- §4.33: membership-export row.
- §4.34: add §2.1 and §2.2 as full quotes plus the §2.3 updates.
- §4.35: rows for issuer keys, claim code, node registry, membership registry, project_policy, OfflineLease, and a new Presence row.
- §4.37: Control row reworded; add the Relay row.
- §5.1: P3/P4 dependencies.
- §5.2: drop the D-19 bullet; migrate registry nodes to Cernere.
- §6.1: AT-SY-SEC-06, 13, 32 updated; add AT-SY-SEC-40 (control-made LCTs, cards and status are rejected; 409 authority_is_cernere) and AT-SY-SEC-41 (directory tampering is dropped).
- §7: Q1, Q7, Q-SY-6 updated; add Q-SY-9 (Actio mapping and delivery).

**SY-TRN:**
- §1.1: R9-A, R9-B.
- §1.2 TG-10: C++ only.
- §1.3: Cernere row.
- §2.2: R7-C.
- §3.7, §3.8, §3.9: R7-A and Cernere-signed directory.
- §4.10: §1.6, §1.7.
- §4.11, §4.12, §4.24: §3 items.
- §4.26: "(Cernere 署名)".
- §4.27: new SPEC-CR rows.
- §4.29: authority of cards.
- §4.31: Topology row note.
- §5.1 T3: dependencies.
- §6.2 test 15: drop cards not in the roster.
- §7 Q7: C++ only.

**SY-AG:**
- §1.1: R9-A, R9-B.
- §1.3: Cernere row.
- §3.2: R9-E.
- §4.3: register `project_ref` in Cernere.
- §4.19: Cernere-signed LCT.
- §4.22: §4 sentence.
- §4.23: X-AG5 and the membership-token row.
- §4.24: SPEC-CR rows.
- §7 Q-A2.

**CU-SEC:**
- §0: revocation from Cernere.
- §1.1: R9-A, R9-B.
- §1.3: Cernere and relay rows.
- §2.2 a/c/l.
- §2.3 CP-05: becomes §1.14.
- §3.2 D-02, §3.19 D-19.
- §4.0 M10.
- §4.2: §1.12.
- §4.4: bundle summary; input endpoint named.
- §4.5: §1.11 plus mechanism, invariants and failure; control-admin-auth translation (§1.10).
- §4.19: SPEC-CR rows.
- §4.21: ProjectRoster cache row.
- §4.22: M10.
- §7: Q7, Q8.
- §7.1: add R14.

**CU-TRN:**
- §0, §1.1 (R9-A, R9-B), §1.3.
- §2.2: R7-C.
- D-T summary: R7-A.
- D-T14, §4.3, §4.6: R9-C.
- 既存決定まとめ.
- §4.0: membership-export note.
- §4.12: R7-C.
- §4.16: SPEC-CR rows.
- §5.1 T3.

**CU-AG:**
- §1.1 (R9-A, R9-B), §1.3.
- §2.3 server-client-architecture.md:30 row and D-A12: Curare tokens never confer membership or roster entries.
- D-A02 reference: R9-E.
- §4.4: SPEC-CR rows.
- §4.6: `project_ref` registration.
- §4.7: ref-log readers.
- §4.18: §1.13 in English.
- X-AG5 rows.
- Transfer-path sections: R9-C.

## 6. Ids touched

- **Replaced:** SPEC-SY-signed-envelope, -lct-v1, -membership-token, -node-enrollment, -node-card, -node-roles, -directory, -invite-v2, -revocation-bundle, -control-admin-auth; SPEC-CU-membership-export, -authority-model, -principal-model, -synergos-control.
- **New:** SPEC-CR-membership-issuer, SPEC-CR-node-roster; ctx `presence`, `project-roster`, `roster-status`; AT-SY-SEC-40, AT-SY-SEC-41; Q-SY-9.
- **Annotated only:** SPEC-SY-relay-auth, SPEC-SY-relay-v2, SPEC-CR-capability-signing-keys, SPEC-CR-ssf-caep-transmitter, SPEC-CR-step-up-assertion.
- **Checked and unchanged:** SPEC-SY-hlo2-session-binding, SPEC-SY-custody-service-auth, -custody-capability-v2, -custody-receipt-v2, -custody-topology; SPEC-CU-ssf-receiver, -project-descriptor, -capability-ledger; all other ids.
