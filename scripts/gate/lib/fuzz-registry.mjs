// fuzz 登録ゲートの判定 (SPEC-SY-decode-bounds、AT-SY-SEC-22 の登録網羅)。ファイル入出力を持たない純関数だけを置く。
const FIELDS = ['decoder_id', 'protocol', 'version', 'kind', 'max_len', 'max_items', 'target', 'corpus_dir', 'domain'];
const DECODER_TAG = /^\s*\/\/\s*@decoder\s+([a-z0-9_.-]+)\s*$/gm;

/** files: [{ path, text }] → [{ id, path }] (`// @decoder <id>` の目印) */
export function taggedDecoders(files) {
  const out = [];
  for (const { path, text } of files) for (const m of text.matchAll(DECODER_TAG)) out.push({ id: m[1], path });
  return out;
}

/**
 * registry: fuzz/registry.json の中身
 * repo: { exists(path), corpusCount(dir), targetText(path), cmakeText, domains: string[] }
 * tags: taggedDecoders の結果
 * → 食い違いの一覧 (空なら合格)
 */
export function checkFuzzRegistry(registry, repo, tags) {
  const errors = [];
  const ids = new Set();
  for (const entry of registry.decoders ?? []) {
    const id = entry.decoder_id;
    for (const field of FIELDS) if (!(field in entry)) errors.push(`${id ?? '?'}: missing field ${field}`);
    if (typeof id !== 'string' || id === '') continue;
    if (ids.has(id)) errors.push(`${id}: registered twice`);
    ids.add(id);
    if (!Number.isInteger(entry.version) || entry.version < 1) errors.push(`${id}: version must be a positive integer`);
    if (!Number.isInteger(entry.max_len) || entry.max_len < 1) errors.push(`${id}: max_len must be a positive integer`);
    if (entry.max_items !== null && (!Number.isInteger(entry.max_items) || entry.max_items < 1)) {
      errors.push(`${id}: max_items must be a positive integer or null`);
    }
    if (entry.target !== `fuzz/targets/${id}.cpp`) errors.push(`${id}: target must be fuzz/targets/${id}.cpp`);
    else if (!repo.exists(entry.target)) errors.push(`${id}: target file is missing`);
    else if (!/LLVMFuzzerTestOneInput/.test(repo.targetText(entry.target))) errors.push(`${id}: target does not define LLVMFuzzerTestOneInput`);
    if (entry.corpus_dir !== `fuzz/${id}`) errors.push(`${id}: corpus_dir must be fuzz/${id}`);
    else if (repo.corpusCount(entry.corpus_dir) === 0) errors.push(`${id}: corpus is empty`);
    if (!repo.domains.includes(entry.domain)) errors.push(`${id}: unknown domain ${entry.domain}`);
    if (!repo.cmakeText.includes(`synergos_fuzz_target(${id} `)) errors.push(`${id}: not built by fuzz/CMakeLists.txt`);
  }
  const tagged = new Set();
  for (const { id, path } of tags) {
    tagged.add(id);
    if (!ids.has(id)) errors.push(`${id}: decoder in ${path} is not registered in fuzz/registry.json`);
  }
  for (const id of ids) if (!tagged.has(id)) errors.push(`${id}: no source carries // @decoder ${id}`);
  return errors;
}
