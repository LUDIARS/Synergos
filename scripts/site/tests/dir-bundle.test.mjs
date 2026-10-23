import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, writeFile, readdir } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { assertSafeRelative, readBundle, validateHeader, writeBundle, MAGIC } from '../lib/dir-bundle.mjs';

const sha = async file => createHash('sha256').update(await readFile(file)).digest('hex');
const tmp = () => mkdtemp(path.join(os.tmpdir(), 'sy-bundle-test-'));

test('round-trips nested files, empty files and excludes generated config', async () => {
  const dir = await tmp();
  const source = path.join(dir, 'src');
  await mkdir(path.join(source, 'custody', 'objects'), { recursive: true });
  await writeFile(path.join(source, 'control-store.json'), '{"orgs":[]}');
  await writeFile(path.join(source, 'custody', 'objects', 'a.data'), Buffer.alloc(200_000, 7));
  await writeFile(path.join(source, 'custody', 'empty'), '');
  await writeFile(path.join(source, 'control.toml'), 'secret-ish');
  const bundle = path.join(dir, 'out.bundle');
  const summary = await writeBundle(source, bundle, 'synergos-control', { exclude: ['control.toml'] });
  assert.equal(summary.files, 3);

  const target = path.join(dir, 'restored');
  await readBundle(bundle, await sha(bundle), target, 'synergos-control');
  assert.deepEqual((await readdir(target)).sort(), ['control-store.json', 'custody']);
  assert.deepEqual(await readFile(path.join(target, 'custody', 'objects', 'a.data')), Buffer.alloc(200_000, 7));
  assert.equal((await readFile(path.join(target, 'custody', 'empty'))).length, 0);
});

test('exports an explicit empty bundle when no data exists', async () => {
  const dir = await tmp();
  const bundle = path.join(dir, 'empty.bundle');
  assert.equal((await writeBundle(path.join(dir, 'missing'), bundle, 'synergos-control')).files, 0);
  await readBundle(bundle, await sha(bundle), path.join(dir, 'restored'), 'synergos-control');
  assert.deepEqual(await readdir(path.join(dir, 'restored')), []);
});

test('refuses a wrong hash, another service and a non-empty target', async () => {
  const dir = await tmp();
  const source = path.join(dir, 'src');
  await mkdir(source);
  await writeFile(path.join(source, 'f'), 'x');
  const bundle = path.join(dir, 'b.bundle');
  await writeBundle(source, bundle, 'synergos-control');
  const hash = await sha(bundle);
  await assert.rejects(readBundle(bundle, '0'.repeat(64), path.join(dir, 'r1'), 'synergos-control'), /sha256/);
  await assert.rejects(readBundle(bundle, hash, path.join(dir, 'r2'), 'curare'), /belongs to/);
  const occupied = path.join(dir, 'r3');
  await mkdir(occupied);
  await writeFile(path.join(occupied, 'keep'), 'mine');
  await assert.rejects(readBundle(bundle, hash, occupied, 'synergos-control'), /non-empty/);
  assert.equal(await readFile(path.join(occupied, 'keep'), 'utf8'), 'mine');
  await assert.rejects(writeBundle(source, bundle, 'synergos-control'), /EEXIST/);
});

test('rejects traversal, absolute and drive paths in headers', () => {
  for (const bad of ['../x', 'a/../../x', '/etc/passwd', 'C:/x', 'a\\b', '', 'a//b', './a']) {
    assert.throws(() => assertSafeRelative(bad), /unsafe path/, bad);
  }
  assert.throws(() => validateHeader({ magic: MAGIC, format: 1, service: 's', files: [
    { path: 'A', size: 0, sha256: '0'.repeat(64) }, { path: 'a', size: 0, sha256: '0'.repeat(64) },
  ] }, 's'), /duplicate/);
});
