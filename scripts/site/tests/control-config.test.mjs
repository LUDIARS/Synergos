import assert from 'node:assert/strict';
import test from 'node:test';
import { buildControlToml } from '../lib/control-config.mjs';
import { readArgs } from '../lib/args.mjs';

const env = { CF_ACCOUNT_ID: 'a'.repeat(32), CF_API_TOKEN: 'cf-token-dummy', SYNERGOS_CONTROL_ADMIN_TOKEN: 'x'.repeat(32) };

test('writes only env var names for secrets and escapes Windows paths', () => {
  const toml = buildControlToml(env, { storePath: 'E:\\data\\site\\control-store.json' });
  assert.match(toml, /store_path = "E:\\\\data\\\\site\\\\control-store.json"/);
  assert.match(toml, /api_token_env = "CF_API_TOKEN"/);
  assert.match(toml, /bind_addr = "127.0.0.1:4250"/);
  assert.doesNotMatch(toml, /cf-token-dummy|xxxxxxxx/);
});

test('fails fast without the Cloudflare account or secrets', () => {
  assert.throws(() => buildControlToml({ ...env, CF_ACCOUNT_ID: 'nope' }, { storePath: '/s' }), /CF_ACCOUNT_ID/);
  assert.throws(() => buildControlToml({ ...env, CF_API_TOKEN: '' }, { storePath: '/s' }), /CF_API_TOKEN/);
  assert.throws(() => buildControlToml({ ...env, SYNERGOS_CONTROL_ADMIN_TOKEN: undefined }, { storePath: '/s' }), /ADMIN_TOKEN/);
});

test('parses the Excubitor argument contract strictly', () => {
  const abs = process.platform === 'win32' ? 'C:\\out.bundle' : '/out.bundle';
  assert.deepEqual(readArgs(['--output', abs], ['output']), { output: abs });
  assert.throws(() => readArgs(['--output', 'rel.bundle'], ['output']), /absolute/);
  assert.throws(() => readArgs(['--input', abs, '--sha256', 'XYZ'], ['input', 'sha256']), /64 lowercase/);
  assert.throws(() => readArgs([], ['output']), /required/);
});
