#!/usr/bin/env node
// 時間を区切った fuzz の実行 (SECURITY-M23)。libFuzzer の実行ファイルは
//   cmake -S . -B build-fuzz -DSYNERGOS_FUZZ=ON (clang、または MSVC の /fsanitize=fuzzer) でビルドしておく。
//   node scripts/gate/fuzz-run.mjs [--seconds 30] [--build-dir <dir>] [--only id,id]
// crash を見つけたら exit 1。再現入力は libFuzzer が crash-* として書くので、該当 decoder の corpus に足す。
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { fuzzPlan } from './lib/fuzz-plan.mjs';

const root = fileURLToPath(new URL('../..', import.meta.url));
const arg = (name, fallback) => {
  const i = process.argv.indexOf(name);
  return i >= 0 ? process.argv[i + 1] : fallback;
};
const windows = process.platform === 'win32';
const buildDir = arg('--build-dir', join(root, 'build-fuzz', 'fuzz', windows ? 'Release' : ''));
const workDir = join(root, 'build-fuzz', 'work');
const only = arg('--only', null)?.split(',') ?? null;
const plan = fuzzPlan(JSON.parse(readFileSync(join(root, 'fuzz', 'registry.json'), 'utf8')), {
  seconds: Number(arg('--seconds', '30')), buildDir, workDir, windows, only,
});

let failed = 0;
for (const step of plan) {
  if (!existsSync(step.binary)) {
    console.error(`fuzz-run: ${step.id}: ${step.binary} is not built (configure with -DSYNERGOS_FUZZ=ON)`);
    failed += 1;
    continue;
  }
  mkdirSync(join(workDir, step.id), { recursive: true });
  // MSVC 14.39 の ASan は新しい Windows で一部関数の差し替えに失敗して起動時に止まる。差し替えできない関数を
  // 飛ばして続けさせる (decoder の検査には影響しない)。
  const env = windows ? { ...process.env, ASAN_WIN_CONTINUE_ON_INTERCEPTION_FAILURE: '1' } : process.env;
  const result = spawnSync(step.binary, step.args, { cwd: root, env, stdio: ['ignore', 'ignore', 'pipe'], encoding: 'utf8' });
  const tail = (result.stderr ?? '').trim().split('\n').slice(-3).join(' | ');
  if (result.status === 0) console.log(`fuzz-run: ${step.id}: ok (${tail})`);
  else {
    failed += 1;
    console.error(`fuzz-run: ${step.id}: exit ${result.status} ${tail}`);
  }
}
if (failed) process.exit(1);
