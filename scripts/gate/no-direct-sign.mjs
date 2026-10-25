#!/usr/bin/env node
// 署名関数の直呼び lint (SECURITY-M01)。repo の C/C++ を歩き、封筒以外での署名呼び出しを拒否する。
//   node scripts/gate/no-direct-sign.mjs
import { readFileSync, readdirSync } from 'node:fs';
import { extname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { SOURCE_EXTENSIONS, findDirectSign } from './lib/direct-sign.mjs';

const root = fileURLToPath(new URL('../..', import.meta.url));
const SKIP = new Set(['.git', 'node_modules', 'target', '_deps', 'data', 'review']);

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

const files = walk(root, []);
const violations = findDirectSign(files);
for (const v of violations) console.error(`no-direct-sign: ${v.path}:${v.line} calls ${v.call} outside the envelope module`);
if (violations.length) process.exit(1);
console.log(`no-direct-sign: ok (${files.length} C/C++ files)`);
