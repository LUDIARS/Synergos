import assert from 'node:assert/strict';
import test from 'node:test';
import { buildNodeToml, nodeSettings } from '../lib/node-config.mjs';

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
