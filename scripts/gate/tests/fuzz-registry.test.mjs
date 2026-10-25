import assert from 'node:assert/strict';
import test from 'node:test';
import { checkFuzzRegistry, taggedDecoders } from '../lib/fuzz-registry.mjs';

const entry = (id, extra = {}) => ({
  decoder_id: id, protocol: 'p', version: 1, kind: 'k', max_len: 10, max_items: null,
  target: `fuzz/targets/${id}.cpp`, corpus_dir: `fuzz/${id}`, domain: 'mesh-core', ...extra,
});
const repo = (over = {}) => ({
  exists: () => true,
  targetText: () => 'extern "C" int LLVMFuzzerTestOneInput(const uint8_t*, size_t) { return 0; }',
  corpusCount: () => 1,
  cmakeText: 'synergos_fuzz_target(a.b lib)\n',
  domains: ['mesh-core'],
  ...over,
});

test('reads // @decoder tags', () => {
  assert.deepEqual(taggedDecoders([{ path: 'x.cpp', text: '// @implements S\n// @decoder a.b\nint x;\n  // @decoder c_d\n' }]), [
    { id: 'a.b', path: 'x.cpp' }, { id: 'c_d', path: 'x.cpp' },
  ]);
});

test('a complete registration passes', () => {
  assert.deepEqual(checkFuzzRegistry({ decoders: [entry('a.b')] }, repo(), [{ id: 'a.b', path: 'x.cpp' }]), []);
});

test('an unregistered decoder and a registration without a tagged source fail', () => {
  const errors = checkFuzzRegistry({ decoders: [entry('a.b')] }, repo(), [{ id: 'z.z', path: 'y.cpp' }]);
  assert.deepEqual(errors, ['z.z: decoder in y.cpp is not registered in fuzz/registry.json', 'a.b: no source carries // @decoder a.b']);
});

test('empty corpus, missing build, bad bounds and unknown domain fail', () => {
  const errors = checkFuzzRegistry(
    { decoders: [entry('a.b', { max_len: 0, max_items: -1, domain: 'nowhere' })] },
    repo({ corpusCount: () => 0, cmakeText: '' }),
    [{ id: 'a.b', path: 'x.cpp' }],
  );
  assert.deepEqual(errors, [
    'a.b: max_len must be a positive integer',
    'a.b: max_items must be a positive integer or null',
    'a.b: corpus is empty',
    'a.b: unknown domain nowhere',
    'a.b: not built by fuzz/CMakeLists.txt',
  ]);
});

test('a target without the libFuzzer entry point fails', () => {
  const errors = checkFuzzRegistry({ decoders: [entry('a.b')] }, repo({ targetText: () => 'int main() {}' }), [{ id: 'a.b', path: 'x.cpp' }]);
  assert.deepEqual(errors, ['a.b: target does not define LLVMFuzzerTestOneInput']);
});
