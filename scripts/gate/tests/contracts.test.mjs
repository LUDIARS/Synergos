import assert from 'node:assert/strict';
import test from 'node:test';
import { buildRegistry, checkRegistry, extractDefinitions, hashDefinition, referencedIds } from '../lib/contracts.mjs';

const plan = {
  path: 'spec/plan/a.md',
  text: [
    '### 4.1 envelope',
    '- 契約 SPEC-SY-alpha (正本: 本書):',
    '',
    '  > Every signature is fixed.',
    '  > Second line.',
    '',
    '- uses SPEC-CU-beta and SPEC-SY-gamma, not SPEC-SY-*',
  ].join('\r\n'),
};
const feature = {
  path: 'spec/feature/f.md',
  text: '# Contracts\n\n## [id: SPEC-SY-delta] Delta\nBody of delta.\n### detail\nmore\n## [id: SPEC-SY-eps] Eps\nE.\n',
};

test('extracts plan blockquotes and feature sections', () => {
  const defs = extractDefinitions([plan, feature]);
  assert.deepEqual(defs.map((d) => [d.id, d.section, d.text]), [
    ['SPEC-SY-alpha', '4.1 envelope', 'Every signature is fixed.\nSecond line.'],
    ['SPEC-SY-delta', '[id: SPEC-SY-delta] Delta', 'Body of delta.\n### detail\nmore'],
    ['SPEC-SY-eps', '[id: SPEC-SY-eps] Eps', 'E.'],
  ]);
});

test('collects referenced ids without wildcards', () => {
  assert.deepEqual(referencedIds([plan]), ['SPEC-CU-beta', 'SPEC-SY-alpha', 'SPEC-SY-gamma']);
});

test('a registry built from the documents passes', () => {
  const files = [plan, feature];
  const defs = extractDefinitions(files);
  const refs = referencedIds(files);
  const registry = buildRegistry(defs, refs);
  assert.deepEqual(checkRegistry(registry, defs, refs), []);
  const statuses = Object.fromEntries(registry.contracts.map((c) => [c.id, c.status]));
  assert.deepEqual(statuses, {
    'SPEC-CU-beta': 'external', 'SPEC-SY-alpha': 'defined', 'SPEC-SY-delta': 'defined', 'SPEC-SY-eps': 'defined', 'SPEC-SY-gamma': 'planned',
  });
});

test('an edited normative text fails until the registry is rewritten', () => {
  const files = [plan, feature];
  const registry = buildRegistry(extractDefinitions(files), referencedIds(files));
  const edited = [{ ...plan, text: plan.text.replace('fixed', 'loose') }, feature];
  const errors = checkRegistry(registry, extractDefinitions(edited), referencedIds(edited));
  assert.deepEqual(errors, ['SPEC-SY-alpha: definition_sha256 does not match the normative text']);
});

test('a second normative copy and an unregistered reference fail', () => {
  const files = [plan, feature];
  const registry = buildRegistry(extractDefinitions(files), referencedIds(files));
  const copy = { path: 'docs/copy.md', text: '- 契約 SPEC-SY-alpha (正本: 本書):\n  > Every signature is fixed.\n\nSPEC-CR-new' };
  const withCopy = [...files, copy];
  const errors = checkRegistry(registry, extractDefinitions(withCopy), referencedIds(withCopy));
  assert.ok(errors.some((e) => e.startsWith('SPEC-SY-alpha: normative text appears 2 times')));
  assert.ok(errors.includes('SPEC-CR-new: referenced but not registered'));
});

test('hash is over the exact definition text', () => {
  assert.equal(hashDefinition('a'), 'ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb');
});
