import assert from 'node:assert/strict';
import test from 'node:test';
import { binaryName, fuzzPlan } from '../lib/fuzz-plan.mjs';

const registry = {
  decoders: [
    { decoder_id: 'trust.lct', max_len: 16384, corpus_dir: 'fuzz/trust.lct' },
    { decoder_id: 'core.manifest', max_len: 67108864, corpus_dir: 'fuzz/core.manifest' },
  ],
};

test('binary names follow fuzz/CMakeLists.txt', () => {
  assert.equal(binaryName('core.version_state', true), 'fuzz_core_version_state.exe');
  assert.equal(binaryName('trust.lct', false), 'fuzz_trust_lct');
});

test('each target writes new inputs to its work dir and reads the registered corpus', () => {
  const plan = fuzzPlan(registry, { seconds: 30, buildDir: 'b', workDir: 'w', windows: false });
  assert.deepEqual(plan[0], {
    id: 'trust.lct', binary: 'b/fuzz_trust_lct', args: ['-max_total_time=30', '-max_len=16385', 'w/trust.lct', 'fuzz/trust.lct'],
  });
  assert.equal(plan[1].args[1], '-max_len=1048576'); // 1 MiB を超える max_len は libFuzzer の入力長を 1 MiB で切る
});

test('the per-merge budget is ten minutes in total', () => {
  assert.throws(() => fuzzPlan(registry, { seconds: 301, buildDir: 'b', workDir: 'w', windows: false }), /exceeds the 600s budget/);
  assert.equal(fuzzPlan(registry, { seconds: 30, buildDir: 'b', workDir: 'w', windows: true, only: ['core.manifest'] }).length, 1);
});
