// Excubitor の常駐コマンド (synergos-node)。環境変数から synergos-net.toml を作り、
// release ビルドの synergos-core daemon を前面で起動して終了コードとシグナルを中継する。
import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { buildNodeToml } from './lib/node-config.mjs';
import { coreBinary, dataDir } from './lib/paths.mjs';

const binary = coreBinary();
if (!existsSync(binary)) {
  console.error('run-node: synergos-core is not built; run the Excubitor setup (scripts/site/setup.mjs) first');
  process.exit(1);
}
let toml;
try {
  toml = buildNodeToml(process.env);
} catch (error) {
  console.error(`run-node: ${error.message}`);
  process.exit(1);
}
const nodeDir = path.join(dataDir(), 'node');
await mkdir(nodeDir, { recursive: true });
const configPath = path.join(nodeDir, 'synergos-net.toml');
await writeFile(configPath, toml, { mode: 0o600 });

const child = spawn(binary, ['start', '--config', configPath], { stdio: 'inherit', env: process.env });
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => child.kill(signal));
child.on('error', error => {
  console.error(`run-node: failed to start synergos-core (${error.code ?? error.message})`);
  process.exit(1);
});
child.on('exit', (code, signal) => process.exit(code ?? (signal ? 1 : 0)));
