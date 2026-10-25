// 時間を区切った fuzz 実行の計画 (SPEC-SY-decode-bounds: マージごとに各 target 30 秒、合計 10 分以内)。純関数。
export const TOTAL_LIMIT_SECONDS = 600;

/** decoder_id → libFuzzer の実行ファイル名 (fuzz/CMakeLists.txt の fuzz_<name>) */
export function binaryName(decoderId, windows) {
  return `fuzz_${decoderId.replace(/\./g, '_')}${windows ? '.exe' : ''}`;
}

/**
 * registry と秒数から実行計画を作る。新しく見つけた入力は work/<id> に書き、登録済みの corpus は読むだけにする。
 * → [{ id, binary, args }]
 */
export function fuzzPlan(registry, { seconds, buildDir, workDir, windows, only = null }) {
  const decoders = (registry.decoders ?? []).filter((d) => !only || only.includes(d.decoder_id));
  if (seconds * decoders.length > TOTAL_LIMIT_SECONDS) {
    throw new Error(`${decoders.length} targets x ${seconds}s exceeds the ${TOTAL_LIMIT_SECONDS}s budget`);
  }
  return decoders.map((d) => ({
    id: d.decoder_id,
    binary: `${buildDir}/${binaryName(d.decoder_id, windows)}`,
    args: [`-max_total_time=${seconds}`, `-max_len=${Math.min(d.max_len + 1, 1 << 20)}`, `${workDir}/${d.decoder_id}`, d.corpus_dir],
  }));
}
