import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

// Execute the archived workflow with deterministic reviewer responses. No model,
// filesystem-writing collector or subprocess is run by this harness.
const source = (await readFile(new URL('../find-verify.js', import.meta.url), 'utf8'))
  .replace('export const meta =', 'const meta =');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const workflow = new AsyncFunction('args', 'phase', 'agent', 'parallel', 'pipeline', 'log', source);

async function run(refute, judge) {
  const finding = { id: 'fixture-1', title: 'fixture', severity: 'high' };
  const agent = async (_prompt, options) => {
    if (options.label.startsWith('find:')) return { findings: [finding] };
    if (options.label.startsWith('refute:')) return refute;
    if (options.label.startsWith('judge:')) return judge;
    if (options.label.startsWith('collect:')) return 'written';
    throw new Error('Unexpected workflow role');
  };
  const parallel = tasks => Promise.all(tasks.map(task => task()));
  const pipeline = (items, stage) => Promise.all(items.map((item, index) => stage(item, item, index)));
  return workflow({ theme: 'security' }, () => {}, agent, parallel, pipeline, () => {});
}
const judged = { verdicts: [{ id: 'fixture-1', material: true, priority: 'P1', reason: 'supported' }] };
const confirmed = { verdicts: [{ id: 'fixture-1', verdict: 'confirmed', reason: 'evidence holds' }] };

test('missing refuter never enters verified design inputs', async () => {
  const result = await run(null, judged);
  assert.equal(result.kept.length, 0);
  assert.equal(result.dropped[0].verdict, 'unverified');
});
test('missing judge never enters verified design inputs', async () => {
  assert.equal((await run(confirmed, null)).kept.length, 0);
});
test('reviewer response for a different finding is not evidence', async () => {
  assert.equal((await run({ verdicts: [{ id: 'different', verdict: 'confirmed' }] }, judged)).kept.length, 0);
});
test('explicit confirmed and partially verified findings remain usable', async () => {
  assert.equal((await run(confirmed, judged)).kept.length, 1);
  assert.equal((await run({ verdicts: [{ id: 'fixture-1', verdict: 'partially' }] }, judged)).kept.length, 1);
});
test('refuted and duplicate findings remain excluded', async () => {
  assert.equal((await run({ verdicts: [{ id: 'fixture-1', verdict: 'refuted' }] }, judged)).kept.length, 0);
  assert.equal((await run(confirmed, { verdicts: [{ ...judged.verdicts[0], duplicate_of: 'other' }] })).kept.length, 0);
});
