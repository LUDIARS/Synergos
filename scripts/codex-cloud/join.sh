#!/usr/bin/env bash
# Codex クラウド環境から本社の Synergos ノードへ tailnet 経由で参加する (docs/site-operations.md「Codex クラウド」)。
#   Codex の setup script: bash scripts/codex-cloud/join.sh
# 必要な環境シークレット: TS_AUTHKEY (ephemeral・tag:codex)、SYNERGOS_INVITE (本社で発行した syn1. 招待トークン)
# 任意: SYNERGOS_SHARE_DIR (同期先。既定 $HOME/synergos-share)
# 秘密の値は出力しない。再実行してよい (起動済みの tailscaled / daemon はそのまま使う)。
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
ts_dir="${CODEX_TS_DIR:-$HOME/.local/tailscale}"
state_dir="${CODEX_TS_STATE_DIR:-$HOME/.local/state/synergos-codex}"
share="${SYNERGOS_SHARE_DIR:-$HOME/synergos-share}"
config="$state_dir/synergos-net.toml"
core="$repo/target/release/synergos-core"

for name in TS_AUTHKEY SYNERGOS_INVITE; do
  if [ -z "${!name:-}" ]; then echo "join: $name is not set" >&2; exit 1; fi
done
# Synergos は QUIC (UDP) で直結する。tailscaled の userspace モードは UDP を中継しないため、
# カーネルの TUN が使えない環境では参加できない。
if [ ! -c /dev/net/tun ]; then
  echo "join: /dev/net/tun がありません。この環境では tailnet 上の QUIC 直結ができません" >&2
  exit 2
fi
mkdir -p "$state_dir" "$share"

if [ ! -x "$ts_dir/tailscaled" ]; then
  mkdir -p "$ts_dir"
  case "$(uname -m)" in
    x86_64) ts_arch=amd64 ;;
    aarch64|arm64) ts_arch=arm64 ;;
    *) echo "join: unsupported architecture $(uname -m)" >&2; exit 1 ;;
  esac
  tarball="$(curl -fsSL 'https://pkgs.tailscale.com/stable/?mode=json' | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{const t=JSON.parse(s).Tarballs[process.argv[1]];if(!t)process.exit(2);process.stdout.write(t)})' "$ts_arch")"
  curl -fsSL "https://pkgs.tailscale.com/stable/$tarball" | tar -xz -C "$ts_dir" --strip-components=1
fi
ts=("$ts_dir/tailscale" --socket="$state_dir/tailscaled.sock")
if ! "${ts[@]}" status >/dev/null 2>&1; then
  nohup "$ts_dir/tailscaled" --state=mem: --socket="$state_dir/tailscaled.sock" >"$state_dir/tailscaled.log" 2>&1 &
  for _ in $(seq 1 30); do "${ts[@]}" version >/dev/null 2>&1 && break; sleep 1; done
  "${ts[@]}" up --authkey="$TS_AUTHKEY" --hostname="codex-$(hostname | tr -cd 'a-z0-9-' | cut -c1-20)" --accept-dns=false >/dev/null
fi

[ -x "$core" ] || (cd "$repo" && cargo build --release --locked -p synergos-core)

cat >"$config" <<'TOML'
auto_promote = false

[quic]
listen_addr = "[::]:0"
max_concurrent_streams = 100
idle_timeout_ms = 30000
max_udp_payload_size = 1200
enable_0rtt = false

[tunnel]
api_token_ref = ""
hostname = ""
allow_simulation = false
auto_restart = false
restart_base_ms = 1000
restart_max_ms = 60000
TOML

if ! "$core" status >/dev/null 2>&1; then
  nohup "$core" start --config "$config" >"$state_dir/synergos-core.log" 2>&1 &
  for _ in $(seq 1 30); do "$core" status >/dev/null 2>&1 && break; sleep 1; done
fi
"$core" project join "$SYNERGOS_INVITE" "$share"
echo "join: joined the HQ Synergos project; files sync into $share"
