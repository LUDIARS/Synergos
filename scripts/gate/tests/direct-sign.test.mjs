import assert from 'node:assert/strict';
import test from 'node:test';
import { ENVELOPE_UNIT, findDirectSign } from '../lib/direct-sign.mjs';

test('only the envelope unit may call a signing primitive', () => {
  const files = [
    { path: ENVELOPE_UNIT, text: 'crypto_ed25519_sign(sig, sk, msg, n);' },
    { path: 'synergos-custody/native/receipt.cpp', text: 'int x;\n  EVP_DigestSign (ctx, sig, &len, m, n);' },
    { path: 'synergos-core/native/a.hpp', text: '// crypto_ed25519_sign is forbidden here\nauto ok = crypto_ed25519_check(s, pk, m, n);' },
  ];
  assert.deepEqual(findDirectSign(files), [
    { path: 'synergos-custody/native/receipt.cpp', line: 2, call: 'EVP_DigestSign' },
  ]);
});
