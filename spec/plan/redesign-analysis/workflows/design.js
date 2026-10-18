export const meta = {
  name: 'design-theme',
  description: 'Per theme: judge-panel synthesis of a redesign brief, then Japanese spec/plan docs for Synergos and Curare, critiqued and revised',
  phases: [
    { title: 'Synthesize', detail: '3 architects from different angles + 1 synthesizer -> brief with contract registry' },
    { title: 'Write', detail: 'one writer per repo (Japanese, FORMAT_SPEC plan)' },
    { title: 'Critique', detail: 'completeness critic + format/consistency critic per doc' },
    { title: 'Revise', detail: 'writer applies critiques' },
  ],
}

const THEME = args && args.theme
if (!THEME) throw new Error('args.theme required')
const SCRATCH = 'spec/plan/redesign-analysis'
const SY_SHA = (args && args.sySha) || 'unknown'
const CU_SHA = (args && args.cuSha) || 'unknown'

const THEME_JA = { security: 'セキュリティ (脆弱性と侵害の歯止め)', transfer: '転送速度と効率', 'ai-game': 'AI開発・ゲーム開発の課題解決' }[THEME]
const FILE_SLUG = { security: 'security', transfer: 'transfer-efficiency', 'ai-game': 'ai-game-dev' }[THEME]

const PREAMBLE = `CONTEXT
Two repositories together form a new version-control system + operations service for game and AI asset production (LUDIARS org). The user's request (Japanese): 「新しいバージョン管理の仕組みとその運用サービスである。AI開発やゲーム開発の課題解決・セキュリティ・転送速度と効率を解決するソリューションにしたいが、全てを満たす上で足りない要素や再設計した方が良い点を洗い出して設計書をそれぞれのブランチにコミットする。脆弱性および侵害の歯止めをかける機構についてはよく調査する。」
- Synergos (Rust) = transport/custody/history/control plane. main @ ${SY_SHA}.
- Curare (TypeScript) = asset catalog/versioning/submission/release/access. main @ ${CU_SHA}.
- Org rules: AIFormat (RULE.md, HARNESS.md §3.5 decision axes = AI学習量 / 作業コスト / 解決度 / 主目的との一致度 and §2.0 enforcement classes 記憶 / フック / ローカル検証ゲート, RULE_SRE.md, RULE_TECH_STACK.md, RULE_DATA_SCHEMA.md, FORMAT_SPEC.md §6 plan format, common/REVIEW_DESIGN.md).
- Existing decisions to respect or explicitly challenge: 2026-08-16 (neco) no delta/CDC, whole-file transfer, history-node flag model (Synergos docs/versioning-design.md); 2026-09-08 (neco) Cu/Sy/Cr boundary: Curare owns versions/submissions/permissions, Synergos moves opaque objects and issues durable receipts, Cernere owns identity/orgs (Curare spec/plan/versioning-types.md, spec/feature/server-client-architecture.md, Synergos spec/feature/opaque-custody.md).
- Inputs you MUST read: verified findings ${SCRATCH}/findings/${THEME}-verified.json (kept[] = verified; dropped[] = do not use), the other two themes' verified findings for cross-references (${SCRATCH}/findings/security-verified.json, transfer-verified.json, ai-game-verified.json — whichever exist), subsystem maps ${SCRATCH}/maps/*.md (12 files), research notes ${SCRATCH}/research/*.md (4 files), the doc template ${SCRATCH}/design/TEMPLATE.md.
Scratchpad root (the ONLY place you may write): ${SCRATCH}. Do NOT modify Synergos or Curare. Today is 2026-10-15.
THEME of this run: ${THEME} = ${THEME_JA}.`

const ANGLES = {
  security: [
    { key: 'containment-first', brief: 'Assume breach. Design for blast radius, revocation speed, immutability/ransomware resilience, per-tenant key isolation, kill switch, detection. Start from the compromise scenarios in the findings (workstation, node token, control admin token, custody token, Cu server, Cernere session, history node, Cloudflare token, hook author, external vendor) and specify the mechanism that contains each.' },
    { key: 'boundary-first', brief: 'Design the identity/authorization model across Cernere -> Curare -> Synergos: token formats (capability/attenuation), scopes, lifetimes, per-session authorization of QUIC/IPC/HTTP surfaces, project membership authentication in gossip/DHT, custody URL binding, admin/node token lifecycle, audit events. Specify contracts (IDs) both repos must implement.' },
    { key: 'operability-first', brief: 'Design what can be enforced cheaply and verified mechanically: supply-chain/CI hardening, secure-by-default config and fail-fast, ingest validation and format-aware script scanning (Maya/Blender/Unity), sandboxing of hooks and parsers, logging/audit/detection pipeline, security tests and fuzzing, incident playbooks. For each item name the enforcement class (記憶/フック/ローカル検証ゲート/ランタイム).' },
  ],
  transfer: [
    { key: 'wire-first', brief: 'Design the byte path: chunked content-addressed objects vs whole-file, verified streaming (BLAKE3 tree/Bao), resumable transfer, QUIC tuning, hashing passes, disk I/O, parallel streams, compression policy per format, with quantitative models per workload (PSD, FBX, textures, Unity/Unreal trees, datasets, checkpoints). Decide what to keep from the 2026-08-16 decision and what to revisit, with 4-axis tables.' },
    { key: 'topology-first', brief: 'Design distribution topology: always-on site seeds/history nodes, multi-source swarm fetch, relay path that can carry QUIC (DERP/TURN-style), STUN/TURN wiring, peer selection, edge caches for studios and vendors, catalog/gossip scalability, mobile/remote. Specify roles as modes of one binary with one replication stream.' },
    { key: 'flow-first', brief: 'Design the Curare<->Synergos end-to-end flow for efficiency: batch/manifest-level admission and receipts, single hashing standard, lazy/partial checkout, clone of N files in O(1) control round trips, index/pagination/N+1 fixes, client-side caching, IPC/control API granularity, SLAs/metrics. Specify contracts (IDs).' },
  ],
  'ai-game': [
    { key: 'studio-first', brief: 'Design for artists and game teams: exclusive locking with branch-aware state, streams/release composition, partial workspaces, DCC/engine integrations (Unity/Unreal/Maya/Blender/Photoshop), previews/visual diff, offline + reconnect, vendor magic links, migration from Perforce/Git LFS (LFS-compatible API), Windows/mac daemon UX. Prioritize with 4-axis tables.' },
    { key: 'ml-first', brief: 'Design for AI development: datasets as versioned trees of millions of files, dataset snapshots/lineage/provenance, model checkpoints, reproducible-type semantics vs ML caches, lazy mount/GPU-cluster access, dedup across datasets, metadata/embedding search, generated-asset provenance (C2PA), license/consent/PII governance, retention; decide build vs integrate (DVC/lakeFS/HF).' },
    { key: 'model-first', brief: 'Design the unified data model: changesets/trees shared by Curare and Synergos, identity of versions across repos, Synergos manifest vs Curare DB source of truth, git-dependency of manifest.json vs "Git not a prerequisite", conflict model, IDs, scale (rows per file), search; migration path from today\'s schemas without DROP.' },
  ],
}

const OUTLINE_SCHEMA = {
  type: 'object',
  properties: {
    outline_path: { type: 'string' },
    thesis: { type: 'string' },
    decisions: { type: 'array', items: { type: 'object', properties: { id: { type: 'string' }, question: { type: 'string' }, options: { type: 'array', items: { type: 'string' } }, chosen: { type: 'string' }, rationale_4axis: { type: 'string' }, challenges_existing_decision: { type: 'boolean' } }, required: ['id', 'question', 'options', 'chosen', 'rationale_4axis'] } },
    mechanisms: { type: 'array', items: { type: 'object', properties: { id: { type: 'string' }, name: { type: 'string' }, repo: { type: 'string', enum: ['synergos', 'curare', 'both', 'cernere'] }, finding_ids: { type: 'array', items: { type: 'string' } }, summary: { type: 'string' }, enforcement: { type: 'string' }, phase: { type: 'string' } }, required: ['id', 'name', 'repo', 'finding_ids', 'summary', 'enforcement', 'phase'] } },
    contracts: { type: 'array', items: { type: 'object', properties: { id: { type: 'string' }, producer: { type: 'string' }, consumer: { type: 'string' }, summary: { type: 'string' } }, required: ['id', 'producer', 'consumer', 'summary'] } },
    open_questions: { type: 'array', items: { type: 'string' } },
  },
  required: ['outline_path', 'thesis', 'decisions', 'mechanisms', 'contracts'],
}

const BRIEF_SCHEMA = { type: 'object', properties: { brief_path: { type: 'string' }, summary: { type: 'string' }, contract_ids: { type: 'array', items: { type: 'string' } }, mechanism_count: { type: 'number' }, finding_ids_covered: { type: 'array', items: { type: 'string' } }, finding_ids_uncovered: { type: 'array', items: { type: 'string' } } }, required: ['brief_path', 'summary', 'contract_ids', 'mechanism_count', 'finding_ids_covered', 'finding_ids_uncovered'] }
const DOC_SCHEMA = { type: 'object', properties: { doc_path: { type: 'string' }, line_count: { type: 'number' }, sections: { type: 'array', items: { type: 'string' } }, finding_ids_cited: { type: 'array', items: { type: 'string' } }, contract_ids_used: { type: 'array', items: { type: 'string' } }, notes: { type: 'string' } }, required: ['doc_path', 'line_count', 'sections', 'finding_ids_cited', 'contract_ids_used'] }
const CRITIQUE_SCHEMA = { type: 'object', properties: { issues: { type: 'array', items: { type: 'object', properties: { severity: { type: 'string', enum: ['blocker', 'major', 'minor'] }, location: { type: 'string' }, problem: { type: 'string' }, fix: { type: 'string' } }, required: ['severity', 'location', 'problem', 'fix'] } }, verdict: { type: 'string' } }, required: ['issues', 'verdict'] }

// ---------- Synthesize ----------
phase('Synthesize')
const angles = ANGLES[THEME]
const outlines = await parallel(angles.map((a) => () =>
  agent(`${PREAMBLE}

ROLE: ARCHITECT (${a.key}). ${a.brief}
Read all inputs listed above fully (findings JSON first, then maps and research). Produce a redesign outline for theme ${THEME} covering BOTH repos. Every mechanism must reference the finding ids it addresses (use kept[] ids only). Decisions must be presented as option comparisons on the 4 LUDIARS axes. Flag any decision that challenges an existing neco decision. Propose contract ids in the form SPEC-SY-<slug> / SPEC-CU-<slug> / SPEC-CR-<slug> for anything that crosses repos. Be concrete: data model fields, API shapes, protocol messages, config keys, enforcement class, rollout phase.
Write the outline (English, Markdown, 1500-4000 words) to ${SCRATCH}/design/${THEME}-outline-${a.key}.md, then return the structured output (outline_path = that path).`,
    { label: `architect:${THEME}:${a.key}`, phase: 'Synthesize', schema: OUTLINE_SCHEMA })
))
const okOutlines = outlines.filter(Boolean)
log(`${THEME}: ${okOutlines.length}/${angles.length} architect outlines`)

const brief = await agent(`${PREAMBLE}

ROLE: SYNTHESIZER. Three architects wrote outlines from different angles: ${okOutlines.map((o) => o.outline_path).join(', ')}. Read them fully, plus the verified findings and the template ${SCRATCH}/design/TEMPLATE.md. Merge them into ONE coherent redesign brief for theme ${THEME} that the two document writers (one per repo) will follow. Rules:
- Resolve conflicts between outlines explicitly (state which option wins and why on the 4 axes); keep the best ideas from the losers as alternatives.
- Produce a CONTRACT REGISTRY: a table of contract ids (SPEC-SY-*/SPEC-CU-*/SPEC-CR-*) with producer, consumer, one-paragraph definition, and which doc (Synergos / Curare) is the 正本 for each. Both docs must use identical ids and definitions.
- Produce a MECHANISM LIST with ids (e.g. ${THEME.toUpperCase().replace('-', '')}-M01...), repo, finding ids addressed, enforcement class, phase, and a 2-5 sentence design summary each.
- Produce a FINDING COVERAGE table: every kept finding id -> mechanism id(s) or an explicit reason it is deferred.
- Produce the DECISION TABLE (4-axis comparisons) and the PHASE PLAN (phases, dependencies, rollback).
- Produce per-repo section guidance: what goes in the Synergos doc vs the Curare doc (1 機能 1 正本; cross-reference the other).
- Produce explicit numeric goals where possible (e.g., revocation propagation <= 60 s; re-transfer ratio for a modified 500 MB PSD; clone control round trips).
Write the brief (English, Markdown; length as needed, typically 4000-8000 words) to ${SCRATCH}/design/${THEME}-brief.md and return the structured output.`,
  { label: `synthesize:${THEME}`, phase: 'Synthesize', schema: BRIEF_SCHEMA })
if (!brief) throw new Error('synthesizer failed')
log(`${THEME}: brief written, ${brief.mechanism_count} mechanisms, uncovered findings: ${brief.finding_ids_uncovered.length}`)

// ---------- Write ----------
phase('Write')
const REPOS = [
  { key: 'synergos', name: 'Synergos', root: 'Synergos', sha: SY_SHA, out: `${SCRATCH}/docs/Synergos/spec/plan/redesign-${FILE_SLUG}.md`, sibling: `Curare/spec/plan/redesign-${FILE_SLUG}.md`, extra: 'Synergos-specific requirements: cite Rust crate/module/file:line; respect docs/versioning-design.md decisions (challenge explicitly if needed); include the crate-level change map (synergos-net / synergos-core / synergos-control / synergos-custody / synergos-relay / synergos-ipc / GUI); domain names from spec/domains/*.domain.json (network-transport, mesh-core, control-plane, opaque-custody, desktop-gui, admin-ui, ars-plugin).' },
  { key: 'curare', name: 'Curare', root: 'Curare', sha: CU_SHA, out: `${SCRATCH}/docs/Curare/spec/plan/redesign-${FILE_SLUG}.md`, sibling: `Synergos/spec/plan/redesign-${FILE_SLUG}.md`, extra: 'Curare-specific requirements: cite curare-server/src file:line and migrations; respect spec/plan/versioning-types.md and spec/feature/server-client-architecture.md (challenge explicitly if needed); include DB changes as additive migrations (no DROP), RULE_DATA_SCHEMA table for new data, DDD boundary names from spec/architecture/ddd.md (asset-catalog, asset-versioning, submission-review, release-composition, asset-access, production-pipeline, production-workflow, asset-storage, shared-node-control); note Cernere-side asks as SPEC-CR-* contracts.' },
]

const writerPrompt = (r, critique) => `${PREAMBLE}

ROLE: DOCUMENT WRITER for the ${r.name} repository, theme ${THEME}.
Read fully: the brief ${SCRATCH}/design/${THEME}-brief.md, the template ${SCRATCH}/design/TEMPLATE.md, the verified findings (${SCRATCH}/findings/${THEME}-verified.json; use only kept[]), the maps relevant to ${r.name} (${r.key === 'synergos' ? 'sy-*.md' : 'cu-*.md'} and cu-docs/sy-clients-docs for intent), and research notes as needed for citations. Open source files to confirm every file:line you cite (current main @ ${r.sha}).
Write the design document in JAPANESE following the template EXACTLY (same section numbering and headings). File: ${r.out} (create directories as needed). Title: 「${r.name} 再設計 — ${THEME_JA}」. Set 対象: ${r.name} main @ ${r.sha}. Sibling document to cross-reference by relative path: ../../${r.sibling} (write it as a plain path, e.g. 「Curare 側: spec/plan/redesign-${FILE_SLUG}.md」), and the other two theme docs of this repo: spec/plan/redesign-security.md, spec/plan/redesign-transfer-efficiency.md, spec/plan/redesign-ai-game-dev.md (whichever are not this one).
${r.extra}
Content rules:
- Section 2 must table every kept finding that concerns ${r.name} (repo = ${r.key} or both) with its id, 区分, 現状 (file:line), 影響, 重大度, 再設計の方向. Findings for the other repo appear only as cross-references.
- Section 3 must contain the decision tables from the brief with 4-axis comparisons (AI 学習量 / 作業コスト / 解決度 / 主目的との一致度) and the 採用案.
- Section 4 must specify each mechanism that lands in ${r.name}: 目的 / 仕組み (データモデル・API・プロトコル・設定・運用; concrete field names, message names, config keys) / 不変条件 / 失敗時の振る舞い / 変更対象 (crate or module, file) / 担保手段 (記憶・フック・ローカル検証ゲート・ランタイム). Use the contract ids from the brief's registry verbatim and quote each contract's definition once.
- Section 5: phase table with 依存 / 検証 / ロールバック, compatibility and migration.
- Section 6: acceptance tests (security: attack-scenario reproduction tests; transfer: measurement targets with 母集団; ai-game: workflow acceptance scenarios).
- Section 7: 未決事項 with 判断者 (role names only, e.g. neco / プロダクトオーナー / セキュリティ担当).
- Section 8: external URLs actually relied on (from research notes), 10-40 items.
- Target length 500-900 lines; longer is acceptable when the content demands it. No YAML frontmatter. No personal names or emails. No secrets or test fixture values. Japanese prose, identifiers in English. Avoid 「〜かもしれない」; write 「未確認 (理由)」 when not verifiable.
${critique ? `\nREVISION PASS. A previous version exists at ${r.out}. Apply ALL of the following critique items (blockers and majors mandatory, minors where reasonable), keeping everything else intact. Re-verify any file:line you change.\nCRITIQUE:\n${critique}\n` : ''}
After writing, return the structured output (doc_path = ${r.out}).`

const docs = await parallel(REPOS.map((r) => () => agent(writerPrompt(r, null), { label: `write:${THEME}:${r.key}`, phase: 'Write', schema: DOC_SCHEMA })))
log(`${THEME}: ${docs.filter(Boolean).length}/2 docs written`)

// ---------- Critique ----------
phase('Critique')
const critiques = await parallel(REPOS.map((r, i) => () => parallel([
  () => agent(`${PREAMBLE}

ROLE: COMPLETENESS CRITIC for ${r.out} (theme ${THEME}, repo ${r.name}). Read the document, the brief ${SCRATCH}/design/${THEME}-brief.md, the verified findings ${SCRATCH}/findings/${THEME}-verified.json (kept[]), and spot-check 10+ cited file:line references against the source tree (${r.root}). Report issues: kept findings for this repo missing from §2 or unaddressed in §4; mechanisms in the brief missing or diluted; contract ids missing or defined differently from the registry; wrong or unverifiable file:line citations; claims contradicting the maps/code; numeric goals missing; decisions without 4-axis comparison; existing neco decisions changed without explicit flagging; §5/§6 missing rollback or acceptance tests. Severity: blocker (must fix before commit), major, minor. Give a concrete fix per issue.`,
    { label: `critic-complete:${THEME}:${r.key}`, phase: 'Critique', schema: CRITIQUE_SCHEMA }),
  () => agent(`${PREAMBLE}

ROLE: FORMAT & CONSISTENCY CRITIC for ${r.out} (theme ${THEME}, repo ${r.name}). Check: exact conformance to ${SCRATCH}/design/TEMPLATE.md (section numbers/headings/order), FORMAT_SPEC.md §6 expectations (背景・ゴール・非ゴール / 設計判断と選択肢 / 実装ステップ・影響範囲・ロールバック), HARNESS.md §2.0 enforcement classes named for each mechanism, RULE_DATA_SCHEMA table present when new data is introduced, Japanese prose quality and consistency with existing spec style (read 2 existing files under ${r.root}/spec for tone), no YAML frontmatter, no personal names/emails, no secrets/test fixtures, no 「かもしれない」-style speculation, correct relative paths for cross-references, and CONSISTENCY WITH THE SIBLING DOCUMENT ${REPOS[1 - i].out} (read it fully): identical contract ids and definitions, no contradictory decisions, no duplicated 正本 (each mechanism has one owner doc and the other cross-references). Also check that the document reads as a standalone design doc for a reader who has not seen the brief. Severity: blocker / major / minor with concrete fixes.`,
    { label: `critic-format:${THEME}:${r.key}`, phase: 'Critique', schema: CRITIQUE_SCHEMA }),
]).then((cs) => cs.filter(Boolean).flatMap((c) => c.issues || []))))

// ---------- Revise ----------
phase('Revise')
const revised = await parallel(REPOS.map((r, i) => () => {
  const issues = (critiques[i] || [])
  const text = issues.map((is, n) => `${n + 1}. [${is.severity}] ${is.location}: ${is.problem}\n   FIX: ${is.fix}`).join('\n')
  log(`${THEME}/${r.key}: ${issues.length} critique items (${issues.filter((x) => x.severity === 'blocker').length} blockers)`)
  return agent(writerPrompt(r, text || '(no issues reported; re-read the document once and fix anything you notice)'), { label: `revise:${THEME}:${r.key}`, phase: 'Revise', schema: DOC_SCHEMA })
}))

return { theme: THEME, brief: brief.brief_path, uncovered: brief.finding_ids_uncovered, docs: revised.filter(Boolean).map((d) => ({ path: d.doc_path, lines: d.line_count, findings: d.finding_ids_cited.length, contracts: d.contract_ids_used })), critique_counts: critiques.map((c) => (c || []).length) }
