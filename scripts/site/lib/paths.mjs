// Site 運用で使うパスの解決 (repo 直下の data/site を既定の保存先にする)。
import { fileURLToPath } from 'node:url';
import path from 'node:path';

export const SERVICE = 'synergos-control';
export const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..', '..');

/** 永続データの置き場。SYNERGOS_SITE_DATA_DIR で上書きできる (絶対パスのみ)。 */
export function dataDir(env = process.env) {
  const configured = env.SYNERGOS_SITE_DATA_DIR;
  if (configured) {
    if (!path.isAbsolute(configured)) throw new Error('SYNERGOS_SITE_DATA_DIR must be an absolute path');
    return configured;
  }
  return path.join(repoRoot, 'data', 'site');
}

export function controlBinary(platform = process.platform) {
  return path.join(repoRoot, 'target', 'release', platform === 'win32' ? 'synergos-control.exe' : 'synergos-control');
}

export function coreBinary(platform = process.platform) {
  return path.join(repoRoot, 'target', 'release', platform === 'win32' ? 'synergos-core.exe' : 'synergos-core');
}
