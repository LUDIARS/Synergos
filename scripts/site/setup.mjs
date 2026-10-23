// Excubitor の setup 入口 (引数なし、再実行可)。synergos-control を release ビルドする。
// 常駐起動は Excubitor が run-control.mjs で行う。既存データ・秘密には触れない。
import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { controlBinary, repoRoot } from './lib/paths.mjs';

const result = spawnSync('cargo', ['build', '--release', '--locked', '-p', 'synergos-control'], {
  cwd: repoRoot,
  stdio: 'inherit',
  env: { ...process.env, CARGO_INCREMENTAL: '0' },
});
if (result.error) {
  console.error(`setup: cargo を起動できません (${result.error.code ?? result.error.message})`);
  process.exit(1);
}
if (result.status !== 0) process.exit(result.status ?? 1);
if (!existsSync(controlBinary())) {
  console.error('setup: build finished but the synergos-control binary is missing');
  process.exit(1);
}
console.log('setup: synergos-control release build ready');
