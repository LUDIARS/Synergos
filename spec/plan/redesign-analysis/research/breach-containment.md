# Mechanisms that curb vulnerabilities and contain breaches in asset-management / VCS / file-transfer platforms

Research notes for the Synergos (Rust: P2P QUIC transfer, whole-file BLAKE3 content-addressed history nodes, Cloudflare tunnel/relay, control plane, custody service with HMAC-signed PUT/GET URLs and SHA-256-verified receipts) + Curare (TypeScript/Hono/PostgreSQL/MinIO asset catalog, working/submission/reproducible versioning, submissions/review/release, access policy via the central identity service "Cernere") redesign. Date: 2026-10-15.

**Method and limits.** All claims below come from web-search result excerpts with the URL attached. Direct document fetches (`csrc.nist.gov`, `docs.aws.amazon.com`, `docs.github.com`, `doc.biscuitsec.org`, `docs.sigstore.dev`) were refused by the session's egress proxy, and the shared search budget was exhausted before every sub-topic was covered. Sub-topics for which **no source was found in this session** are flagged as such; nothing there should be treated as cited. Cost estimates in the final catalogue are engineering judgement, not sourced.

---

## 0. Threat model this document is written against

The platform moves and stores the most leak-sensitive artefact a studio owns (unreleased game/AI assets and source), across P2P nodes, a relay, a control plane and a web catalog. Looking at the incident set in section 6, almost every real game-studio breach followed one of four paths:

1. **A single human credential/session was stolen** (phishing, infostealer on a laptop, session cookie in a HAR file) and then had *standing*, *broad*, *long-lived* access: Rockstar 2022, Okta 2023, CircleCI 2023, LastPass 2022.
2. **An exposed or legacy network edge** (VPN appliance, Helix Core on port 1666) gave an attacker a foothold that escalated to domain admin: Capcom 2020, Rhysida/Insomniac 2023 (claimed "domain admin in under 25 minutes").
3. **A trusted supply-chain component ran with secrets in scope**: Codecov 2021, SolarWinds SUNSPOT, GitHub OAuth tokens 2022, tj-actions 2025.
4. **The data itself was executable**: Maya scriptNode worms, FBX/importer RCEs, Blender auto-run scripts.

So the mechanisms that matter most are: (a) make every credential short-lived, narrowly scoped and revocable at once; (b) make the storage tier unable to lose history even to an admin; (c) make every byte and every receipt verifiable after the fact; (d) treat uploaded assets as hostile input; (e) detect bulk exfiltration within minutes, not weeks.

---

## 1. Identity & access

### 1.1 Zero trust as the frame (NIST SP 800-207)

NIST SP 800-207 (final PDF: https://nvlpubs.nist.gov/nistpubs/specialpublications/NIST.SP.800-207.pdf) defines the architecture around a Policy Decision Point (Policy Engine + Policy Administrator) and Policy Enforcement Points in the data path, and lists seven tenets. The NCCoE briefing (https://csrc.nist.gov/CSRC/media/Presentations/zero-trust-networks-brief/images-media/2-4Kerman%20-%20Zero%20Trust%20Architecture%20-%20NCCoE%20-%202019%20-%20ISPAB.pdf) enumerates them: all data sources and computing services are resources; all communication is secured regardless of network location; access to individual resources is granted on a **per-connection basis**; access is determined by **dynamic policy including the observable state of user identity and the requesting system**; the enterprise keeps owned systems in the most secure state possible and monitors them; authentication and authorization are dynamic and strictly enforced before access; and (tenet 7, not in the retrieved excerpt) the enterprise collects asset/network state to improve policy. NIST SP 800-207A (https://nvlpubs.nist.gov/nistpubs/SpecialPublications/NIST.SP.800-207A.pdf) and CSWP 20 "Planning for a Zero Trust Architecture" (https://nvlpubs.nist.gov/nistpubs/CSWP/NIST.CSWP.20.pdf) restate the same tenets for cloud-native/multi-location deployments.

**Mapping to Synergos/Curare:** the control plane is the PDP; every Synergos node, the relay and the Curare API are PEPs. "Per-connection" means a QUIC session from a node must be authorised per session with a fresh, scoped credential, not by a node-level long-lived key.

### 1.2 Identity-aware proxies with device posture

- **Cloudflare Access**: policies can require the Cloudflare One (WARP) client as a posture check, evaluated at login *and continuously* for self-hosted apps (https://developers.cloudflare.com/cloudflare-one/reusable-components/posture-checks/client-checks/require-warp/, https://developers.cloudflare.com/cloudflare-one/access-controls/policies/). Posture checks are only available after enrollment; restricting *which devices may enroll* requires mutual TLS, Enterprise plan only (https://developers.cloudflare.com/learning-paths/replace-vpn/configure-device-agent/device-enrollment-permissions/). Client-certificate posture is an alternative (https://developers.cloudflare.com/cloudflare-one/identity/devices/warp-client-checks/). Design guide: https://developers.cloudflare.com/reference-architecture/design-guides/designing-ztna-access-policies/.
- **Tailscale grants**: the recommended policy syntax; a grant can carry `srcPosture` (array of device-posture conditions) in addition to `ip`/`app` fields; ACLs and grants are at feature parity for device posture (https://tailscale.com/docs/features/access-control/grants.md, https://tailscale.com/docs/reference/grants-vs-acls.md, https://tailscale.com/kb/1018-acls).

**Implication:** since Synergos already fronts nodes with a Cloudflare tunnel/relay, Cloudflare Access with a WARP/mTLS posture requirement is the lowest-cost way to get device posture on the relay path. P2P direct paths bypass it, so the control plane must itself demand a posture claim (e.g. from Cernere) before issuing per-session credentials.

### 1.3 Capability tokens: Biscuit vs macaroons

- Biscuit (Eclipse project; Rust reference impl https://github.com/eclipse-biscuit/biscuit-rust, crate https://docs.rs/biscuit-auth) is a public-key-signed token with **offline attenuation**: any holder can append a block of Datalog checks to restrict it (resource, expiry) without contacting the issuer; verifiers only need the root public key (spec: https://doc.biscuitsec.org/reference/specifications.html; policies: https://doc.biscuitsec.org/getting-started/authorization-policies.html).
- **Revocation identifiers** are derived from the token structure (one per block); because of offline attenuation, revoking a token must revoke all derived tokens, and the application must store the revocation list itself (https://www.biscuitsec.org/docs/guides/revocation/).
- **Third-party blocks** let an external authority (e.g. Cernere) sign a block that is appended to a token minted elsewhere, with explicit origin scoping of facts (https://www.biscuitsec.org/blog/third-party-blocks-why-how-when-who/).
- Macaroons use HMAC chains from a shared root secret, so every verifier holds the secret; Biscuit uses signatures so private keys need not be distributed (https://secappdev.org/2024/sessions/introduction-to-macaroons).

**Implication:** Cernere mints an authority block (user, org, device posture, exp ≤ hours); Curare attenuates it to "download blobs {hashes} of submission S until T+15min"; the holder hands it to Synergos nodes, which verify with Cernere's public key and check the revocation list pushed by the control plane. This replaces per-node shared HMAC secrets for GET URLs.

### 1.4 Short-lived, scoped credentials instead of long-lived keys

- SPIRE defaults: `default_x509_svid_ttl = 1h`, `default_jwt_svid_ttl = 5m`, rotation pushed by the agent before expiry (https://spiffe.io/docs/latest/deploying/spire_server/, https://github.com/spiffe/spire/blob/v1.9.5/doc/spire_server.md).
- GitHub fine-grained PATs launched with a **maximum lifetime of one year**, org-approvable and revocable (https://devclass.com/2022/10/19/github-fixes-over-broad-token-permissions-with-fine-grained-personal-access-tokens-and-controversial-enforced-expiration); GitHub Actions OIDC tokens live about **5 minutes** and are meant to be exchanged immediately (https://support.spacelift.io/articles/8847614416-why-does-my-github-oidc-token-expire-after-5-minutes-when-using-spacelift-api). Octo STS / GATE exchange a third-party OIDC token for a short-lived GitHub credential so no PAT exists to leak (https://www.chainguard.dev/unchained/the-end-of-github-pats-you-cant-leak-what-you-dont-have, https://pkg.go.dev/github.com/thomsonreuters/gate).

**Implication:** Synergos node identity = SPIFFE-style X.509 SVID (≤1h) from the control plane; custody PUT/GET URLs = minutes, bound to the blob hash and caller identity; CI access to the catalog = OIDC exchange, never stored API keys.

### 1.5 Session-revocation propagation

OpenID CAEP 1.0 (https://openid.net/specs/openid-caep-1_0.html) is a profile of the Shared Signals Framework (https://openid.net/wg/sharedsignals/): transmitters send Security Event Tokens such as **Session Revoked**, identified by subject, with optional reason claims; the WG's motivation is that SAML/OIDC assert validity only at login while sessions last days. Google Workspace ships an SSF receiver whose first supported event is session revocation (https://developers.google.com/workspace/shared-signals/api/ssf-api).

**Implication:** Cernere should be an SSF transmitter; Curare and the Synergos control plane are receivers that, on Session Revoked, add the subject's Biscuit revocation IDs to the list and close QUIC sessions.

### 1.6 Step-up auth and just-in-time admin

Azure/Entra PIM provides time-bound eligible roles; activation can require MFA and (preview) Conditional Access policies (https://learn.microsoft.com/en-us/training/modules/manage-authorization-microsoft-entra-id/15-configure-privileged-identity-management). *No source was found in this session for GitHub "sudo mode" or per-action re-authentication specifics*, so treat "re-auth within N minutes for destructive ops" as an unsourced but standard pattern.

**Implication:** "destroy history", "override retention", "change org policy", "export whole project" in Curare require a fresh step-up assertion from Cernere and a JIT-activated role, logged with the activation reason.

---

## 2. Data protection

### 2.1 Envelope encryption with per-object DEKs

AWS documents SSE-KMS as envelope encryption: KMS generates a **unique data key per object**, returns plaintext + wrapped copies, S3 stores ciphertext and wrapped key together; uploads need `kms:GenerateDataKey`, downloads `kms:Decrypt`; the object ARN is the encryption context, unless S3 Bucket Keys are enabled (then a bucket-level key derives object keys, cutting KMS calls by up to 99 % but widening the context to the bucket ARN) (https://docs.aws.amazon.com/AmazonS3/latest/userguide/UsingKMSEncryption.html, https://docs.aws.amazon.com/kms/latest/developerguide/services-emr.html, https://docs.aws.amazon.com/prescriptive-guidance/latest/aws-kms-best-practices/data-protection-encryption.html). KEK rotation only rewraps DEKs; old key versions remain for decrypt (https://community.exoscale.com/product/security/kms/how-to/rotate-key/).

**Implication:** Synergos blobs are content-addressed by BLAKE3 of plaintext; the ciphertext per history node can use a per-project KEK (KMS/HSM) with a per-blob DEK stored in the manifest; convergent encryption is *not* needed because dedup is per-project anyway. Encryption context should bind `{org, project, blob_hash}` so a key can't be reused across tenants.

### 2.2 Crypto-shredding and secure deletion

Crypto-shredding destroys the key rather than the data; it does not remove plaintext copies that leaked into caches, search indexes or support exports (https://www.encryptionconsulting.com/introduction-to-crypto-shredding/). Per-record DEK wrapped by a per-user/tenant KEK is described as the practical granularity; cloud KMS key destruction is scheduled with a waiting period (https://oneuptime.com/blog/post/2026-02-17-how-to-set-up-crypto-shredding-for-gdpr-right-to-erasure-compliance-in-google-cloud/markdown). NIST SP 800-88 classifies cryptographic erase under *Purge* (relayed via https://community.isc2.org/t5/Tech-Talk/CCSP-Cryptoshredding/m-p/79357 — verify against the NIST text).

**Implication:** "project off-boarding" and "contractor off-boarding" = destroy the project/contractor KEK after the retention lock expires; keep the BLAKE3 manifest so the history graph stays auditable even though blobs are unrecoverable.

### 2.3 Immutability: S3 Object Lock semantics (the reference model)

From AWS docs (https://docs.aws.amazon.com/AmazonS3/latest/userguide/object-lock.html, https://docs.aws.amazon.com/AmazonS3/latest/userguide/object-lock-managing.html, https://docs.aws.amazon.com/AmazonS3/latest/userguide/object-lock-configure.html, https://docs.aws.amazon.com/cli/latest/reference/s3api/put-object-retention.html):

- **Compliance mode**: no user, including the account root, can overwrite/delete a locked version; mode cannot be changed and the period cannot be shortened.
- **Governance mode**: deletable only by principals with `s3:BypassGovernanceRetention` *and* the `x-amz-bypass-governance-retention: true` header (the console sets it by default).
- **Legal hold**: same protection, no expiry, independent of retention; added/removed by `s3:PutObjectLegalHold`.
- Requires **bucket versioning**; locks are per object version; lifecycle expiration cannot delete a locked version; event-based retention duration range 1–36,500 days (1–100 years).

**Implication for history nodes:** emulate this exactly in Synergos: every blob/manifest version carries `{retain_until, mode ∈ {governance, compliance}, legal_hold}`; the daemon refuses physical delete while any lock holds; "delete" from the catalog writes a **tombstone** in the manifest (soft delete) and never touches the blob until retention lapses. *No vendor source for soft-delete/tombstone semantics was found in this session*; the Object Lock version-based model is the closest documented analogue. If MinIO is kept for Curare, verify its Object Lock support from MinIO docs (not retrieved here).

### 2.4 Backup isolation and ransomware resilience

- Veeam's **3-2-1-1-0**: 3 copies, 2 media, 1 off-site, **1 offline/air-gapped/immutable**, **0 errors** after recovery verification (https://bp.veeam.com/vbo/guide/design/3-2-1, https://veeam.com/blog/321-backup-rule.html). Practical immutability options: hardened Linux repository, S3 Object Lock, ejected tape; immutable backups cannot be deleted even by a compromised backup admin (https://community.veeam.com/blogs-and-podcasts-57/the-3-2-1-1-0-rule-in-practice-how-to-actually-implement-it-with-veeam-12799).
- CD Projekt Red 2021 (HelloKitty): devices were encrypted but "our backups remain intact" and the company refused to negotiate (https://threatpost.com/cyberpunk-2077-publisher-hack-ransomware/163775/, https://www.computerweekly.com/news/252496143/HelloKitty-almost-certainly-behind-CD-Projekt-ransomware-attack). Intact backups are what made refusal possible.
- Perforce Helix Core: Microsoft found four unauthenticated flaws (one CVSS 10.0, RCE as LocalSystem) and **over 1,000 Helix Core servers exposed on default port 1666** (https://www.theregister.com/2023/12/19/microsoft_warns_patch_critical_perforce/, https://tenable.com/plugins/nessus/187212). The lesson for a VCS: the server must never be reachable without an identity-aware proxy.

**Implication:** history nodes are the primary; add a separate **immutable replica tier** (different credentials, different org/account, Object-Lock compliance mode) that only the control plane's replication identity can write to and nobody can delete; schedule automated restore drills (the "0").

---

## 3. Integrity & provenance

### 3.1 Transparency logs for receipts (Sigstore Rekor model)

Rekor is a Merkle-tree log; entries carry inclusion proofs and the log periodically signs its tree head with a timestamp (https://docs.sigstore.dev/about/security/, https://edu.chainguard.dev/open-source/sigstore/rekor/an-introduction-to-rekor/). The Sigstore client spec says verifiers MAY verify inclusion proofs per **RFC 9162 §8.1.4** against a signed tree head and SHOULD fetch that head in a way that prevents equivocation, e.g. by requiring signatures from independent **witnesses**; consistency proofs plus witnesses defend against **split-view** attacks by a compromised log (https://github.com/sigstore/architecture-docs/blob/main/client-spec.md). Rekor v2 initial checkpoints may not be witness-cosigned yet (secondary: https://github.com/Luminous-Dynamics/symthaea/issues/3205 — verify).

**Implication:** Synergos custody receipts (SHA-256 verified) should become leaves in a per-org append-only Merkle log run by the control plane; the signed checkpoint is published to at least one independent witness (another region, or a public log). A tampered history node then produces a detectable inconsistency.

### 3.2 Tamper-evident audit logs

Crosby & Wallach (USENIX Security 2009) treat the logger as untrusted and prove membership and consistency with a **history tree** giving O(log n) proofs; hash chains and skip lists need linear-time auditing and cannot audit random subsets; events logged before a compromise stay tamper-evident even if the compromise time is unknown (forward integrity) (https://static.usenix.org/events/sec09/tech/slides/crosby.pdf, https://handouts.secappdev.org/handouts/2016/Dan%20Wallach/tamper-evident-structures.pdf, survey: https://arxiv.org/pdf/2305.01378).

**Implication:** Curare's PostgreSQL audit table gets a Merkle history tree (not just a hash chain) and daily checkpoints are shipped to the immutable replica tier and to the customer (so the vendor cannot rewrite its own audit trail).

### 3.3 C2PA for asset provenance

C2PA 2.x: assertions are wrapped into a signed **Claim**, bound into a **Manifest**; every manifest must have a **hard binding** (hash of the asset bytes, SHA-256 in examples) and may carry soft bindings (fingerprints/watermarks) that survive edits; since 2.0 only X.509 certificates sign (https://c2pa.org/specifications/specifications/2.0/specs/C2PA_Specification.html, https://c2pa.org/specifications/specifications/1.4/guidance/Guidance.html, https://spec.c2pa.org/specifications/specifications/2.1/specs/_attachments/C2PA_Specification.pdf).

**Implication:** for AI-generated or AI-edited assets, Curare can emit a C2PA manifest per release whose hard binding equals the BLAKE3/SHA-256 blob hash, giving customers industry-standard provenance for marketing/legal review. Low priority for breach containment, high for AI-pipeline trust.

### 3.4 SLSA, SBOM, dependency audit

- SLSA v1.0 Build track: L1 = provenance exists (digest + how built; forgeable); L2 = hosted build service signs provenance; L3 = isolated build environments and unforgeable provenance (https://slsa.dev/spec/v1.0/levels, https://slsa.dev/spec/v1.0/requirements).
- SolarWinds SUNSPOT ran *inside the build environment*, swapped a source file during compilation, suppressed build failures, and the result was **signed by SolarWinds' legitimate process** (https://www.crowdstrike.com/blog/sunspot-malware-technical-analysis/, https://krebsonsecurity.com/2021/01/solarwinds-what-hit-us-could-hit-others/). Signing proves the pipeline ran, not that inputs were clean — the control is SLSA L3 isolation + reproducible builds.
- Codecov 2021: an HMAC key leaked from an intermediate layer of the self-hosted Docker image let the attacker modify the Bash Uploader in GCS; the modified script exfiltrated CI **environment variables** for ~2 months; Codecov replaced the script with a compiled binary (https://www.cybersecuritydive.com/news/codecov-bash-uploader-supply-chain-attack/601757/, https://blog.rapid7.com/2021/05/13/rapid7s-response-to-codecov-incident/). Controls: no secrets in image layers; CI egress allowlist; secrets injected per-step, not as global env.
- Rust: `cargo-audit` checks Cargo.lock against the RustSec advisory DB; `cargo-deny` adds `advisories`, `licenses`, `bans`, `sources` sections configured in `deny.toml` (e.g. deny registries/git not explicitly allowed) and runs in CI (https://www.linuxlinks.com/cargo-deny-subcommand/, https://github.com/matrix-org/rustsec-advisory-db, example config https://beto-core.googlesource.com/beto-core/+/refs/heads/main/nearby/deny.toml). *Official cargo-deny docs were not retrieved; verify option names.*

### 3.5 GitHub Actions hardening

- Avoid `pull_request_target` unless privileged context is required; `workflow_run` is the privilege-separation alternative; the hazard is checking out and *then executing* fork code (Makefile, build script, dependency) (https://docs.github.com/en/actions/reference/security/securely-using-pull_request_target, https://docs.github.com/en/actions/reference/security/secure-use). From June 2026 `actions/checkout` blocks fork head/merge refs in `pull_request_target` workflows by default; SHA-pinned workflows must upgrade to get it; opt-out is `allow-unsafe-pr-checkout` (https://github.blog/changelog/2026-06-18-safer-pull_request_target-defaults-for-github-actions-checkout/, https://github.blog/changelog/2025-11-07-actions-pull_request_target-and-environment-branch-protections-changes/).
- Pin third-party actions to a **full-length commit SHA** verified to be from the action's repo, not a fork; repo setting "Require actions to be pinned to a full-length commit SHA" enforces it; the 2026 roadmap adds a `dependencies:` lockfile for direct and transitive action deps (https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/enabling-features-for-your-repository/managing-github-actions-settings-for-a-repository, https://github.blog/news-insights/product-news/whats-coming-to-our-github-actions-2026-security-roadmap/).
- `GITHUB_TOKEN`: default to read-only for contents, raise per job (https://docs.github.com/actions/security-guides/automatic-token-authentication).
- Why it matters: **tj-actions/changed-files (CVE-2025-30066)** — tags v1…v45.0.7 were retroactively repointed to a malicious commit (0e58ed8) on 14–15 Mar 2025; ~23,000 repos affected; payload dumped CI secrets into workflow logs; listed in CISA KEV (https://thehackernews.com/2025/03/github-action-compromise-puts-cicd.html, https://www.sentinelone.com/vulnerability-database/cve-2025-30066, https://www.wiz.io/it-it/blog/github-action-tj-actions-changed-files-supply-chain-attack-cve-2025-30066). Tag pinning would not have helped; SHA pinning would.

### 3.6 Dependency confusion

Birsan (Feb 2021) exploited package managers preferring a higher-versioned public package over a private one; scoped npm packages with the **scope claimed on the public registry** are the primary mitigation; also registry pinning, lockfiles, placeholder packages and CI network restrictions (https://snyk.io/blog/detect-prevent-dependency-confusion-attacks-npm-supply-chain-security/, https://www.csoonline.com/article/3609779/dependency-confusion-explained-another-risk-when-using-open-source-repositories.html). Unity's own blog documents the same risk for Unity Package Manager pulling from npm registries (https://unity.com/blog/engine-platform/upm-dependency-confusion-assetbundle-security-in-the-editor).

**Implication:** Curare's `pnpm` workspace must use a claimed `@scope`, a pinned private registry in `.npmrc`, and `pnpm audit` + lockfile checks in CI; Synergos' `deny.toml` must restrict `sources`.

---

## 4. Blast-radius limits

### 4.1 Tenant isolation

Separate KEKs per org/project (section 2.1/2.2), separate buckets/namespaces, encryption context bound to `{org, project}`, and SPIFFE-style identities per service with ≤1 h certificates (section 1.4). Codecov and LastPass both show that one global credential in one process was enough to reach all customers' data; LastPass's four DevOps engineers held the AWS backup-decryption keys in a corporate vault reachable from a home PC with an unpatched Plex (CVE-2020-5741) (https://palant.info/2023/02/28/lastpass-breach-update-the-few-additional-bits-of-information/, https://9to5mac.com/2023/02/27/lastpass-devops-engineers-hacked/).

### 4.2 SSRF / egress controls (relevant to the control plane's webhooks, Cloudflare relay callbacks and any "fetch asset from URL" feature)

OWASP's SSRF cheat sheet (https://cheatsheetseries.owasp.org/cheatsheets/Server_Side_Request_Forgery_Prevention_Cheat_Sheet.html) and summaries (https://www.wiz.io/api/md/academy/application-security/server-side-request-forgery, https://answers.securityscientist.net/q/25866/what-defenses-actually-stop-ssrf): allowlist, not denylist (127.0.0.1 has too many encodings); **validate the resolved IP, not only the hostname**, against DNS rebinding (0-TTL domain flips to 127.0.0.1 on the real request); block loopback, RFC1918 and **169.254.0.0/16** (cloud metadata); **disable redirects or re-validate every hop** (302 to a private IP is the canonical bypass); restrict schemes to http/https; require IMDSv2 on AWS.

### 4.3 Sandboxing hooks / plugins / importers

- gVisor: a user-space application kernel (Sentry) intercepts syscalls; it runs under its own seccomp filter and file access goes through an external agent that does not trust the Sentry; Systrap is the default platform, KVM is faster in some cases but slower under nested virtualisation (https://gvisor.dev/docs/architecture_guide/intro/, https://gvisor.dev/blog/2024/02/01/seccomp/, https://northflank.com/blog/what-is-gvisor). With seccomp alone the workload is "one system call away from host compromise" and policies are hand-written.
- Bubblewrap: namespace + seccomp based, cheap, host-kernel attack surface (only a thin secondary source was found; verify from bubblewrap docs). *Deno permissions and WASI capability sandboxes: no source found in this session.*

**Implication:** any Synergos hook, Curare review-time transcoder, thumbnailer or format validator that parses untrusted asset bytes runs in gVisor (or at minimum bubblewrap + seccomp) with no network, a read-only input mount, a write-only output mount and a CPU/memory/time budget.

### 4.4 Archive extraction and path traversal

- `decompress` npm **CVE-2026-10732**: two entries with the same path, first a symlink to an arbitrary target, second a regular file, bypass all prior protections including the CVE-2020-12265 fix because of micro-task ordering (https://stack.watch/vuln/CVE-2026-10732/, https://osv.dev/vulnerability/GHSA-94vh-gphv-8pm8).
- Rust `zip` crate **CVE-2025-29787**: symlinks created earlier in an archive were followed by later entries; `enclosed_name` alone was insufficient; the usual fix is to refuse out-of-tree symlinks or **defer symlink creation until all regular files are written** (https://www.miggo.io/vulnerability-database/cve/CVE-2025-29787).
- Android guidance: canonicalise every entry's target and verify it is a child of the destination, and assume the destination is writable only by the app (https://developer.android.google.cn/topic/security/risks/zip-path-traversal?hl=nl).

**Implication:** Synergos never extracts archives on history nodes; if Curare extracts `.unitypackage`/zip for preview, it does so in the sandbox with symlinks rejected outright.

### 4.5 File-type validation (OWASP ASVS 5.0 chapter V5)

ASVS 5.0 renumbered everything (cite as `v5.0.0-5.2.x`) (https://github.com/OWASP/ASVS/tree/master/5.0/en, https://github.com/OWASP/ASVS/issues/2996). V5.2.1: only accept sizes the app can process (L1–L3); V5.2.2: extension must match content via magic bytes/validation libraries (L1 for decision-relevant files, L2+ for all); V5.2.3–5.2.6 (L2–L3): compressed-file limits, quotas, **symlinks in archives**, image pixel caps against pixel-flood (https://cornucopia.owasp.org/taxonomy/asvs-5.0/05-file-handling/02-file-upload-and-content, https://cornucopia.owasp.org/taxonomy/asvs-5.0/level-2-controls). ASVS 4.0 V12.4 required storing untrusted files outside the web root and scanning them with antivirus (https://owasp-aasvs4.readthedocs.io/en/latest/V12.4.html). *ClamAV specifics and polyglot-file literature were not sourced in this session.*

### 4.6 Executable asset formats (the game-pipeline-specific hazard)

- **Maya scriptNodes** execute on scene open: CVE-2009-3578 (CVSS 9.3, Maya 6.5–2010) with Autodesk's own mitigation "disable MEL/python commands in .ma/.mb" (https://www.coresecurity.com/node/103356, https://cvedb.shodan.io/dashboard/cve/CVE-2009-3578).
- Current worm family **MEL/Vacphage.A** ("Vaccine"): a scriptNode writes a payload into `userSetup.mel` / overwrites `userSetup.py` so every subsequent scene is infected; ESET has detected it since Jan 2021; in 2025 **39 sellers on Epic's Fab.com unknowingly distributed infected Maya files**; studios report it "often coming from outsourcers" (https://av-comparatives.org/maya-malware-fab-com/, https://www.tech-artists.org/t/another-maya-malware-in-the-wild/12970, https://twitter.com/Red9_Anim/status/1389854605656342528). Autodesk ships a "Security Tools" plug-in and a cleanup procedure (https://forums.autodesk.com/t5/maya-forum/maya-malicious-script-security-update-and-resources/td-p/9597247, https://www.autodesk.com/support/technical/article/caas/sfdcarticles/sfdcarticles/How-to-clear-Maya-ScriptExploit-using-a-Mel-Script.html). **The names "Phoenix" and "KiBoom" returned no results**; Vacphage/Vaccine is the documented family.
- **Blender**: Python auto-execution is off by default; triggers are text blocks with *Register* and animation drivers; a dialog asks to allow/ignore; "Trusted Source" paths and `-Y/--disable-autoexec` exist (https://docs.blender.org/manual/nl/5.2/advanced/scripting/security.html).
- **Unity**: Jan 2023 advisory — Editor FBX and SketchUp importers had memory-corruption bugs leading to RCE (https://unity.com/security/jan-2023-01); a community tool warns that scripts/DLLs inside a `.unitypackage` can run during import (weak source: https://booth.pm/ko/items/8335990).
- **FBX SDK ≤2020.0**: CVE-2020-7080…7085 — buffer overflow, type confusion, UAF, integer overflow, heap overflow; "a user may be tricked into opening a malicious FBX file which … could run arbitrary code" (https://threatprotect.qualys.com/2020/04/23/autodesk-fbx-sdk-multiple-vulnerabilities-cve-2020-7080cve-2020-7081cve-2020-7082cve-2020-7083cve-2020-7084cve-2020-7085/, https://tenable.com/cve/CVE-2020-7080).

**Implication:** a *format-aware static scanner* at submission time is a real differentiator: parse `.ma` text for `scriptNode`/`userSetup` writes, flag `.mb` script nodes, flag `.blend` text blocks with Register and drivers with Python expressions, flag `.unitypackage` entries ending in `.cs`/`.dll`/`Editor/`, and quarantine with a review step. Run every binary parser (FBX, images) only inside the sandbox from 4.3. *Memory-safety statistics and cargo-fuzz documentation were not sourced in this session*; the Rust daemon should still fuzz its manifest/QUIC frame parsers.

---

## 5. Detection & response

### 5.1 Behavioural anomaly detection

Microsoft Defender for Cloud Apps documents two relevant detections: **impossible travel** (two activities from distant locations faster than travel allows; ML filters VPN/common locations; **7-day learning period per user**; on by default) and **mass download** (a custom Activity policy: user downloads a massive number of files in a short period; exclude Azure-tagged IPs; filter on download activity types) (https://learn.microsoft.com/en-us/defender-cloud-apps/policies-threat-protection).

**Implication:** the control plane already sees every GET receipt; compute per-identity rolling baselines (bytes/hour, distinct projects/hour, new-device flag, geo) and emit SIEM events; auto-throttle or require step-up when >N× baseline. The GitHub 2022 campaign was noticed because a customer spotted "suspicious GitHub OAuth activity" (https://www.malwarebytes.com/blog/news/2023/01/circleci-malware-stole-github-oauth-keys-bypassing-2fa); the GTA6 attacker downloaded videos straight from Slack — a volume anomaly.

### 5.2 Honeytokens / canary assets

Canarytokens (Thinkst) create decoy AWS keys, Word/PDF/kubeconfig files, URLs and DNS names bound to a memo and an alert channel; any touch is a high-signal alert (https://zeltser.com/honeytokens-canarytokens-setup, https://zeltser.com/plant-honeytokens). Caveat: TruffleHog showed canarytokens.org AWS keys can be identified statically from the account ID; self-hosted/paid tokens avoid this (https://trufflesecurity.com/canaries).

**Implication:** seed each project with a few canary assets ("`final_boss_cutscene_v9.mov`") whose GET fires an alert; seed node config with a canary custody URL.

### 5.3 Kill switch / global revocation

Documented primitives: Biscuit revocation-ID lists (1.3), CAEP Session Revoked (1.5), GitHub revoking all affected OAuth tokens in April 2022 (https://github.blog/news-insights/company-news/security-alert-stolen-oauth-user-tokens/), Okta revoking session tokens embedded in exposed HAR files (https://nightfall.ai/blog/okta-data-breach-what-happened-impact-and-security-lessons-learned).

**Implication:** the Synergos control plane publishes a signed, monotonically versioned **revocation bundle** (identities, Biscuit revocation IDs, node IDs) that every node must fetch at ≤60 s intervals and **fail closed** if stale beyond a configurable window; "quarantine node" = add node ID to the bundle + rotate the project KEK wrapping for new DEKs.

### 5.4 Incident guidance and log retention

CISA's #StopRansomware Rhysida advisory AA23-319A (updated 30 Apr 2025) tells organisations to enable MFA for all services, particularly webmail, VPN and accounts that access critical systems; secondary analysis lists initial access as valid credentials against MFA-less VPNs, phishing and Zerologon (https://www.cisa.gov/news-events/cybersecurity-advisories/aa23-319a, https://cybelangel.com/blog/rhysida-ransomware-explained-ttps-iocs-and-how-to-defend-in-2026/). *NIST SP 800-61r3 playbooks and concrete audit-log retention numbers (PCI/ISO/SOC 2) were not sourced in this session.*

---

## 6. Lessons from real incidents

| Incident | Vector (as reported) | Control that would have limited it | Sources |
|---|---|---|---|
| Rockstar / GTA6 (Sep 2022, Lapsus$) | Phishing of a Rockstar India employee working from home; browser data logged; attacker reached Slack and downloaded dev videos and claimed GTA5/6 source | Device posture + identity-aware proxy for Slack/VCS; short sessions bound to device; download-volume anomaly alerts; step-up for bulk export | https://80.lv/articles/gta-6-files-were-allegedly-leaked-because-of-phishing-email, https://infosecurity-magazine.com/news/gta-publisher-rockstar-games-hacked, https://www.polymerhq.io/blog/breach/how-did-slack-led-to-grand-theft-auto-leak/, https://www.techradar.com/gaming/teenager-involved-with-hack-which-led-to-gta-6-leak-court-finds |
| Insomniac Games (Dec 2023, Rhysida) | Entry unconfirmed (phishing suspected); group claims domain admin in 20–25 min; 1.67 TB / 1.3 M files leaked after 50 BTC demand refused; passports and design docs included | Segmented identity (asset platform not trusting AD domain admin), immutable history tier, per-project KEKs, exfil-volume alerting | https://Techraptor.net/gaming/news/insomniac-games-ransomware-attack-report, https://www.twingate.com/blog/tips/Insomniac%20Games-data-breach, https://80.lv/articles/spider-man-2-dev-was-breached-by-hackers/, https://esports.gg/news/gaming/hackers-release-stolen-data-from-sonys-insomniac-studios |
| Capcom (Nov 2020, Ragnar Locker) | Old backup VPN device at Capcom USA kept as emergency fallback; 1 TB claimed stolen; ~15,649 individuals' data exposed | Inventory and decommission legacy edges; ZTNA instead of VPN; MFA; no standing network trust | https://www.watchguard.com/wgrd-news/blog/capcoms-vpn-entry-point-ransomware, https://www.scworld.com/brief/capcom-blames-old-vpn-device-for-2020-ransomware-attack-2, https://www.itsecurityguru.org/2021/04/14/capcom-release-final-update-on-ransomware-attack/ |
| CD Projekt Red (Feb 2021, HelloKitty) | Network encryption + source-code theft (Cyberpunk, Witcher 3, Gwent); backups intact; no negotiation; second attack after 2017 | Immutable/offline backups (worked); add exfil controls since theft still happened | https://threatpost.com/cyberpunk-2077-publisher-hack-ransomware/163775/, https://www.computerweekly.com/news/252496143/HelloKitty-almost-certainly-behind-CD-Projekt-ransomware-attack, https://www.incibe.es/index.php/en/incibe-cert/publications/cybersecurity-highlights/new-cyberattack-against-video-game-industry |
| Ubisoft & Crytek (Oct 2020, Egregor) | Entry unconfirmed (phishing suspected); 20 MB Ubisoft / 300 MB Crytek; later ~560 GB Watch Dogs Legion archive on trackers | Same as above; per-project isolation so one foothold does not expose whole engine | https://grahamcluley.com/following-ubisoft-cyber-attack-hackers-claim-to-leak-watch-dogs-legion-code-online/, https://techraptor.net/gaming/news/ransomware-gang-leaks-data-from-ubisoft-and-crytek |
| Nintendo "Gigaleak" (2018–2020) | Zammis Clark hacked internal network Mar 2018; BroadOn (contractor) servers suspected source of Wii/iQue material | Contractor/outsourcer isolation; crypto-shred contractor KEKs at contract end; retention of decades-old assets under lock, not on reachable servers | https://en.wikipedia.org/wiki/Nintendo_data_leak, https://www.videogameschronicle.com/news/nintendo-says-it-increased-security-following-a-prolific-leak-of-game-prototypes |
| Okta support system (Sep–Oct 2023) | Service-account credentials saved in an employee's personal Google account on a managed laptop; HAR files with session tokens for 134 customers; 5 sessions hijacked | No service accounts with passwords; short-lived sessions; token binding; CAEP revocation; scrub diagnostics | https://nightfall.ai/blog/okta-data-breach-what-happened-impact-and-security-lessons-learned, https://www.bleepingcomputer.com/news/security/okta-breach-134-customers-exposed-in-october-support-system-hack/amp/, https://thehackernews.com/2023/11/oktas-recent-customer-support-data.html |
| LastPass (Aug–Oct 2022) | DevOps engineer's home PC via unpatched Plex (CVE-2020-5741) → keylogger → master password after MFA → corporate vault holding AWS backup decryption keys (only 4 people had them) | Device posture (no personal devices); JIT access to backup keys; split keys/HSM; per-shard KEKs | https://palant.info/2023/02/28/lastpass-breach-update-the-few-additional-bits-of-information/, https://9to5mac.com/2023/02/27/lastpass-devops-engineers-hacked/, https://www.cpomagazine.com/?p=30705 |
| CircleCI (Dec 2022–Jan 2023) | Infostealer on an engineer's laptop stole a 2FA-backed SSO session cookie; engineer could mint production tokens; encryption keys pulled from a running process | Short session TTL + device-bound tokens; JIT production access; HSM-held keys; alert on unusual token minting | https://circleci.com/blog/jan-4-2023-incident-report, https://www.malwarebytes.com/blog/news/2023/01/circleci-malware-stole-github-oauth-keys-bypassing-2fa |
| GitHub OAuth token theft (Apr 2022) | OAuth user tokens issued to Heroku/Travis CI stolen (Heroku breach); dozens of orgs' private repos cloned; npm AWS key found in a cloned repo | Scoped, short-lived integration tokens; secret scanning of repos; anomaly on org-wide clone bursts | https://github.blog/news-insights/company-news/security-alert-stolen-oauth-user-tokens/, https://threats.wiz.io/all-incidents/github-npm-breach, https://securityaffairs.com/posts/github-warns-stolen-oauth-tokens-access-data |
| Codecov (Jan–Apr 2021) | Credential in Docker image layer → modified Bash Uploader → CI env vars exfiltrated for ~2 months | No secrets in images; signed/pinned uploaders; CI egress allowlist | https://www.cybersecuritydive.com/news/codecov-bash-uploader-supply-chain-attack/601757/, https://blog.rapid7.com/2021/05/13/rapid7s-response-to-codecov-incident/ |
| SolarWinds SUNSPOT (2019–2020) | Implant in build environment swapped a source file during compile; output legitimately signed | SLSA L3 isolated builds; reproducible builds; compare built output with VCS | https://www.crowdstrike.com/blog/sunspot-malware-technical-analysis/, https://krebsonsecurity.com/2021/01/solarwinds-what-hit-us-could-hit-others/ |
| tj-actions/changed-files (Mar 2025) | Tags repointed to malicious commit; secrets printed to logs in ~23k repos | Full-SHA pinning; read-only GITHUB_TOKEN; secrets masked and scoped per job | https://thehackernews.com/2025/03/github-action-compromise-puts-cicd.html, https://www.sentinelone.com/vulnerability-database/cve-2025-30066 |

---

## 7. Standards and what enterprise game customers will ask for

- **NIST SP 800-207 / 800-207A / CSWP 20**: use as the architecture narrative (PDP/PEP, per-session, posture). URLs in 1.1.
- **OWASP ASVS 5.0**: cite version-qualified IDs; chapter V5 File Handling (V5.2 upload/content, V5.3 storage, V5.4 download) is the checklist for the asset ingest path (https://owasp.org/projects/asvs, https://github.com/OWASP/ASVS/tree/master/5.0/en, https://cheatsheetseries.owasp.org/IndexASVS.html).
- **MPA Content Security Best Practices via TPN**: current version **v5.3.1** (https://www.ttpn.org/links-resources/); TPN+ assessment tiers are now **Blue (self-assessed), Silver (third-party assessed with remediation plan), Gold (all Best-Practice remediation reviewed by TPN), Gold Star (Best Practices + Additional Recommendations)**, replacing classic Blue/Gold; Gold is a baseline for each content owner's own risk decision, not a pass/fail (https://www.ttpn.org/press-releases/tpn-to-launch-enhanced-shield-tier-system-to-strengthen-global-content-security/, https://www.ttpn.org/2024/07/tpn-assessment-what-you-need-to-know/, https://www.ttpn.org/wp-content/uploads/2025/11/TPNplus-Assessor-How-To-Guide_v5.3.1_Nov25.pdf). TPN added hardening guidelines for **software application providers** — i.e. SaaS vendors like this platform are in scope (https://www.ttpn.org/press-releases/tpn-updates-mpa-best-practices-to-include-hardening-guidelines-for-software-application-providers/). Vendor summaries state the CSBP maps to ISO/IEC 27001:2022, 27002:2022, NIST SP 800-53 r5 and SOC 2 TSC (https://massive.io/?p=126187, https://www.securityscientist.net/blog/mpa-content-security-standard/).
- **NIST SP 800-53 control families and CIS Controls v8: no source retrieved in this session**; the TPN mapping above is the practical bridge to 800-53.
- **What a SaaS asset platform must show** (inference from the above, not a sourced list): SSO + enforced MFA + device posture; per-tenant encryption keys with customer-visible rotation and crypto-shred on off-boarding; immutable retention and documented backup isolation with restore evidence; exportable tamper-evident audit logs; a TPN Silver/Gold assessment or ISO 27001 certificate; a malware/script-node scanning statement for Maya/Blender/Unity/FBX; incident-response SLAs and a token kill switch.

---

## 8. Prioritised catalogue of mechanisms

Cost scale (engineering judgement): **S** < 2 person-weeks, **M** 2–6, **L** > 6. "Rust" = Synergos daemon/control plane; "TS" = Curare/Hono.

| # | Mechanism | What it stops | Cost | Prerequisites | Sources |
|---|---|---|---|---|---|
| P0-1 | **Short-lived, scoped Biscuit capability tokens** minted by Cernere, attenuated by Curare, verified offline by nodes; revocation-ID list distributed by control plane | Stolen session/token reuse (Okta, CircleCI, GitHub 2022); over-broad node keys | M (Rust: `biscuit-auth`; TS: biscuit JS/WASM or server-side attenuation) | Cernere publishes root public key; control plane revocation feed | 1.3, 5.3 |
| P0-2 | **Signed revocation bundle + fail-closed nodes** (≤60 s refresh; stale ⇒ deny) and CAEP receiver in Cernere→Curare/control plane | Lateral movement after a credential theft; lets ops "kill" a user/device/node in seconds | S–M (Rust: fetch+verify loop; TS: SSF receiver endpoint) | P0-1; Cernere as SSF transmitter | 1.5, 5.3 |
| P0-3 | **Object-Lock-style retention on history nodes + tombstone soft delete + immutable replica tier** (separate identity/account, compliance mode) | Ransomware destruction of history; insider/admin deletion; the "backups intact" outcome of CDPR | M (Rust: lock metadata + delete gate + replicator) | Second storage account/region; restore-drill automation | 2.3, 2.4 |
| P0-4 | **Per-project envelope encryption** (DEK per blob in manifest, KEK per project in KMS/HSM, encryption context `{org,project,hash}`) + crypto-shred on off-boarding | Cross-tenant exposure from one leaked key; outsourcer retention (Nintendo/BroadOn); unrecoverable deletion while keeping manifests | M (Rust: AEAD + KMS client; TS: key lifecycle API) | KMS/HSM; key-rotation runbook | 2.1, 2.2 |
| P0-5 | **Identity-aware proxy with device posture** on relay/web (Cloudflare Access + WARP or mTLS); posture claim required by control plane for P2P sessions | Personal/unmanaged devices (LastPass, Rockstar, Okta) | S (config) + S (Rust: posture claim check) | Managed-device enrollment; Enterprise plan for mTLS enrollment gating | 1.2 |
| P0-6 | **Custody URL hardening**: minutes-TTL, bound to blob hash + caller identity + method, single-use nonce, receipts logged | Replay of leaked PUT/GET URLs; silent overwrite | S (Rust) | P0-1 | 1.4, 3.1 |
| P0-7 | **CI/supply-chain hygiene**: full-SHA pinned actions, read-only `GITHUB_TOKEN`, no `pull_request_target`, `cargo-deny` (advisories+sources+licenses), `pnpm audit` + claimed npm scope + pinned registry | tj-actions-style tag hijack; dependency confusion; vulnerable crates | S | None | 3.4–3.6 |
| P1-1 | **Asset ingest validation per ASVS V5.2** + **format-aware script-node scanner** (Maya scriptNode/userSetup, Blender Register/driver, `.unitypackage` scripts/DLLs) + quarantine/review state in Curare | Vacphage-type worms spreading through the asset store; outsourcer-introduced malware | M (TS: parsers for `.ma`, zip manifests; Rust optional) | Submission state machine supports "quarantined" | 4.5, 4.6 |
| P1-2 | **Sandboxed parsers/hooks** (gVisor; fallback bubblewrap+seccomp; no network, RO input/WO output, resource limits) for FBX/image/preview and any user hook | Importer RCE (Unity Jan 2023, FBX SDK CVEs) becoming a server compromise | M | Container runtime with gVisor (`runsc`) | 4.3 |
| P1-3 | **Merkle transparency log for receipts + history-tree audit log**, checkpoints witnessed by a second party and exported to customers | Silent tampering by a compromised node or by the vendor; proves what was delivered to whom | M–L (Rust: RFC 9162 tree; TS: audit tree + checkpoint export) | Stable receipt schema | 3.1, 3.2 |
| P1-4 | **Exfiltration analytics**: per-identity/device baselines on bytes, blob count, projects, geo; impossible-travel; auto-throttle + step-up; SIEM export | Bulk dumps (Insomniac 1.67 TB, GTA6 Slack downloads, GitHub org clones) | M (control plane stream processor) | Receipts carry identity/device/geo; SIEM | 5.1 |
| P1-5 | **Canary assets and canary custody URLs** per project | Early, near-zero-false-positive breach signal | S | Alerting channel | 5.2 |
| P1-6 | **Step-up + JIT admin** for destructive/bulk ops (destroy history, override retention, org export, KEK destroy) | Abuse of standing admin (CircleCI token minting, LastPass vault) | S–M (TS: policy + Cernere assertion check) | Cernere supports re-auth assertions/role activation | 1.6 |
| P1-7 | **SSRF/egress allowlist** for every server-side fetch and webhook; resolved-IP validation; no redirects; block 169.254/16; CI egress allowlist | Cloud-metadata credential theft; Codecov-style env exfil | S | Outbound proxy | 4.2 |
| P2-1 | **Zip/archive safety**: never extract on nodes; sandboxed extraction with symlink rejection and canonical-path checks | Zip-slip (CVE-2026-10732, CVE-2025-29787) | S | P1-2 | 4.4 |
| P2-2 | **SLSA L3 builds + reproducible Rust binaries + signed releases (Sigstore)** for the daemon and Curare images | SolarWinds-class build-time implant; tampered daemon updates | M | Hosted isolated builder | 3.4 |
| P2-3 | **C2PA manifests on release** (hard binding = blob hash) | Provenance disputes for AI-generated assets | S–M (TS) | X.509 signing cert | 3.3 |
| P2-4 | **Fuzzing of manifest/QUIC/receipt parsers** (cargo-fuzz) — *unsourced in this session* | Memory/logic bugs in the Rust parsing surface | S | CI minutes | — |
| P2-5 | **3-2-1-1-0 restore drills** with evidence retained | Backups that exist but don't restore | S | P0-3 | 2.4 |

### Prerequisite chain

Cernere must (1) publish a root public key and mint Biscuit authority blocks with device-posture claims, (2) act as an SSF/CAEP transmitter, (3) offer step-up/JIT assertions. Everything in P0 except P0-3 and P0-7 depends on (1); P0-2 and P1-6 depend on (2)/(3). P0-3 and P0-4 are independent and should start immediately because they are the controls that turned CD Projekt's attack into an inconvenience rather than a loss.

---

## 9. Gaps in sources (not found in this session)

- Autodesk advisory or vendor report naming "Phoenix" or "KiBoom" Maya malware (only MEL/Vacphage.A found).
- Deno permission model and WASI capability sandboxing documentation.
- cargo-fuzz documentation and memory-safety vulnerability statistics.
- ClamAV scanning docs and polyglot-file research.
- NIST SP 800-61 Rev 3 playbooks; NIST SP 800-53 family text; CIS Controls v8.
- Concrete audit-log retention requirements (PCI DSS/ISO 27001/SOC 2).
- Vendor documentation for soft delete / tombstones (Azure, GCS) and MinIO Object Lock.
- GitHub "sudo mode" / per-action re-authentication documentation.
- Primary pages for Crosby–Wallach paper, OWASP SSRF cheat sheet body, Sigstore Rekor docs, Biscuit root doc, AWS Object Lock page and NIST 800-207 body were not fetched directly (proxy refused); claims come from search excerpts of those URLs.
- Ubisoft 2022 incident; Unreal Engine asset code-execution risks.

---

## Sources

Identity & access
- https://nvlpubs.nist.gov/nistpubs/specialpublications/NIST.SP.800-207.pdf
- https://nvlpubs.nist.gov/nistpubs/SpecialPublications/NIST.SP.800-207A.pdf
- https://nvlpubs.nist.gov/nistpubs/CSWP/NIST.CSWP.20.pdf
- https://csrc.nist.gov/CSRC/media/Presentations/zero-trust-networks-brief/images-media/2-4Kerman%20-%20Zero%20Trust%20Architecture%20-%20NCCoE%20-%202019%20-%20ISPAB.pdf
- https://developers.cloudflare.com/cloudflare-one/reusable-components/posture-checks/client-checks/require-warp/
- https://developers.cloudflare.com/cloudflare-one/access-controls/policies/
- https://developers.cloudflare.com/reference-architecture/design-guides/designing-ztna-access-policies/
- https://developers.cloudflare.com/cloudflare-one/identity/devices/warp-client-checks/
- https://developers.cloudflare.com/learning-paths/replace-vpn/configure-device-agent/device-enrollment-permissions/
- https://tailscale.com/docs/features/access-control/grants.md
- https://tailscale.com/docs/reference/grants-vs-acls.md
- https://tailscale.com/kb/1018-acls
- https://doc.biscuitsec.org/reference/specifications.html
- https://doc.biscuitsec.org/getting-started/authorization-policies.html
- https://www.biscuitsec.org/docs/guides/revocation/
- https://www.biscuitsec.org/blog/third-party-blocks-why-how-when-who/
- https://docs.rs/biscuit-auth
- https://github.com/eclipse-biscuit/biscuit-rust
- https://secappdev.org/2024/sessions/introduction-to-macaroons
- https://spiffe.io/docs/latest/deploying/spire_server/
- https://github.com/spiffe/spire/blob/v1.9.5/doc/spire_server.md
- https://openid.net/specs/openid-caep-1_0.html
- https://openid.net/wg/sharedsignals/
- https://developers.google.com/workspace/shared-signals/api/ssf-api
- https://learn.microsoft.com/en-us/training/modules/manage-authorization-microsoft-entra-id/15-configure-privileged-identity-management
- https://devclass.com/2022/10/19/github-fixes-over-broad-token-permissions-with-fine-grained-personal-access-tokens-and-controversial-enforced-expiration
- https://github.com/orgs/community/discussions/184161
- https://www.chainguard.dev/unchained/the-end-of-github-pats-you-cant-leak-what-you-dont-have
- https://support.spacelift.io/articles/8847614416-why-does-my-github-oidc-token-expire-after-5-minutes-when-using-spacelift-api
- https://pkg.go.dev/github.com/thomsonreuters/gate

Data protection
- https://docs.aws.amazon.com/AmazonS3/latest/userguide/object-lock.html
- https://docs.aws.amazon.com/AmazonS3/latest/userguide/object-lock-managing.html
- https://docs.aws.amazon.com/AmazonS3/latest/userguide/object-lock-configure.html
- https://docs.aws.amazon.com/cli/latest/reference/s3api/put-object-retention.html
- https://docs.aws.amazon.com/AmazonS3/latest/userguide/UsingKMSEncryption.html
- https://docs.aws.amazon.com/kms/latest/developerguide/services-emr.html
- https://docs.aws.amazon.com/prescriptive-guidance/latest/aws-kms-best-practices/data-protection-encryption.html
- https://community.exoscale.com/product/security/kms/how-to/rotate-key/
- https://www.encryptionconsulting.com/introduction-to-crypto-shredding/
- https://oneuptime.com/blog/post/2026-02-17-how-to-set-up-crypto-shredding-for-gdpr-right-to-erasure-compliance-in-google-cloud/markdown
- https://community.isc2.org/t5/Tech-Talk/CCSP-Cryptoshredding/m-p/79357
- https://bp.veeam.com/vbo/guide/design/3-2-1
- https://veeam.com/blog/321-backup-rule.html
- https://community.veeam.com/blogs-and-podcasts-57/the-3-2-1-1-0-rule-in-practice-how-to-actually-implement-it-with-veeam-12799
- https://www.theregister.com/2023/12/19/microsoft_warns_patch_critical_perforce/
- https://tenable.com/plugins/nessus/187212

Integrity & provenance
- https://docs.sigstore.dev/about/security/
- https://github.com/sigstore/architecture-docs/blob/main/client-spec.md
- https://edu.chainguard.dev/open-source/sigstore/rekor/an-introduction-to-rekor/
- https://github.com/Luminous-Dynamics/symthaea/issues/3205
- https://static.usenix.org/events/sec09/tech/slides/crosby.pdf
- https://handouts.secappdev.org/handouts/2016/Dan%20Wallach/tamper-evident-structures.pdf
- https://arxiv.org/pdf/2305.01378
- https://c2pa.org/specifications/specifications/2.0/specs/C2PA_Specification.html
- https://c2pa.org/specifications/specifications/1.4/guidance/Guidance.html
- https://spec.c2pa.org/specifications/specifications/2.1/specs/_attachments/C2PA_Specification.pdf
- https://slsa.dev/spec/v1.0/levels
- https://slsa.dev/spec/v1.0/requirements
- https://www.crowdstrike.com/blog/sunspot-malware-technical-analysis/
- https://krebsonsecurity.com/2021/01/solarwinds-what-hit-us-could-hit-others/
- https://www.cybersecuritydive.com/news/codecov-bash-uploader-supply-chain-attack/601757/
- https://blog.rapid7.com/2021/05/13/rapid7s-response-to-codecov-incident/
- https://www.linuxlinks.com/cargo-deny-subcommand/
- https://github.com/matrix-org/rustsec-advisory-db
- https://beto-core.googlesource.com/beto-core/+/refs/heads/main/nearby/deny.toml
- https://docs.github.com/en/actions/reference/security/securely-using-pull_request_target
- https://docs.github.com/en/actions/reference/security/secure-use
- https://github.blog/changelog/2026-06-18-safer-pull_request_target-defaults-for-github-actions-checkout/
- https://github.blog/changelog/2025-11-07-actions-pull_request_target-and-environment-branch-protections-changes/
- https://github.blog/news-insights/product-news/whats-coming-to-our-github-actions-2026-security-roadmap/
- https://docs.github.com/actions/security-guides/automatic-token-authentication
- https://docs.github.com/en/repositories/managing-your-repositorys-settings-and-features/enabling-features-for-your-repository/managing-github-actions-settings-for-a-repository
- https://thehackernews.com/2025/03/github-action-compromise-puts-cicd.html
- https://www.sentinelone.com/vulnerability-database/cve-2025-30066
- https://www.wiz.io/it-it/blog/github-action-tj-actions-changed-files-supply-chain-attack-cve-2025-30066
- https://snyk.io/blog/detect-prevent-dependency-confusion-attacks-npm-supply-chain-security/
- https://www.csoonline.com/article/3609779/dependency-confusion-explained-another-risk-when-using-open-source-repositories.html
- https://unity.com/blog/engine-platform/upm-dependency-confusion-assetbundle-security-in-the-editor

Blast-radius limits
- https://cheatsheetseries.owasp.org/cheatsheets/Server_Side_Request_Forgery_Prevention_Cheat_Sheet.html
- https://www.wiz.io/api/md/academy/application-security/server-side-request-forgery
- https://answers.securityscientist.net/q/25866/what-defenses-actually-stop-ssrf
- https://gvisor.dev/docs/architecture_guide/intro/
- https://gvisor.dev/blog/2024/02/01/seccomp/
- https://northflank.com/blog/what-is-gvisor
- https://stack.watch/vuln/CVE-2026-10732/
- https://osv.dev/vulnerability/GHSA-94vh-gphv-8pm8
- https://www.miggo.io/vulnerability-database/cve/CVE-2025-29787
- https://developer.android.google.cn/topic/security/risks/zip-path-traversal?hl=nl
- https://owasp.org/projects/asvs
- https://github.com/OWASP/ASVS/tree/master/5.0/en
- https://github.com/OWASP/ASVS/issues/2996
- https://cornucopia.owasp.org/taxonomy/asvs-5.0/05-file-handling/02-file-upload-and-content
- https://cornucopia.owasp.org/taxonomy/asvs-5.0/level-2-controls
- https://owasp-aasvs4.readthedocs.io/en/latest/V12.4.html
- https://cheatsheetseries.owasp.org/IndexASVS.html
- https://www.coresecurity.com/node/103356
- https://cvedb.shodan.io/dashboard/cve/CVE-2009-3578
- https://av-comparatives.org/maya-malware-fab-com/
- https://www.tech-artists.org/t/another-maya-malware-in-the-wild/12970
- https://twitter.com/Red9_Anim/status/1389854605656342528
- https://forums.autodesk.com/t5/maya-forum/maya-malicious-script-security-update-and-resources/td-p/9597247
- https://www.autodesk.com/support/technical/article/caas/sfdcarticles/sfdcarticles/How-to-clear-Maya-ScriptExploit-using-a-Mel-Script.html
- https://docs.blender.org/manual/nl/5.2/advanced/scripting/security.html
- https://unity.com/security/jan-2023-01
- https://booth.pm/ko/items/8335990
- https://threatprotect.qualys.com/2020/04/23/autodesk-fbx-sdk-multiple-vulnerabilities-cve-2020-7080cve-2020-7081cve-2020-7082cve-2020-7083cve-2020-7084cve-2020-7085/
- https://tenable.com/cve/CVE-2020-7080

Detection & response
- https://learn.microsoft.com/en-us/defender-cloud-apps/policies-threat-protection
- https://zeltser.com/honeytokens-canarytokens-setup
- https://zeltser.com/plant-honeytokens
- https://trufflesecurity.com/canaries
- https://www.cisa.gov/news-events/cybersecurity-advisories/aa23-319a
- https://cybelangel.com/blog/rhysida-ransomware-explained-ttps-iocs-and-how-to-defend-in-2026/

Incidents
- https://80.lv/articles/gta-6-files-were-allegedly-leaked-because-of-phishing-email
- https://infosecurity-magazine.com/news/gta-publisher-rockstar-games-hacked
- https://www.polymerhq.io/blog/breach/how-did-slack-led-to-grand-theft-auto-leak/
- https://www.techradar.com/gaming/teenager-involved-with-hack-which-led-to-gta-6-leak-court-finds
- https://Techraptor.net/gaming/news/insomniac-games-ransomware-attack-report
- https://www.twingate.com/blog/tips/Insomniac%20Games-data-breach
- https://80.lv/articles/spider-man-2-dev-was-breached-by-hackers/
- https://esports.gg/news/gaming/hackers-release-stolen-data-from-sonys-insomniac-studios
- https://www.watchguard.com/wgrd-news/blog/capcoms-vpn-entry-point-ransomware
- https://www.scworld.com/brief/capcom-blames-old-vpn-device-for-2020-ransomware-attack-2
- https://www.itsecurityguru.org/2021/04/14/capcom-release-final-update-on-ransomware-attack/
- https://threatpost.com/cyberpunk-2077-publisher-hack-ransomware/163775/
- https://www.computerweekly.com/news/252496143/HelloKitty-almost-certainly-behind-CD-Projekt-ransomware-attack
- https://www.incibe.es/index.php/en/incibe-cert/publications/cybersecurity-highlights/new-cyberattack-against-video-game-industry
- https://grahamcluley.com/following-ubisoft-cyber-attack-hackers-claim-to-leak-watch-dogs-legion-code-online/
- https://techraptor.net/gaming/news/ransomware-gang-leaks-data-from-ubisoft-and-crytek
- https://en.wikipedia.org/wiki/Nintendo_data_leak
- https://www.videogameschronicle.com/news/nintendo-says-it-increased-security-following-a-prolific-leak-of-game-prototypes
- https://nightfall.ai/blog/okta-data-breach-what-happened-impact-and-security-lessons-learned
- https://www.bleepingcomputer.com/news/security/okta-breach-134-customers-exposed-in-october-support-system-hack/amp/
- https://thehackernews.com/2023/11/oktas-recent-customer-support-data.html
- https://palant.info/2023/02/28/lastpass-breach-update-the-few-additional-bits-of-information/
- https://9to5mac.com/2023/02/27/lastpass-devops-engineers-hacked/
- https://www.cpomagazine.com/?p=30705
- https://circleci.com/blog/jan-4-2023-incident-report
- https://www.malwarebytes.com/blog/news/2023/01/circleci-malware-stole-github-oauth-keys-bypassing-2fa
- https://github.blog/news-insights/company-news/security-alert-stolen-oauth-user-tokens/
- https://threats.wiz.io/all-incidents/github-npm-breach
- https://securityaffairs.com/posts/github-warns-stolen-oauth-tokens-access-data

Standards
- https://www.ttpn.org/links-resources/
- https://www.ttpn.org/press-releases/tpn-to-launch-enhanced-shield-tier-system-to-strengthen-global-content-security/
- https://www.ttpn.org/2024/07/tpn-assessment-what-you-need-to-know/
- https://www.ttpn.org/wp-content/uploads/2025/11/TPNplus-Assessor-How-To-Guide_v5.3.1_Nov25.pdf
- https://www.ttpn.org/press-releases/tpn-updates-mpa-best-practices-to-include-hardening-guidelines-for-software-application-providers/
- https://massive.io/?p=126187
- https://www.securityscientist.net/blog/mpa-content-security-standard/
