// ディレクトリ 1 つを単一ファイルの bundle にする / bundle から復元する。
// 形式: 1 行目 = JSON ヘッダ (magic, format, service, files[{path,size,sha256}]) + "\n"、
// 以降 = files の順に各ファイルの生バイトを連結。大きな保管物も逐次読み書きする。
import { createHash } from 'node:crypto';
import { createReadStream, createWriteStream } from 'node:fs';
import { mkdir, open, readdir, rename, rm, stat } from 'node:fs/promises';
import path from 'node:path';
import { once } from 'node:events';
import { pipeline } from 'node:stream/promises';


/** 連結先を閉じずに 1 ファイル分を書き足す (pipeline の end:false はリスナーが積み上がるため使わない)。 */
async function appendFile(out, file) {
  for await (const chunk of createReadStream(file)) {
    if (!out.write(chunk)) await once(out, 'drain');
  }
}

export const MAGIC = 'synergos-site-bundle';
export const FORMAT = 1;
const MAX_HEADER_BYTES = 64 * 1024 * 1024;

async function sha256File(file) {
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(file)) hash.update(chunk);
  return hash.digest('hex');
}

async function listFiles(root, rel = '') {
  const entries = await readdir(path.join(root, rel), { withFileTypes: true });
  const files = [];
  for (const entry of entries.sort((a, b) => a.name.localeCompare(b.name))) {
    const child = rel ? `${rel}/${entry.name}` : entry.name;
    if (entry.isDirectory()) files.push(...await listFiles(root, child));
    else if (entry.isFile()) files.push(child);
    else throw new Error(`unsupported entry in data directory: ${child}`);
  }
  return files;
}

/** 復元先に書いてよい相対パスだけを通す (絶対パス・ドライブ指定・.. を拒否)。 */
export function assertSafeRelative(rel) {
  if (typeof rel !== 'string' || rel === '' || rel.includes('\\') || rel.includes('\0')
    || path.posix.isAbsolute(rel) || /^[a-zA-Z]:/.test(rel)
    || rel.split('/').some(part => part === '' || part === '.' || part === '..')) {
    throw new Error(`unsafe path in bundle: ${JSON.stringify(rel)}`);
  }
}

/** @param {{ exclude?: string[] }} [options] exclude は data 直下の相対パス (例: 起動時に生成する設定)。 */
export async function writeBundle(sourceDir, output, service, { exclude = [] } = {}) {
  const exists = await stat(sourceDir).then(s => s.isDirectory(), () => false);
  const names = (exists ? await listFiles(sourceDir) : []).filter(rel => !exclude.includes(rel));
  const files = [];
  for (const rel of names) {
    const full = path.join(sourceDir, ...rel.split('/'));
    files.push({ path: rel, size: (await stat(full)).size, sha256: await sha256File(full) });
  }
  const out = createWriteStream(output, { flags: 'wx' });
  await once(out, 'open');
  out.write(JSON.stringify({ magic: MAGIC, format: FORMAT, service, files }) + '\n');
  for (const file of files) {
    await appendFile(out, path.join(sourceDir, ...file.path.split('/')));
  }
  await new Promise((resolve, reject) => out.end(error => (error ? reject(error) : resolve())));
  return { files: files.length, bytes: files.reduce((sum, f) => sum + f.size, 0) };
}

async function readHeader(input) {
  const handle = await open(input, 'r');
  try {
    const chunks = [];
    let offset = 0;
    for (;;) {
      const buffer = Buffer.alloc(64 * 1024);
      const { bytesRead } = await handle.read(buffer, 0, buffer.length, offset);
      if (bytesRead === 0) throw new Error('bundle header is not terminated');
      const newline = buffer.subarray(0, bytesRead).indexOf(0x0a);
      if (newline >= 0) {
        chunks.push(buffer.subarray(0, newline));
        const header = JSON.parse(Buffer.concat(chunks).toString('utf8'));
        return { header, bodyOffset: offset + newline + 1 };
      }
      chunks.push(buffer.subarray(0, bytesRead));
      offset += bytesRead;
      if (offset > MAX_HEADER_BYTES) throw new Error('bundle header is too large');
    }
  } finally {
    await handle.close();
  }
}

export function validateHeader(header, service) {
  if (header?.magic !== MAGIC || header.format !== FORMAT) throw new Error('not a supported Synergos site bundle');
  if (header.service !== service) throw new Error(`bundle belongs to ${header.service}, not ${service}`);
  if (!Array.isArray(header.files)) throw new Error('bundle file list is missing');
  const seen = new Set();
  for (const file of header.files) {
    assertSafeRelative(file.path);
    const key = file.path.toLowerCase();
    if (seen.has(key)) throw new Error(`duplicate path in bundle: ${file.path}`);
    seen.add(key);
    if (!Number.isSafeInteger(file.size) || file.size < 0 || !/^[0-9a-f]{64}$/.test(file.sha256)) {
      throw new Error(`invalid file entry: ${file.path}`);
    }
  }
}

/**
 * bundle 全体の sha256 を確かめ、空の復元先へだけ展開する。
 * 一時ディレクトリへ書き切ってから rename するので、途中で失敗しても復元先は作られない。
 */
export async function readBundle(input, expectedSha256, targetDir, service) {
  if (await sha256File(input) !== expectedSha256) throw new Error('bundle sha256 does not match');
  const { header, bodyOffset } = await readHeader(input);
  validateHeader(header, service);
  const existing = await readdir(targetDir).catch(error => (error.code === 'ENOENT' ? null : Promise.reject(error)));
  if (existing && existing.length > 0) throw new Error(`refusing to import into non-empty ${targetDir}`);
  const staging = `${targetDir}.import-${process.pid}`;
  await rm(staging, { recursive: true, force: true });
  try {
    let offset = bodyOffset;
    for (const file of header.files) {
      const destination = path.join(staging, ...file.path.split('/'));
      await mkdir(path.dirname(destination), { recursive: true });
      if (file.size > 0) {
        await pipeline(createReadStream(input, { start: offset, end: offset + file.size - 1 }), createWriteStream(destination, { flags: 'wx' }));
      } else {
        await (await open(destination, 'wx')).close();
      }
      if (await sha256File(destination) !== file.sha256) throw new Error(`content mismatch: ${file.path}`);
      offset += file.size;
    }
    if (offset !== (await stat(input)).size) throw new Error('bundle has trailing or missing bytes');
    await mkdir(path.dirname(targetDir), { recursive: true });
    if (existing) await rm(targetDir, { recursive: true });
    await mkdir(staging, { recursive: true });
    await rename(staging, targetDir);
  } catch (error) {
    await rm(staging, { recursive: true, force: true });
    throw error;
  }
  return { files: header.files.length };
}
