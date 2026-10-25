#!/usr/bin/env node
// fuzz 登録ゲート (SECURITY-M23)。`// @decoder <id>` の付いた C/C++ の decoder がすべて fuzz/registry.json にあり、
// 登録ごとに target・空でない corpus・CMake の組み立てがあることを検査する。
//   node scripts/gate/fuzz-registry.mjs
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { extname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { SOURCE_EXTENSIONS } from './lib/direct-sign.mjs';
import { checkFuzzRegistry, taggedDecoders } from './lib/fuzz-registry.mjs';

const root = fileURLToPath(new URL('../..', import.meta.url));
const SKIP = new Set(['.git', 'node_modules', 'target', '_deps', 'data', 'review', 'fuzz']);

function walk(dir, out) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (SKIP.has(entry.name) || entry.name.startsWith('build')) continue;
    const full = join(dir, entry.name);
    if (entry.isDirectory()) walk(full, out);
    else if (SOURCE_EXTENSIONS.includes(extname(entry.name))) {
      out.push({ path: relative(root, full).replace(/\\/g, '/'), text: readFileSync(full, 'utf8') });
    }
  }
  return out;
}

const at = (p) => join(root, p);
const repo = {
  exists: (p) => existsSync(at(p)),
  targetText: (p) => readFileSync(at(p), 'utf8'),
  corpusCount: (dir) => (existsSync(at(dir)) ? readdirSync(at(dir)).length : 0),
  cmakeText: readFileSync(at('fuzz/CMakeLists.txt'), 'utf8'),
  domains: readdirSync(at('spec/domains')).filter((n) => n.endsWith('.domain.json')).map((n) => n.replace(/\.domain\.json$/, '')),
};
const registry = JSON.parse(readFileSync(at('fuzz/registry.json'), 'utf8'));
const tags = taggedDecoders(walk(root, []));
const errors = checkFuzzRegistry(registry, repo, tags);
for (const e of errors) console.error(`fuzz-registry: ${e}`);
if (errors.length) process.exit(1);
console.log(`fuzz-registry: ok (${registry.decoders.length} decoders, ${tags.length} tagged sources)`);
