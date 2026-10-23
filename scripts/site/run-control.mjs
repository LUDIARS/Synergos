// Excubitor の常駐コマンド。Vault から注入された環境変数で control.toml を作り、
// release ビルドの synergos-control を前面で起動して終了コードとシグナルを中継する。
import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { buildControlToml } from './lib/control-config.mjs';
import { controlBinary, dataDir } from './lib/paths.mjs';

const binary = controlBinary();
if (!existsSync(binary)) {
  console.error('run-control: synergos-control is not built; run the Excubitor setup (scripts/site/setup.mjs) first');
  process.exit(1);
}
const root = dataDir();
const custodyRoot = process.env.SYNERGOS_CUSTODY_ROOT || path.join(root, 'custody');
await mkdir(custodyRoot, { recursive: true });
const configPath = path.join(root, 'control.toml');
await writeFile(configPath, buildControlToml(process.env, { storePath: path.join(root, 'control-store.json') }), { mode: 0o600 });

const child = spawn(binary, ['serve', '--config', configPath], {
  stdio: 'inherit',
  env: { ...process.env, SYNERGOS_CUSTODY_ROOT: custodyRoot },
});
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => child.kill(signal));
child.on('error', error => {
  console.error(`run-control: failed to start synergos-control (${error.code ?? error.message})`);
  process.exit(1);
});
child.on('exit', (code, signal) => process.exit(code ?? (signal ? 1 : 0)));
