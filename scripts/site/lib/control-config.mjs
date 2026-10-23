// synergos-control の control.toml を環境変数から組み立てる純関数。
// 秘密値はファイルに書かず、読み出す環境変数名だけを書く (synergos-control/src/config.rs)。

const ACCOUNT_ID = /^[0-9a-f]{32}$/i;

const tomlString = value => JSON.stringify(value);

/**
 * @param {Record<string, string | undefined>} env
 * @param {{ storePath: string }} paths
 */
export function buildControlToml(env, { storePath }) {
  const accountId = env.CF_ACCOUNT_ID?.trim();
  if (!accountId || !ACCOUNT_ID.test(accountId)) throw new Error('CF_ACCOUNT_ID must be a 32-character Cloudflare account ID');
  for (const name of ['CF_API_TOKEN', 'SYNERGOS_CONTROL_ADMIN_TOKEN']) {
    if (!env[name]) throw new Error(`${name} is required (inject it from the Excubitor Vault)`);
  }
  const bind = env.SYNERGOS_CONTROL_BIND || '127.0.0.1:4250';
  return [
    `bind_addr = ${tomlString(bind)}`,
    `store_path = ${tomlString(storePath)}`,
    'admin_token_env = "SYNERGOS_CONTROL_ADMIN_TOKEN"',
    '',
    '[cloudflare]',
    `account_id = ${tomlString(accountId)}`,
    'api_token_env = "CF_API_TOKEN"',
    '',
  ].join('\n');
}
