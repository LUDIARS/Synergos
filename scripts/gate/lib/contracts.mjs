// 契約登録ゲートの判定 (redesign-security §4.1 SECURITY-M01)。ファイル入出力を持たない純関数だけを置く。
//
// 規範文の置き方は 2 通り:
//   計画文書: `- 契約 SPEC-SY-x (正本: …)` の行の後に続く英語の引用ブロック (`> …`)
//   feature 文書: `## [id: SPEC-SY-x] …` の見出しから、同じか上位の次の見出しまで
import { createHash } from 'node:crypto';

const ID = /SPEC-[A-Z]{2}-[a-z0-9]+(?:-[a-z0-9]+)*/g;
const PLAN_MARKER = /^\s*- 契約 (SPEC-SY-[a-z0-9]+(?:-[a-z0-9]+)*) \(正本/;
const FEATURE_HEADING = /^(#{1,6}) \[id: (SPEC-[A-Z]{2}-[a-z0-9]+(?:-[a-z0-9]+)*)\]/;
const HEADING = /^(#{1,6}) (.*)$/;
const OWNERS = { SY: 'Synergos', CU: 'Curare', CR: 'Cernere' };

function lines(text) {
  return text.replace(/\r\n/g, '\n').split('\n').map((line) => line.replace(/\s+$/, ''));
}

export function hashDefinition(text) {
  return createHash('sha256').update(text, 'utf8').digest('hex');
}

/** files: [{ path, text }] → [{ id, path, section, text }] (規範文の出現をすべて返す。重複の判定は呼び出し側) */
export function extractDefinitions(files) {
  const out = [];
  for (const { path, text } of files) {
    const rows = lines(text);
    let section = null;
    for (let i = 0; i < rows.length; i += 1) {
      const heading = HEADING.exec(rows[i]);
      const feature = FEATURE_HEADING.exec(rows[i]);
      if (feature) {
        const level = feature[1].length;
        const body = [];
        let j = i + 1;
        for (; j < rows.length; j += 1) {
          const next = HEADING.exec(rows[j]);
          if (next && next[1].length <= level) break;
          body.push(rows[j]);
        }
        out.push({ id: feature[2], path, section: heading[2], text: body.join('\n').trim() });
      }
      if (heading) { section = heading[2]; continue; }
      const marker = PLAN_MARKER.exec(rows[i]);
      if (!marker) continue;
      let j = i + 1;
      while (j < rows.length && rows[j].trim() === '') j += 1;
      const quote = [];
      for (; j < rows.length; j += 1) {
        const m = /^\s*>\s?(.*)$/.exec(rows[j]);
        if (!m) break;
        quote.push(m[1]);
      }
      out.push({ id: marker[1], path, section, text: quote.join('\n').trim() });
    }
  }
  return out;
}

/** 文書が参照する契約 ID (重複なし、整列済み) */
export function referencedIds(files) {
  const ids = new Set();
  for (const { text } of files) for (const m of text.matchAll(ID)) ids.add(m[0]);
  return [...ids].sort();
}

function ownerOf(id) {
  return OWNERS[id.slice(5, 7)] ?? null;
}

/** 文書から登録表を組み立てる (`--write` 用)。規範文が複数あれば例外 (先に直す)。 */
export function buildRegistry(definitions, references) {
  const byId = new Map();
  for (const d of definitions) {
    if (byId.has(d.id)) throw new Error(`${d.id}: normative text appears more than once`);
    byId.set(d.id, d);
  }
  const ids = [...new Set([...references, ...byId.keys()])].sort();
  return {
    version: 1,
    contracts: ids.map((id) => {
      const owner = ownerOf(id);
      const d = byId.get(id);
      if (d) return { id, owner_repo: owner, status: 'defined', owner_doc: d.path, owner_section: d.section, definition_sha256: hashDefinition(d.text) };
      if (owner === 'Synergos') return { id, owner_repo: owner, status: 'planned', owner_doc: null, owner_section: null, definition_sha256: null };
      return { id, owner_repo: owner, status: 'external', owner_doc: null, owner_section: null, definition_sha256: null };
    }),
  };
}

/** 登録表と文書の食い違いを返す (空なら合格)。 */
export function checkRegistry(registry, definitions, references) {
  const errors = [];
  const counts = new Map();
  for (const d of definitions) counts.set(d.id, [...(counts.get(d.id) ?? []), d.path]);
  for (const [id, paths] of counts) {
    if (paths.length > 1) errors.push(`${id}: normative text appears ${paths.length} times (${paths.join(', ')})`);
  }
  const defined = new Map(definitions.map((d) => [d.id, d]));
  const entries = new Map();
  for (const entry of registry.contracts ?? []) {
    if (entries.has(entry.id)) errors.push(`${entry.id}: registered twice`);
    entries.set(entry.id, entry);
  }
  for (const id of references) if (!entries.has(id)) errors.push(`${id}: referenced but not registered`);
  for (const [id, entry] of entries) {
    const owner = ownerOf(id);
    if (entry.owner_repo !== owner) errors.push(`${id}: owner_repo must be ${owner}`);
    const d = defined.get(id);
    if (d && d.text === '') errors.push(`${id}: normative marker without a definition (${d.path})`);
    if (entry.status === 'defined') {
      if (!d) { errors.push(`${id}: registered as defined but no normative text was found`); continue; }
      if (entry.owner_doc !== d.path) errors.push(`${id}: owner_doc is ${entry.owner_doc} but the text is in ${d.path}`);
      if (entry.owner_section !== d.section) errors.push(`${id}: owner_section differs (${d.section})`);
      if (entry.definition_sha256 !== hashDefinition(d.text)) errors.push(`${id}: definition_sha256 does not match the normative text`);
    } else if (d) {
      errors.push(`${id}: normative text exists but the entry is ${entry.status}`);
    } else if (entry.status === 'planned' && owner !== 'Synergos') {
      errors.push(`${id}: only Synergos contracts can be planned here`);
    } else if (!['planned', 'external'].includes(entry.status)) {
      errors.push(`${id}: unknown status ${entry.status}`);
    }
    if (entry.status !== 'defined' && !references.includes(id)) errors.push(`${id}: registered but neither defined nor referenced`);
  }
  return errors;
}
