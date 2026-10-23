// Excubitor が渡す --name value 形式の引数を読む。
import path from 'node:path';

export function readArgs(argv, names) {
  const out = {};
  for (let i = 0; i < argv.length; i += 2) {
    const key = argv[i]?.replace(/^--/, '');
    if (!names.includes(key) || argv[i + 1] === undefined) throw new Error(`unexpected argument: ${argv[i]}`);
    out[key] = argv[i + 1];
  }
  for (const name of names) if (!out[name]) throw new Error(`--${name} is required`);
  for (const name of ['output', 'input']) {
    if (out[name] && !path.isAbsolute(out[name])) throw new Error(`--${name} must be an absolute path`);
  }
  if (out.sha256 && !/^[0-9a-f]{64}$/.test(out.sha256)) throw new Error('--sha256 must be 64 lowercase hex characters');
  return out;
}
