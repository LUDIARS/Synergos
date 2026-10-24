import assert from 'node:assert/strict';
import test from 'node:test';
import { buildNodeToml, nodeSettings } from '../lib/node-config.mjs';
import { contract } from './contracts/contract-runtime.mjs'; /* augur-inject:import:e392d8f1 */
import augurContract_0e3ce785 from './contracts/wildcard-rejection.contract.mjs'; /* augur-inject:contract-predicate:5cea5ae0 */
import augurContract_9d1317c0 from './contracts/valid-addresses.contract.mjs'; /* augur-inject:contract-predicate:d469a1ae */

export function verifyWildcardRejection(addresses) {
  for (const ip of addresses) {
    assert.throws(() => nodeSettings({ SYNERGOS_NODE_BIND_IP: ip }), {
      message: 'SYNERGOS_NODE_BIND_IP must not be a wildcard address',
    }, ip);
  }
  return addresses.length;
}
// @ts-expect-error augur-inject
verifyWildcardRejection = contract(verifyWildcardRejection, { ...augurContract_0e3ce785, contractId: 'C-1', mode: 'observe', sample: 1, where: 'scripts/site/tests/node-config.test.mjs:5', rule: 'contract-wrap', id: '0e3ce785' }); /* augur-inject:contract-wrap:0e3ce785 */

export function verifyValidAddresses(addresses) {
  for (const ip of addresses) {
    assert.deepEqual(nodeSettings({ SYNERGOS_NODE_BIND_IP: ip }), {
      ip, peerInfoPort: 7780, quicPort: 4433,
    }, ip);
  }
  return addresses.length;
}
// @ts-expect-error augur-inject
verifyValidAddresses = contract(verifyValidAddresses, { ...augurContract_9d1317c0, contractId: 'C-2', mode: 'observe', sample: 1, where: 'scripts/site/tests/node-config.test.mjs:14', rule: 'contract-wrap', id: '9d1317c0' }); /* augur-inject:contract-wrap:9d1317c0 */

test('refuses equivalent unspecified IPv4, IPv6 and mapped addresses', () => {
  verifyWildcardRejection([
    '0.0.0.0', '::', '::0', '0:0:0:0:0:0:0:0',
    '0000:0000:0000:0000:0000:0000:0000:0000',
    '::ffff:0.0.0.0', '::ffff:0:0', '::FFFF:0000:0000',
    '0:0:0:0:0:ffff:0:0', '0:0:0:0:0:ffff:0.0.0.0',
    '::%eth0', '::0%eth0', '::ffff:0.0.0.0%eth0',
  ]);
});

test('preserves valid IPv4, IPv6 and mapped addresses', () => {
  verifyValidAddresses([
    '100.64.0.1', '100.127.255.254', 'fd7a:115c:a1e0::1',
    'FD7A:115C:A1E0:0:0:0:0:1', '::ffff:100.64.0.1',
    '::FFFF:6440:1', '::1', '::2', 'fe80::1%eth0',
  ]);
});

test('binds peer-info and QUIC to the given tailnet address only', () => {
  const toml = buildNodeToml({ SYNERGOS_NODE_BIND_IP: '100.122.174.105' });
  assert.match(toml, /^peer_info_listen_addr = "100\.122\.174\.105:7780"$/m);
  assert.match(toml, /^peer_info_advertised_url = "http:\/\/100\.122\.174\.105:7780"$/m);
  assert.match(toml, /^quic_advertised_addr = "100\.122\.174\.105:4433"$/m);
  assert.match(toml, /^listen_addr = "100\.122\.174\.105:4433"$/m);
  assert.match(toml, /^auto_promote = false$/m);
  assert.match(toml, /^max_udp_payload_size = 1200$/m);
});

test('brackets IPv6 and honours port overrides', () => {
  const toml = buildNodeToml({ SYNERGOS_NODE_BIND_IP: 'fd7a:115c:a1e0::732:ae69', SYNERGOS_NODE_PEER_INFO_PORT: '17780', SYNERGOS_NODE_QUIC_PORT: '14433' });
  assert.match(toml, /^peer_info_listen_addr = "\[fd7a:115c:a1e0::732:ae69\]:17780"$/m);
  assert.match(toml, /^listen_addr = "\[fd7a:115c:a1e0::732:ae69\]:14433"$/m);
});

test('refuses a missing, non-literal or wildcard address and bad ports', () => {
  assert.throws(() => nodeSettings({}), /SYNERGOS_NODE_BIND_IP is not set/);
  assert.throws(() => nodeSettings({ SYNERGOS_NODE_BIND_IP: 'desktop.ts.net' }), /literal IP/);
  assert.throws(() => nodeSettings({ SYNERGOS_NODE_BIND_IP: '0.0.0.0' }), /wildcard/);
  assert.throws(() => nodeSettings({ SYNERGOS_NODE_BIND_IP: '::' }), /wildcard/);
  assert.throws(() => nodeSettings({ SYNERGOS_NODE_BIND_IP: '100.64.0.1', SYNERGOS_NODE_QUIC_PORT: '70000' }), /port/);
});
