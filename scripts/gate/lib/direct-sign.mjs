// 署名関数の直呼び検査 (SPEC-SY-signed-envelope: Direct `sign(` calls outside the envelope module fail a lint gate)。
// 純関数だけを置く。C/C++ の署名プリミティブを呼んでよいのは封筒の翻訳単位 1 つだけ。
export const ENVELOPE_UNIT = 'synergos-net/native/trust/envelope.cpp';

const SIGN_CALL = /\b(crypto_ed25519_sign|crypto_eddsa_sign|crypto_sign|ED25519_sign|EVP_DigestSign\w*|EVP_PKEY_sign\w*)\s*\(/g;

export const SOURCE_EXTENSIONS = ['.c', '.cc', '.cpp', '.cxx', '.h', '.hh', '.hpp'];

/** files: [{ path, text }] (path は repo 相対・/ 区切り) → 違反 [{ path, line, call }] */
export function findDirectSign(files) {
  const out = [];
  for (const { path, text } of files) {
    if (path === ENVELOPE_UNIT) continue;
    const rows = text.split(/\r?\n/);
    rows.forEach((row, i) => {
      for (const m of row.matchAll(SIGN_CALL)) out.push({ path, line: i + 1, call: m[1] });
    });
  }
  return out;
}
