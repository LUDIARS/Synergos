export const meta = {
  name: 'find-verify-theme',
  description: 'Lens-specialized finders for one theme (security | transfer | ai-game) with adversarial refute + materiality judge, then collector writes verified JSON',
  phases: [
    { title: 'Find', detail: 'one finder per lens, reads maps/research/code' },
    { title: 'Verify', detail: 'per group of findings: code refuter + materiality judge' },
    { title: 'Collect', detail: 'write verified findings JSON' },
  ],
}

const THEME = args && args.theme
if (!THEME) throw new Error('args.theme required: security | transfer | ai-game')
const SCRATCH = '/tmp/claude-0/-home-user/a3d7da4f-7a05-5e3e-8d37-b3591d10105f/scratchpad'

const PREAMBLE = `CONTEXT
Two repositories together form a new version-control system + operations service for game and AI asset production (LUDIARS org). The user wants a redesign that satisfies all of: (1) solving AI-development and game-development problems, (2) security, with special depth on mechanisms that curb vulnerabilities and contain breaches, (3) transfer speed and efficiency.
- /home/user/Synergos (Rust workspace): synergos-net (QUIC with ed25519 SPKI pinning + HLO1 hello, DHT, gossip, relay, cloudflared tunnel, STUN, bitswap/chunker/chain modules), synergos-core daemon (exchange = whole-file transfer with 64 KiB frames + BLAKE3, manifest.json committed to git as pointer set, history nodes retaining every version content-addressed, hook scripts, IPC server), synergos-control (org/node registry, Cloudflare mesh automation, admin/node tokens), synergos-custody (opaque object store: HMAC-signed PUT/GET URLs, SHA-256 verified, signed receipts, tombstones), synergos-relay (WSS relay), Tauri GUI, Dioxus admin UI, ars plugin.
- /home/user/Curare (TypeScript/Hono/Drizzle/PostgreSQL/Redis/MinIO): asset catalog, versioning types working/submission/reproducible, upload admission, submissions/review/release composition, provenance, access policy (identity/orgs from Cernere), storage port (MinIO or Synergos custody), shared-node control, workflow/assignments, client CLI/SDK and vanilla-JS Web UI.
- Org rules: /home/user/ludiars/aiformat (RULE.md, HARNESS.md incl. §3.5 decision axes = AI学習量 / 作業コスト / 解決度 / 主目的との一致度, RULE_SRE.md, RULE_TECH_STACK.md, RULE_DATA_SCHEMA.md, FORMAT_SPEC.md, common/REVIEW_VULNERABILITY.md, common/REVIEW_DESIGN.md, web/REVIEW_VULNERABILITY_WEB.md, local-app/REVIEW_VULNERABILITY_LOCAL.md). Auth centralizes in Cernere; secrets via Infisical; no personal data in service DBs; migrations idempotent and never DROP.
- Subsystem maps written by earlier readers (READ ALL OF THEM FIRST, they are your primary context): ${SCRATCH}/maps/*.md (sy-transport, sy-discovery, sy-content, sy-core-versioning, sy-history-hooks-ipc, sy-control-custody, sy-clients-docs, cu-auth-access, cu-storage-transfer, cu-versioning-core, cu-workflow-infra, cu-docs).
- Industry research notes: ${SCRATCH}/research/*.md (vcs-game-ai, transfer-efficiency, breach-containment, ai-dev-data).
- NOTE: the map sy-clients-docs.md may still be being written when you start; read it LAST, and if it is still absent after you finished the other 11 maps, retry once more before starting code work, then proceed without it and say so in coverage_notes.
Scratchpad root (the ONLY place you may write): ${SCRATCH}. Do NOT modify any file under /home/user/Synergos or /home/user/Curare. Today is 2026-10-15.`

const FINDING_ITEM = {
  type: 'object',
  properties: {
    id: { type: 'string' },
    title: { type: 'string' },
    theme: { type: 'string', enum: ['security', 'transfer', 'ai-game'] },
    repo: { type: 'string', enum: ['synergos', 'curare', 'both'] },
    area: { type: 'string' },
    severity: { type: 'string', enum: ['critical', 'high', 'medium', 'low'] },
    kind: { type: 'string', enum: ['vulnerability', 'missing-control', 'design-gap', 'inefficiency', 'missing-feature', 'inconsistency'] },
    evidence: { type: 'array', items: { type: 'object', properties: { file_line: { type: 'string' }, quote: { type: 'string' } }, required: ['file_line', 'quote'] } },
    scenario: { type: 'string' },
    impact_ai_game: { type: 'string' },
    current_state: { type: 'string' },
    proposed_redesign: { type: 'string' },
    alternatives: { type: 'array', items: { type: 'string' } },
    enforcement: { type: 'string', enum: ['memory', 'hook', 'ci-gate', 'runtime', 'n/a'] },
    confidence: { type: 'number' },
    sources: { type: 'array', items: { type: 'string' } },
  },
  required: ['id', 'title', 'theme', 'repo', 'area', 'severity', 'kind', 'evidence', 'scenario', 'impact_ai_game', 'current_state', 'proposed_redesign', 'confidence'],
}
const FINDINGS_SCHEMA = { type: 'object', properties: { findings: { type: 'array', items: FINDING_ITEM }, coverage_notes: { type: 'string' } }, required: ['findings', 'coverage_notes'] }

const REFUTE_SCHEMA = { type: 'object', properties: { verdicts: { type: 'array', items: { type: 'object', properties: { id: { type: 'string' }, verdict: { type: 'string', enum: ['confirmed', 'partially', 'refuted'] }, reason: { type: 'string' }, corrected_evidence: { type: 'string' }, corrected_severity: { type: 'string', enum: ['critical', 'high', 'medium', 'low'] } }, required: ['id', 'verdict', 'reason'] } } }, required: ['verdicts'] }
const JUDGE_SCHEMA = { type: 'object', properties: { verdicts: { type: 'array', items: { type: 'object', properties: { id: { type: 'string' }, material: { type: 'boolean' }, priority: { type: 'string', enum: ['P0', 'P1', 'P2', 'P3'] }, reason: { type: 'string' }, redesign_note: { type: 'string' }, duplicate_of: { type: 'string' } }, required: ['id', 'material', 'priority', 'reason'] } } }, required: ['verdicts'] }

const LENSES = {
  security: [
    { key: 'authz-boundaries', prefix: 'SEC-AZ', title: 'Authentication & authorization boundaries on every surface', brief: `Surfaces to audit, each with evidence: synergos-control admin/node/org APIs and token issuance; synergos-custody admin API and HMAC-signed transfer URLs; /peer-info HTTP; relay room join; IPC (uid/SID checks, which commands are privileged); gossip/DHT message authenticity and project membership (a 2026-08-16 problem log says membership is enforced only by reachability); hook registration; Curare every route (does each read/list/search/download/write enforce Cernere membership + Cu grant? IDOR on ids?), Cernere JWT/session verification and dev-mode header fallback, grant revocation after除名, admin-only operations, Web UI; Cu→Sy confused-deputy (can Cu be made to issue URLs for objects it should not?). Apply checklists: common/REVIEW_VULNERABILITY.md §1 auth items, web/REVIEW_VULNERABILITY_WEB.md §1-2, local-app §3.` },
    { key: 'transport-crypto', prefix: 'SEC-CR', title: 'Transport security, cryptography, keys and secrets', brief: `Audit: QUIC verifier/SPKI pinning and HLO1 (downgrade, replay, identity binding, peer-id derivation), identity file encryption (Argon2id params, AES-GCM nonce handling), invite token format/entropy/expiry/replay/single-use, HMAC URL signing (canonicalization, timing-safe comparison, key separation per purpose, expiry skew), receipt signature canonical JSON (unicode/number edge cases), control/custody HTTP transport (plaintext? behind tunnel? Host validation), cloudflared trust, randomness sources, absence of key rotation/revocation, secrets in env/config/logs/URLs, Tauri updater/code signing, TLS versions. Apply common/REVIEW_VULNERABILITY.md §1 crypto items and web §3.` },
    { key: 'integrity-dataloss', prefix: 'SEC-IN', title: 'Storage integrity, immutability and anti-data-loss (including ransomware resilience)', brief: `Audit: manifest atomic writes and concurrent writers; exchange destination replacement; history store objects/index/GC/rotation safety (can referenced versions be deleted? index corruption recovery; rotation put/delete ordering; tags protection); custody tombstones/reclaim/re-PUT; MinIO conditional PUT and checksum; completeness of size+hash verification on every receive path; symlink/path traversal/zip-slip in exchange, history root, custody keys, Curare fileName/object keys; TOCTOU; crash consistency (fsync); Curare immutability of confirmed versions (what can UPDATE/DELETE them, soft delete, migrations never DROP); who can bulk-delete or overwrite history from a compromised client; backups and restore. Apply common §1 items on path traversal, TOCTOU, validation.` },
    { key: 'blast-radius', prefix: 'SEC-BR', title: 'Breach containment and blast radius', brief: `For each compromise scenario enumerate what the attacker can reach and what stops them, with evidence: (a) a member's workstation/daemon, (b) a node token, (c) control admin token, (d) custody admin token or receipt secret, (e) Cu server host, (f) a Cernere session/JWT, (g) a history node, (h) Cloudflare API token, (i) a hook script author, (j) an external vendor via magic link/short URL. Check: lateral movement across projects/orgs, token scope/lifetime, revocation propagation delay, audit trail of downloads, exfiltration detection, quotas, kill switch, encryption at rest, per-org isolation of storage/keys, hook/plugin execution privileges and sandboxing, Tauri capabilities. Rate each scenario's containment today and name the mechanism that would contain it (consult ${SCRATCH}/research/breach-containment.md).` },
    { key: 'supply-chain-config', prefix: 'SEC-SC', title: 'Supply chain, CI/CD, configuration and secrets hygiene', brief: `Audit: both CI workflows (permissions, action pinning by SHA, frozen lockfile, cargo-audit/cargo-deny/pnpm audit presence, secret scanning, pull_request_target), dependency inventory (git deps, unmaintained/yanked crates, native addons, cernere-id-cache file: dep), docker-compose/.env secrets and defaults, Infisical/Excubitor injection vs hardcoded fallbacks, fail-fast on missing secrets, log redaction, cloudflared binary provenance and auto-update, Tauri updater signing, release artifact signing/SBOM, admin-ui wasm build. Apply common §2 and web §3 supply-chain items.` },
    { key: 'dos-abuse-injection', prefix: 'SEC-DA', title: 'Denial of service, abuse, and injection (parsers, commands, URLs, UI)', brief: `Audit: untrusted frame parsing limits (QUIC streams, gossip/DHT/relay message sizes, msgpack/protobuf/JSON size caps, length-prefixed allocation), unbounded loops/collections, relay amplification, upload quotas/object size limits, rate limiting on all public endpoints, DB query cost (search/list without pagination/limits), command injection (hooks runner, cloudflared spawn args), SSRF (rotation backends, bootstrap URLs with redirects, Cloudflare API, Cu→Sy control-client/IPC URLs, presigned URL hosts), header/URL injection, XSS in the vanilla-JS UI (innerHTML usage), SQL via raw queries, deserialization. Apply common §1 and web §1 items.` },
    { key: 'detection-response-ops', prefix: 'SEC-DR', title: 'Audit logging, detection, incident response and operational security', brief: `Audit: what security-relevant events are logged in each service (auth failures, token issuance, downloads, deletes, hook changes, admin ops), tamper evidence/retention/search, correlation ids, integration with Vestigium/Concordia, anomaly detection on bulk downloads, honeytokens, node quarantine/global revocation/kill switch, backup+restore drills, key-compromise playbooks, security headers/CSP/CORS/Host checks, /api/health leakage, RULE_SRE.md compliance, operational docs for incident handling. Compare with RULE.md §1 Step 8 (operation_logs) expectations and web §2 zero-trust checklist.` },
  ],
  transfer: [
    { key: 'wire-path', prefix: 'TRN-WP', title: 'Synergos wire path throughput and latency', brief: `Analyse the actual transfer path: framing (64 KiB frames?), per-file stream usage, concurrency limits, QUIC/quinn config (congestion controller, stream/connection windows, MTU/GSO, idle timeout, keepalive, 0-RTT off), BLAKE3 hashing placement (streaming vs post-hoc, extra passes), disk I/O (temp file + rename + copy into history objects = how many writes per byte?), memory copies, catalog/offer/want round trips before bytes flow, auto-pull latency. Compare with DESIGN.md §15 promises (bandwidth classes, speed test, bandwidth-aware peer selection, multicast, monitoring) and state what is implemented. Give a quantitative model (e.g., for a 500 MB PSD, 5 GB FBX set, 100 GB checkpoint) of where time goes today and what each proposed change buys (cite ${SCRATCH}/research/transfer-efficiency.md).` },
    { key: 'dedup-delta-resume', prefix: 'TRN-DD', title: 'Deduplication, delta, chunking and resumability', brief: `Evaluate the 2026-08-16 decision to do whole-file transfer and no CDC: quantify its cost for iterative PSD saves, FBX re-exports, texture sets, Unity/Unreal trees, multi-million-file datasets, 10-100 GB checkpoints using research numbers; examine history-store file-level dedup, custody whole-object re-send after interruption, Curare single presigned PUT (no multipart/tus), client double hashing (SHA-256 for Cu, BLAKE3 for Sy), dormant chunker/bitswap code. Propose a minimal-regret design (e.g., chunked content-addressed objects with verified streaming (BLAKE3/Bao), resumable ranges, dedup inside custody/history only, delta for known formats) with the 4-axis evaluation.` },
    { key: 'topology-relay', prefix: 'TRN-TP', title: 'Topology, discovery, relay and multi-source distribution', brief: `Analyse: Cloudflare Tunnel cannot carry UDP so QUIC falls back to WSS relay (throughput, head-of-line blocking, single relay SPOF), STUN/TURN unwired, promotion logic, peer selection, no multi-source/parallel fetch, DHT/gossip overhead and catalog sync chattiness at scale (N peers x M files), history node as always-on seed, site/edge caches for studios with 50-500 seats, mobile/remote vendors. Propose topology redesign (e.g., always-on site seed + swarm fetch + relay with UDP/QUIC-capable path such as DERP-style or TURN) with 4-axis evaluation.` },
    { key: 'e2e-flow-metadata', prefix: 'TRN-E2', title: 'Curare-Synergos end-to-end flow efficiency and metadata scalability', brief: `Count round trips and hashing passes for: working save, submission of N files, clone of an asset set with dependencies (download URL per file? 900 s URLs), receipt verification, register. Examine batch APIs absence, per-file DB rows for datasets, N+1 queries, missing indexes, pagination, search cost, client-side pre-hash cost, IPC-client command granularity, lazy/virtual checkout absence, caching. Propose manifest-level/batch APIs, lazy fetch, and index changes with 4-axis evaluation.` },
  ],
  'ai-game': [
    { key: 'gamedev-workflow', prefix: 'AGD-GW', title: 'Game-studio workflow gaps versus Perforce / Unity VCS / Git LFS', brief: `Map each capability to Cu/Sy status with evidence: exclusive locking of binaries, branches/streams and release composition, shelving, partial workspaces (Gluon-like), engine integration (Unity/Unreal/Maya/Blender/Photoshop plugins), visual diff/preview/thumbnails, artist UX (Web UI vanilla JS, Tauri daemon GUI), offline work and reconnection, build/cook artifacts and DDC, submission/review/assignments/magic links for vendors, text-asset merge (Unity YAML via git), migration from Perforce/SVN/Git LFS, scale (seats, TB). Use ${SCRATCH}/research/vcs-game-ai.md. Findings should be concrete design gaps with proposed redesign and 4-axis evaluation.` },
    { key: 'ai-dataset-model', prefix: 'AGD-AI', title: 'AI-development needs: datasets, models, lineage, governance', brief: `Evaluate fit for: datasets of millions of files (asset = one file in Cu; per-path manifest in Sy), dataset snapshots and lineage (sourceVersionIds/provenance), model checkpoints 10-100 GB, the 'reproducible' versioning type (rejected from storage) versus ML cache semantics, lazy/virtual mount and GPU-cluster access (S3-compatible gateway), dedup across datasets, metadata/embedding search, AI-generated asset provenance (C2PA/prompt/model/seed), license/consent/opt-out flags, PII screening, retention/deletion, integration vs build for DVC/lakeFS/HF Xet. Use ${SCRATCH}/research/ai-dev-data.md. Quote where current docs mention AI (cu-docs map).` },
    { key: 'data-model-fit', prefix: 'AGD-DM', title: 'Data model and API fit for a version-control system', brief: `Examine: multi-file atomic changes (changesets) vs per-asset versions, directory trees/renames/moves, partial/sparse checkout, dependency graph, ID schemes, cross-project reuse, Synergos manifest (per-path monotonic version, no changeset identity, requires git to carry manifest.json while Curare says Git is not a prerequisite -- inconsistency?), tag/pin semantics across Sy and Cu, conflict model for binaries, Sy node-local state vs Cu DB as source of truth, scaling of Postgres rows, search. Produce findings on model mismatches and a proposed unified model (e.g., content-addressed tree/changeset objects shared by Cu and Sy) with 4-axis evaluation.` },
    { key: 'ops-platform-scale', prefix: 'AGD-OP', title: 'Operability, platform and multi-tenant scale for studios', brief: `Examine: Windows/macOS daemon and Tauri readiness, admin UI, path/encoding issues (Unicode NFC/NFD, case-insensitive FS, long paths, reserved names), multi-site replication/history nodes, observability (metrics/tracing), backup/restore, migration tooling, SaaS multi-tenancy isolation (Cu shared nodes, Sy control orgs), Cernere coupling for external studios, licensing/monetization assumptions (spec/plan/monetization.md), RULE_SRE.md watchpoints for local apps and P2P. Use maps sy-clients-docs, cu-docs, cu-workflow-infra.` },
  ],
}

const lenses = LENSES[THEME]
if (!lenses) throw new Error('unknown theme ' + THEME)

phase('Find')
const findResults = await parallel(lenses.map((l) => () =>
  agent(`${PREAMBLE}

YOUR LENS (theme=${THEME}): ${l.title}
${l.brief}

METHOD
1. Read all 12 maps and the 4 research notes first (they are long; read them fully).
2. Then open the source files relevant to your lens and verify every claim you will make at the exact file:line (the maps may be wrong; you are responsible for evidence). Prefer quoting 1-3 lines of code.
3. Produce 8-25 findings. Ids must be ${l.prefix}-01, ${l.prefix}-02, ... Each finding must have: concrete evidence (file:line + quote, or doc path + quote for design gaps), an attack/failure scenario a reviewer can follow, why it matters for AI/game asset production, current state, a proposed redesign (specific: data model / API / protocol / config / process change), alternatives considered, how the fix would be enforced (memory / hook / ci-gate / runtime), confidence 0-1, and sources (research note sections or URLs) when you rely on external facts.
4. Do not pad with trivia; do not omit anything material. Mark design gaps (nothing to exploit, but missing capability) with kind=design-gap or missing-feature.
5. Write the same findings as JSON to ${SCRATCH}/findings/${THEME}-${l.key}.json (an object {findings:[...], coverage_notes:"..."}). Then return the structured output.`,
    { label: `find:${THEME}:${l.key}`, phase: 'Find', schema: FINDINGS_SCHEMA })
))

const all = findResults.filter(Boolean).flatMap((r) => r.findings || [])
log(`${THEME}: ${all.length} raw findings from ${findResults.filter(Boolean).length}/${lenses.length} finders`)

const seen = new Set()
const unique = all.filter((f) => { if (seen.has(f.id)) return false; seen.add(f.id); return true })
const GROUP = 6
const groups = []
for (let i = 0; i < unique.length; i += GROUP) groups.push(unique.slice(i, i + GROUP))
log(`${THEME}: verifying ${unique.length} findings in ${groups.length} groups`)

phase('Verify')
const verified = await pipeline(groups,
  (group, _item, gi) => parallel([
    () => agent(`${PREAMBLE}

ROLE: adversarial REFUTER for theme=${THEME}. For each finding below, try to REFUTE it: open the cited file:line in the current working tree, check the claim literally, look for an existing mitigating control elsewhere in the code (grep the repos), and check whether the scenario is actually reachable. verdict=confirmed only when the evidence holds and the scenario is reachable; partially when the core issue exists but severity/scope is overstated or evidence is slightly off (give corrected_evidence / corrected_severity); refuted when the claim is false, already mitigated, or unreachable. Default to refuted when you cannot verify. Be rigorous and specific in reason (quote the code you checked).

FINDINGS (JSON):
${JSON.stringify(group, null, 1)}`, { label: `refute:${THEME}:g${gi + 1}`, phase: 'Verify', schema: REFUTE_SCHEMA }),
    () => agent(`${PREAMBLE}

ROLE: materiality JUDGE for theme=${THEME}. The goal is a redesign document (spec/plan) listing missing elements and points to redesign so the platform satisfies AI/game development needs, security (esp. breach containment), and transfer efficiency. For each finding decide: material=true if it warrants a design change or explicit design statement (not a trivial one-line fix that needs no design), priority P0 (must redesign before production use) / P1 (next design iteration) / P2 (later) / P3 (nice to have), and give a redesign_note: the crispest statement of what the design doc should say, evaluated briefly on the LUDIARS 4 axes (AI学習量, 作業コスト, 解決度, 主目的との一致度). If a finding duplicates another in this group, set duplicate_of to the surviving id.

FINDINGS (JSON):
${JSON.stringify(group, null, 1)}`, { label: `judge:${THEME}:g${gi + 1}`, phase: 'Verify', schema: JUDGE_SCHEMA }),
  ]).then(([ref, jud]) => group.map((f) => {
    const r = ref && ref.verdicts ? ref.verdicts.find((v) => v.id === f.id) : null
    const j = jud && jud.verdicts ? jud.verdicts.find((v) => v.id === f.id) : null
    return { ...f, refute: r || { id: f.id, verdict: 'unverified', reason: 'refuter missing' }, judge: j || { id: f.id, material: true, priority: 'P2', reason: 'judge missing' } }
  }))
)

const flat = verified.filter(Boolean).flat()
const kept = flat.filter((f) => f.refute.verdict !== 'refuted' && f.judge.material && !f.judge.duplicate_of)
const dropped = flat.filter((f) => !(f.refute.verdict !== 'refuted' && f.judge.material && !f.judge.duplicate_of))
log(`${THEME}: kept ${kept.length}, dropped ${dropped.length} (refuted/immaterial/duplicate)`)

phase('Collect')
const outPath = `${SCRATCH}/findings/${THEME}-verified.json`
const payload = JSON.stringify({ theme: THEME, kept, dropped: dropped.map((f) => ({ id: f.id, title: f.title, refute: f.refute, judge: f.judge })) }, null, 1)
await agent(`Write the following JSON verbatim (do not alter, reformat, or summarize it) to the file ${outPath} using the Write tool, then return the string "written". Do not write anywhere else.

${payload}`, { label: `collect:${THEME}`, phase: 'Collect', effort: 'low' })

return { theme: THEME, raw: all.length, kept: kept.map((f) => ({ id: f.id, title: f.title, severity: f.refute.corrected_severity || f.severity, priority: f.judge.priority, verdict: f.refute.verdict })), dropped: dropped.map((f) => ({ id: f.id, verdict: f.refute.verdict, material: f.judge.material, dup: f.judge.duplicate_of || null })), outPath }
