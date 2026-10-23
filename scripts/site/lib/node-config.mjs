// synergos-core (ノード本体) の synergos-net.toml を環境変数から組み立てる純関数。
// 待ち受けは tailnet 等の具体アドレスに限り、0.0.0.0 / :: のような全面待ち受けは拒否する
// (/peer-info は無認証なので、届く範囲をネットワーク層で絞る前提: docs/two-node-operations.md)。
import net from 'node:net';

const WILDCARDS = new Set(['0.0.0.0', '::']);
// Tailscale の MTU は 1280。QUIC の UDP ペイロードがそれを超えないよう抑える。
const TAILNET_UDP_PAYLOAD = 1200;

function port(env, name, fallback) {
  const raw = env[name] ?? String(fallback);
  const value = Number(raw);
  if (!Number.isInteger(value) || value < 1 || value > 65535) throw new Error(`${name} must be a TCP/UDP port (1-65535)`);
  return value;
}

export function nodeSettings(env = process.env) {
  const ip = env.SYNERGOS_NODE_BIND_IP;
  if (!ip) throw new Error('SYNERGOS_NODE_BIND_IP is not set (the tailnet address this node listens on)');
  if (!net.isIP(ip)) throw new Error('SYNERGOS_NODE_BIND_IP must be a literal IP address');
  if (WILDCARDS.has(ip)) throw new Error('SYNERGOS_NODE_BIND_IP must not be a wildcard address');
  return {
    ip,
    peerInfoPort: port(env, 'SYNERGOS_NODE_PEER_INFO_PORT', 7780),
    quicPort: port(env, 'SYNERGOS_NODE_QUIC_PORT', 4433),
  };
}

const hostPort = (ip, p) => (net.isIPv6(ip) ? `[${ip}]:${p}` : `${ip}:${p}`);

export function buildNodeToml(env = process.env) {
  const { ip, peerInfoPort, quicPort } = nodeSettings(env);
  const peerInfo = hostPort(ip, peerInfoPort);
  const quic = hostPort(ip, quicPort);
  return [
    '# Excubitor の run-node.mjs が生成する。手で編集しない (起動のたびに上書きされる)。',
    `peer_info_listen_addr = "${peerInfo}"`,
    `peer_info_advertised_url = "http://${peerInfo}"`,
    `quic_advertised_addr = "${quic}"`,
    '# tailnet / Mesh 上では NAT 越え probe を行わない。',
    'auto_promote = false',
    '',
    '[quic]',
    `listen_addr = "${quic}"`,
    'max_concurrent_streams = 100',
    'idle_timeout_ms = 30000',
    `max_udp_payload_size = ${TAILNET_UDP_PAYLOAD}`,
    'enable_0rtt = false',
    '',
    '[tunnel]',
    'api_token_ref = ""',
    'hostname = ""',
    'allow_simulation = false',
    'auto_restart = false',
    'restart_base_ms = 1000',
    'restart_max_ms = 60000',
    '',
  ].join('\n');
}
