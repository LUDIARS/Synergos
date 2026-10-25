#!/usr/bin/env node
// 契約登録ゲート (SECURITY-M01)。spec/contracts/registry.json と文書の規範文を照合する。
//   node scripts/gate/contract-registry.mjs          検査 (食い違いがあれば exit 1)
//   node scripts/gate/contract-registry.mjs --write  文書から登録表を作り直す (規範文を変えた変更で使う)
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { buildRegistry, checkRegistry, extractDefinitions, referencedIds } from './lib/contracts.mjs';

const root = fileURLToPath(new URL('../..', import.meta.url));
const registryPath = join(root, 'spec', 'contracts', 'registry.json');
// 規範文と参照を探す文書。設計の下書き (spec/plan/redesign-analysis/) は対象外。
const docDirs = ['spec/plan', 'spec/feature', 'docs'];

function readDocs() {
  const files = [];
  for (const dir of docDirs) {
    let names = [];
    try { names = readdirSync(join(root, dir)); } catch { continue; }
    for (const name of names.filter((n) => n.endsWith('.md')).sort()) {
      const full = join(root, dir, name);
      files.push({ path: relative(root, full).replace(/\\/g, '/'), text: readFileSync(full, 'utf8') });
    }
  }
  return files;
}

const files = readDocs();
const definitions = extractDefinitions(files);
const references = referencedIds(files);

if (process.argv.includes('--write')) {
  writeFileSync(registryPath, `${JSON.stringify(buildRegistry(definitions, references), null, 2)}\n`);
  console.log(`wrote ${relative(root, registryPath)}`);
} else {
  const errors = checkRegistry(JSON.parse(readFileSync(registryPath, 'utf8')), definitions, references);
  for (const e of errors) console.error(`contract-registry: ${e}`);
  if (errors.length) process.exit(1);
  console.log(`contract-registry: ok (${definitions.length} defined, ${references.length} referenced)`);
}
