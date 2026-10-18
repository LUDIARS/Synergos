export const meta = {
  name: 'implement-tasks',
  description: 'One Opus implementer per task in its own git worktree (edit, test, commit), then an independent verifier re-runs checks and reviews the diff',
  phases: [
    { title: 'Implement', detail: 'Opus implementer per task in isolated worktree', model: 'opus' },
    { title: 'Verify', detail: 'independent verifier reruns checks and reviews the diff' },
  ],
}

const SCRATCH = 'spec/plan/redesign-analysis'
const tasks = args && args.tasks
if (!Array.isArray(tasks) || tasks.length === 0) throw new Error('args.tasks[] required')

const COMMON = `CONTEXT
LUDIARS org, two repos forming a version-control platform for game/AI assets: Synergos (Rust workspace, Synergos main) and Curare (TypeScript Hono server, Curare main). Org rules: AIFormat (RULE.md: Cernere-central auth, migrations idempotent and additive only, never DROP, no personal data columns; RULE_CODE.md conventions; RULE_TEST.md; HARNESS.md §3.7: never swallow errors, no silent fallbacks). Subsystem maps: ${SCRATCH}/maps/*.md. Verified findings: ${SCRATCH}/findings/*-verified.json. Design briefs (if present): ${SCRATCH}/design/*-brief.md.
BUILD/TEST ENVIRONMENT
- Rust: use the shared target dir: export CARGO_TARGET_DIR=<configured-build-cache> (always set it; builds are cached there). The Tauri crate cannot build here (no GTK): exclude it, e.g. cargo test --workspace --exclude synergos-tauri, cargo clippy --workspace --exclude synergos-tauri --all-targets -- -D warnings, cargo fmt --all -- --check. CI runs RUSTFLAGS=-D warnings, clippy -D warnings, fmt check, tests on Linux/macOS/Windows: keep code portable (no unix-only APIs without cfg).
- TypeScript: cd into <worktree>/curare-server; pnpm install --no-frozen-lockfile (node_modules may already exist); pnpm build (tsc) and pnpm test (vitest) must pass. The dependency @ludiars/cernere-id-cache resolves via file:../../Cernere/packages/id-cache, which exists as a symlink beside the worktrees. Integration tests needing PostgreSQL/MinIO are skipped when services are absent; do not try to start services.
- Do not touch files outside your worktree. Do not push. Do not modify main checkouts. Commit on your branch only.`

const RESULT_SCHEMA = {
  type: 'object',
  properties: {
    task_id: { type: 'string' },
    status: { type: 'string', enum: ['done', 'partial', 'blocked'] },
    branch: { type: 'string' },
    commits: { type: 'array', items: { type: 'string' } },
    files_changed: { type: 'array', items: { type: 'string' } },
    checks_run: { type: 'array', items: { type: 'object', properties: { command: { type: 'string' }, result: { type: 'string' } }, required: ['command', 'result'] } },
    tests_added: { type: 'array', items: { type: 'string' } },
    summary_ja: { type: 'string' },
    deviations: { type: 'string' },
    followups: { type: 'array', items: { type: 'string' } },
  },
  required: ['task_id', 'status', 'branch', 'commits', 'files_changed', 'checks_run', 'summary_ja'],
}
const VERIFY_SCHEMA = {
  type: 'object',
  properties: {
    task_id: { type: 'string' },
    verdict: { type: 'string', enum: ['pass', 'fix-required', 'fail'] },
    checks: { type: 'array', items: { type: 'object', properties: { command: { type: 'string' }, result: { type: 'string' } }, required: ['command', 'result'] } },
    review_findings: { type: 'array', items: { type: 'object', properties: { severity: { type: 'string', enum: ['blocker', 'major', 'minor'] }, file_line: { type: 'string' }, problem: { type: 'string' }, fix: { type: 'string' } }, required: ['severity', 'file_line', 'problem', 'fix'] } },
    summary_ja: { type: 'string' },
  },
  required: ['task_id', 'verdict', 'checks', 'review_findings', 'summary_ja'],
}

const implPrompt = (t, fixes) => `${COMMON}

YOUR TASK: ${t.id} — ${t.title}
Repository: ${t.repo}   Theme branch base: ${t.base_branch}
Your worktree (already created, on branch ${t.branch}): ${t.worktree}
Specification:
${t.spec}
Scope (files/modules you are expected to touch; stay inside unless strictly necessary, and say so if you must):
${t.scope}
Acceptance criteria (all must hold):
${t.acceptance}
Related finding ids: ${(t.finding_ids || []).join(', ')}
${t.notes ? `Notes: ${t.notes}` : ''}

WORKING RULES
1. Read the related findings (in ${SCRATCH}/findings/*-verified.json) and the relevant map(s) first; then read the code you will change in full.
2. Implement the full feature as specified (no MVP shortcuts, HARNESS §3.1), with tests in the same change (unit tests; integration tests only if they run without external services). Keep the change minimal in scope but complete in function. Follow existing code style, module layout and naming (1 file 1 responsibility; repositories for DB access; Zod validation at routes; tracing/structured logs; never swallow errors).
3. Update documentation that the change makes stale (spec/feature, docs/*.md, .env.example, config examples) in the same commit.
4. Run the checks for the repo (Rust: fmt --check, clippy -D warnings excluding synergos-tauri, tests for the crates you changed and then the workspace; TS: pnpm build, pnpm test). Fix everything until green. Record each command and its result.
5. Commit on branch ${t.branch} with a conventional Japanese commit message like the repo history (e.g. "feat(custody): 署名URLを単回使用nonceと呼出主体に束縛する"), body explaining the why and referencing finding ids. Multiple logical commits are fine. End the commit message body with these two lines exactly:
Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01V6yGvn99C1KtBomR5ipYKo
   (git user is preconfigured.) Do NOT include any model identifier other than that line.
6. If something in the spec is impossible or would break an invariant, implement the sound subset, explain in 'deviations', and list follow-ups. Never disable or skip existing tests to get green.
${fixes ? `\nFIX PASS: a verifier reviewed your work and requires these fixes. Apply them, re-run checks, and commit again:\n${fixes}\n` : ''}
Return the structured result.`

phase('Implement')
const results = await pipeline(tasks,
  (t) => agent(implPrompt(t, null), { label: `impl:${t.id}`, phase: 'Implement', model: 'opus', schema: RESULT_SCHEMA }),
  (r, t) => r ? agent(`${COMMON}

ROLE: independent VERIFIER for task ${t.id} — ${t.title} (repo ${t.repo}). Worktree: ${t.worktree}, branch ${t.branch}, base ${t.base_branch}.
Implementer's report: ${JSON.stringify(r)}
Do this:
1. git -C ${t.worktree} log --oneline ${t.base_branch}..HEAD and git diff ${t.base_branch}...HEAD — read the full diff.
2. Re-run the repo checks yourself (Rust: CARGO_TARGET_DIR=<configured-build-cache> cargo fmt --all -- --check; cargo clippy --workspace --exclude synergos-tauri --all-targets -- -D warnings; cargo test --workspace --exclude synergos-tauri. TS: pnpm build && pnpm test in curare-server). Record results.
3. Review the diff against the specification and acceptance criteria:
${t.acceptance}
   Check security correctness (no new bypass, fail-closed, no secrets in logs), Windows/macOS portability, idempotent/additive migrations, no DROP, no personal-data columns, errors not swallowed, no silent fallbacks, tests meaningful (not tautological), docs updated, commit message format (Japanese conventional + the two trailer lines).
4. verdict: pass (mergeable as is), fix-required (list concrete blockers/majors), fail (approach wrong). Do not edit code yourself.`,
    { label: `verify:${t.id}`, phase: 'Verify', schema: VERIFY_SCHEMA }).then((v) => ({ impl: r, verify: v })) : null,
  async (pair, t) => {
    if (!pair) return null
    const v = pair.verify
    if (!v || v.verdict === 'pass') return { ...pair, fixed: null }
    if (v.verdict === 'fail') return { ...pair, fixed: null }
    const text = v.review_findings.filter((f) => f.severity !== 'minor').map((f, n) => `${n + 1}. [${f.severity}] ${f.file_line}: ${f.problem}\n   FIX: ${f.fix}`).join('\n')
    if (!text) return { ...pair, fixed: null }
    log(`${t.id}: fix pass (${v.review_findings.length} findings)`)
    const fixed = await agent(implPrompt(t, text), { label: `fix:${t.id}`, phase: 'Implement', model: 'opus', schema: RESULT_SCHEMA })
    const reverify = await agent(`${COMMON}

ROLE: VERIFIER (second pass) for task ${t.id} — ${t.title}. Worktree ${t.worktree}, branch ${t.branch}, base ${t.base_branch}. Previous verdict: ${JSON.stringify(v)}. Implementer's fix report: ${JSON.stringify(fixed)}.
Re-run the checks (same commands as before) and confirm each previous blocker/major is resolved by reading the new diff. Return the structured verdict.`, { label: `reverify:${t.id}`, phase: 'Verify', schema: VERIFY_SCHEMA })
    return { ...pair, fixed, verify: reverify || v }
  }
)

const out = results.map((r, i) => ({ task: tasks[i].id, branch: tasks[i].branch, status: r && r.impl ? r.impl.status : 'missing', verdict: r && r.verify ? r.verify.verdict : 'missing', summary: r && r.impl ? r.impl.summary_ja : '', followups: r && r.impl ? (r.impl.followups || []) : [], deviations: r && r.impl ? r.impl.deviations : '', review: r && r.verify ? r.verify.review_findings : [] }))
log(`implemented: ${out.filter((o) => o.verdict === 'pass').length}/${tasks.length} pass`)
return out
