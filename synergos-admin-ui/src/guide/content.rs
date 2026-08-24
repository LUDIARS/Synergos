//! ガイド本文 (日本語)。
//!
//! 正本は docs/ 配下。ここでは「管理コンソールを触りながら順に進む」順序で並べる。
//! id は画面の「?」ヘルプが指すアンカーなので、変更するときは呼び出し側も直す。

use super::{Command, Os, Section, Step};

/// 「?」ヘルプが指すアンカー。文字列直書きの散在を避ける。
pub mod anchors {
    pub const OVERVIEW: &str = "overview";
    pub const CONTROL_SETUP: &str = "control-setup";
    pub const ORG_MEMBERS: &str = "org-members";
    pub const NODE_REGISTER: &str = "node-register";
    pub const NODE_ENROLL: &str = "node-enroll";
    pub const FIREWALL: &str = "firewall";
    pub const SYNERGOS_CONFIG: &str = "synergos-config";
    pub const HEARTBEAT: &str = "heartbeat";
    pub const RECONCILE: &str = "reconcile";
}

pub fn sections() -> &'static [Section] {
    SECTIONS
}

static SECTIONS: &[Section] = &[
    Section {
        id: anchors::OVERVIEW,
        title: "1. 全体像を掴む",
        summary: "Synergos は Cloudflare Mesh の中だけで完結する P2P ネットワークとして動く。\
                  管制サーバー (synergos-control) が「誰のノードが居るか」を握り、\
                  Cloudflare 側の設定を自動化する。",
        steps: &[
            Step {
                title: "登場人物",
                body: "Mesh node = Linux の常駐サーバ (warp-cli connector で参加)。\
                       Client device = 人の PC/スマホ (Cloudflare One Client でエンロール)。\
                       どちらにも 100.96.0.0/12 の Mesh IP が割り当てられ、双方向に通信できる。",
                commands: &[],
            },
            Step {
                title: "進め方",
                body: "(1) 管制サーバーを起動 → (2) 組織とメンバーを作る → \
                       (3) ノードを登録してトークンを受け取る → (4) ノード側でエンロール → \
                       (5) ファイアウォールを開ける → (6) synergos-net.toml を書く → \
                       (7) heartbeat と dark node 点検を回す。この画面の各節がその順番。",
                commands: &[],
            },
        ],
    },
    Section {
        id: anchors::CONTROL_SETUP,
        title: "2. 管制サーバーを起動する",
        summary: "秘密情報は環境変数からのみ渡す。設定ファイルにトークンを書かない。",
        steps: &[
            Step {
                title: "設定ファイルを用意する",
                body: "control.example.toml を複製し、Cloudflare の Account ID を記入する。\
                       管理 UI を配信する場合は [ui] dist_path にビルド成果物を指定する。",
                commands: &[
                    Command {
                        os: Some(Os::Linux),
                        caption: "設定ファイルを複製",
                        body: "cp synergos-control/control.example.toml control.toml",
                    },
                    Command {
                        os: Some(Os::Mac),
                        caption: "設定ファイルを複製",
                        body: "cp synergos-control/control.example.toml control.toml",
                    },
                    Command {
                        os: Some(Os::Windows),
                        caption: "設定ファイルを複製 (PowerShell)",
                        body: "Copy-Item synergos-control\\control.example.toml control.toml",
                    },
                ],
            },
            Step {
                title: "秘密情報を環境変数で渡して起動する",
                body: "SYNERGOS_CONTROL_ADMIN_TOKEN は 32 バイト以上。\
                       ここで作った値が、この管理コンソールのログインに使う管理トークンになる。\
                       必須の環境変数が無いと起動を拒否する (フォールバックしない)。",
                commands: &[
                    Command {
                        os: Some(Os::Linux),
                        caption: "起動",
                        body: "export SYNERGOS_CONTROL_ADMIN_TOKEN=$(openssl rand -hex 32)\n\
                               export CLOUDFLARE_API_TOKEN=<Cloudflare API token>\n\
                               synergos-control serve --config control.toml",
                    },
                    Command {
                        os: Some(Os::Mac),
                        caption: "起動",
                        body: "export SYNERGOS_CONTROL_ADMIN_TOKEN=$(openssl rand -hex 32)\n\
                               export CLOUDFLARE_API_TOKEN=<Cloudflare API token>\n\
                               synergos-control serve --config control.toml",
                    },
                    Command {
                        os: Some(Os::Windows),
                        caption: "起動 (PowerShell)",
                        body: "$env:SYNERGOS_CONTROL_ADMIN_TOKEN = -join ((1..32) | ForEach-Object { '{0:x2}' -f (Get-Random -Max 256) })\n\
                               $env:CLOUDFLARE_API_TOKEN = '<Cloudflare API token>'\n\
                               synergos-control serve --config control.toml",
                    },
                ],
            },
            Step {
                title: "管理コンソールを開く",
                body: "ブラウザで http://127.0.0.1:4250/ui/ を開き、管理トークンを入力する。\
                       トークンは sessionStorage にだけ保持され、タブを閉じると消える。",
                commands: &[Command {
                    os: None,
                    caption: "URL",
                    body: "http://127.0.0.1:4250/ui/",
                }],
            },
        ],
    },
    Section {
        id: anchors::ORG_MEMBERS,
        title: "3. 組織とメンバーを作る",
        summary: "組織 (org) はノードと「許可された人のメール」を束ねる単位。\
                  メンバーに居ない人のノードは登録できず、突合では dark 扱いになる。",
        steps: &[Step {
            title: "組織を作る",
            body: "org の id は英小文字・数字・ハイフンの slug。members には\
                   Cloudflare のデバイスエンロールで使うメールアドレスを、そのまま入れる。\
                   ここが一致していないと reconcile が本人の端末を dark と判定する。",
            commands: &[Command {
                os: None,
                caption: "API で作る場合",
                body: "curl -s -X POST http://127.0.0.1:4250/v1/orgs \\\n  \
                       -H \"Authorization: Bearer $SYNERGOS_CONTROL_ADMIN_TOKEN\" \\\n  \
                       -H \"Content-Type: application/json\" \\\n  \
                       -d '{\"id\":\"acme\",\"name\":\"Acme\",\"members\":[\"alice@acme.test\"]}'",
            }],
        }],
    },
    Section {
        id: anchors::NODE_REGISTER,
        title: "4. ノードを登録する",
        summary: "「組織 / ノード管理」画面の登録フォームから行う。\
                  Mesh node を登録すると Cloudflare 側の connector が自動作成され、\
                  登録トークン (connector_token) が一度だけ返る。",
        steps: &[
            Step {
                title: "登録フォームを埋める",
                body: "表示名・所有者メール (組織メンバーであること)・種別を選ぶ。\
                       常駐 Linux サーバなら Mesh node、人の端末なら Client device。",
                commands: &[],
            },
            Step {
                title: "返ってきた値を控える",
                body: "connector_token はノードのエンロールに使う。node_key は daemon の \
                       heartbeat 認証に使う。どちらも control には保存されないため、\
                       この画面を閉じる前にコピーする。無くしたら再発行できる。",
                commands: &[],
            },
            Step {
                title: "次に何をするか",
                body: "connector_token を持って「5. ノードをエンロールする」へ。\
                       node_key は「8. heartbeat を設定する」で使う。",
                commands: &[],
            },
        ],
    },
    Section {
        id: anchors::NODE_ENROLL,
        title: "5. ノードをエンロールする",
        summary: "Mesh node (Linux) は connector_token で参加する。\
                  人の端末は Cloudflare One Client に team name を入れて参加する。",
        steps: &[
            Step {
                title: "WARP クライアントを入れる",
                body: "Mesh node にする Linux では cloudflare-warp パッケージを入れ、\
                       IP forwarding を有効にする。Windows / macOS は Cloudflare One Client\
                       (GUI) をインストールする。",
                commands: &[
                    Command {
                        os: Some(Os::Linux),
                        caption: "WARP 導入 (Debian / Ubuntu)",
                        body: "curl -fsSL https://pkg.cloudflareclient.com/pubkey.gpg | sudo gpg --yes --dearmor -o /usr/share/keyrings/cloudflare-warp-archive-keyring.gpg\n\
                               echo \"deb [signed-by=/usr/share/keyrings/cloudflare-warp-archive-keyring.gpg] https://pkg.cloudflareclient.com/ $(. /etc/os-release && echo $VERSION_CODENAME) main\" | sudo tee /etc/apt/sources.list.d/cloudflare-client.list\n\
                               sudo apt-get update && sudo apt-get install -y cloudflare-warp",
                    },
                    Command {
                        os: Some(Os::Linux),
                        caption: "IP forwarding",
                        body: "printf 'net.ipv4.ip_forward = 1\\nnet.ipv6.conf.all.forwarding = 1\\n' | sudo tee /etc/sysctl.d/99-zzz-cloudflare-warp-connector.conf\n\
                               sudo sysctl --system",
                    },
                    Command {
                        os: Some(Os::Windows),
                        caption: "参加方法",
                        body: "Cloudflare One Client をインストール → 設定 → Zero Trust security →\n\
                               team name を入力してログイン (許可されたメールのみ通る)",
                    },
                    Command {
                        os: Some(Os::Mac),
                        caption: "参加方法",
                        body: "Cloudflare One Client をインストール → 設定 → Zero Trust security →\n\
                               team name を入力してログイン (許可されたメールのみ通る)",
                    },
                ],
            },
            Step {
                title: "登録トークンで参加する (Mesh node のみ)",
                body: "ノード登録画面で受け取った connector_token を、そのノードの上で使う。\
                       headless の Mesh node として参加できるのは Linux だけ。\
                       Windows / macOS は前ステップの Client device として参加する。",
                commands: &[Command {
                    os: Some(Os::Linux),
                    caption: "connector 登録",
                    body: "sudo warp-cli connector new <connector_token> && sudo warp-cli connect",
                }],
            },
            Step {
                title: "Mesh IP を確認する",
                body: "参加すると 100.96.0.0/12 のアドレスが割り当てられる。\
                       この IP を synergos-net.toml と、必要ならノードの mesh_ip に記録する。",
                commands: &[
                    Command {
                        os: Some(Os::Linux),
                        caption: "確認",
                        body: "warp-cli status && ip -4 addr show | grep 100\\.",
                    },
                    Command {
                        os: Some(Os::Mac),
                        caption: "確認",
                        body: "ifconfig | grep 'inet 100\\.'",
                    },
                    Command {
                        os: Some(Os::Windows),
                        caption: "確認 (PowerShell)",
                        body: "Get-NetIPAddress -AddressFamily IPv4 | Where-Object { $_.IPAddress -like '100.*' }",
                    },
                ],
            },
        ],
    },
    Section {
        id: anchors::FIREWALL,
        title: "6. ファイアウォールを開ける",
        summary: "Synergos の QUIC listen ポート (既定 4433/UDP) を Mesh レンジに開ける。\
                  Windows は既定で 100.96.0.0/12 からの inbound をブロックする。",
        steps: &[Step {
            title: "Mesh レンジからの UDP を許可する",
            body: "許可元は 100.96.0.0/12 に限定する (全開放しない)。",
            commands: &[
                Command {
                    os: Some(Os::Windows),
                    caption: "PowerShell (管理者)",
                    body: "New-NetFirewallRule -DisplayName \"Synergos QUIC (Cloudflare Mesh)\" `\n  \
                           -Direction Inbound -Protocol UDP -LocalPort 4433 `\n  \
                           -RemoteAddress 100.96.0.0/12 -Action Allow",
                },
                Command {
                    os: Some(Os::Linux),
                    caption: "ufw の例",
                    body: "sudo ufw allow from 100.96.0.0/12 to any port 4433 proto udp",
                },
                Command {
                    os: Some(Os::Mac),
                    caption: "確認",
                    body: "# 既定のパケットフィルタは inbound を塞がない。\n\
                           # 追加でファイアウォールを入れている場合のみ 4433/UDP を許可する。",
                },
            ],
        }],
    },
    Section {
        id: anchors::SYNERGOS_CONFIG,
        title: "7. synergos-net.toml を書く",
        summary: "Mesh 上では外部 IP の自己検出をせず、自分の Mesh IP を明示 advertise する。",
        steps: &[Step {
            title: "各ノードの設定",
            body: "quic_advertised_addr に自分の Mesh IP を書き、bootstrap_urls に\
                   相手ノードの Mesh IP を書く。cloudflared spawn (tunnel) は使わない。",
            commands: &[Command {
                os: None,
                caption: "synergos-net.toml",
                body: "quic_advertised_addr = \"<自分の Mesh IP>:4433\"\n\
                       peer_info_listen_addr = \"0.0.0.0:7777\"\n\
                       bootstrap_urls = [\"http://<相手の Mesh IP>:7777/peer-info\"]\n\
                       \n\
                       [tunnel]\n\
                       hostname = \"\"",
            }],
        }],
    },
    Section {
        id: anchors::HEARTBEAT,
        title: "8. heartbeat を設定する",
        summary: "daemon が peer_id と Mesh IP を管制サーバーへ定期報告する。\
                  これでレジストリ上に「どのノードがどの peer_id か」が揃う。",
        steps: &[
            Step {
                title: "node_key を環境変数で渡す",
                body: "ノード登録時に返った node_key をノード側の環境変数に置く。\
                       設定ファイルにはキー本体を書かず、環境変数名だけを書く。",
                commands: &[
                    Command {
                        os: Some(Os::Linux),
                        caption: "環境変数",
                        body: "export SYNERGOS_NODE_KEY=<node_key>",
                    },
                    Command {
                        os: Some(Os::Mac),
                        caption: "環境変数",
                        body: "export SYNERGOS_NODE_KEY=<node_key>",
                    },
                    Command {
                        os: Some(Os::Windows),
                        caption: "環境変数 (PowerShell)",
                        body: "$env:SYNERGOS_NODE_KEY = '<node_key>'",
                    },
                ],
            },
            Step {
                title: "synergos-net.toml に [control] を足す",
                body: "heartbeat_url があるのに node_id / キーが無いと daemon は起動時に落ちる\
                       (設定ミスを黙って無視しない)。",
                commands: &[Command {
                    os: None,
                    caption: "synergos-net.toml",
                    body: "[control]\n\
                           heartbeat_url = \"http://<管制サーバーの Mesh IP>:4250/v1/heartbeat\"\n\
                           node_id = \"<登録時に返った node.id>\"\n\
                           node_key_env = \"SYNERGOS_NODE_KEY\"\n\
                           interval_secs = 60",
                }],
            },
            Step {
                title: "bind_addr の注意",
                body: "heartbeat を受けるには管制サーバーの bind を Mesh から届く範囲に広げる\
                       必要がある。その場合も 0.0.0.0 ではなく管制サーバー自身の Mesh IP に\
                       限定する — 管理 API も同じリスナー上にあり、管理トークンが唯一の防壁になる。",
                commands: &[],
            },
        ],
    },
    Section {
        id: anchors::RECONCILE,
        title: "9. dark node を点検する",
        summary: "Cloudflare 側の実態とレジストリを突合し、未登録の参加者 (dark node) を炙り出す。\
                  定期実行を推奨。",
        steps: &[
            Step {
                title: "レポートを取る",
                body: "ダッシュボードの「dark node を点検」か、Mesh 自動設定の突合ステップで実行できる。\
                       dark_connectors = CF に居るが未登録の Mesh node、\
                       dark_devices = どの組織メンバーでもない端末、\
                       missing_connectors = 登録済みだが CF に実体が無いノード。",
                commands: &[Command {
                    os: None,
                    caption: "API で取る場合 (レポートのみ)",
                    body: "curl -s -X POST http://127.0.0.1:4250/v1/reconcile \\\n  \
                           -H \"Authorization: Bearer $SYNERGOS_CONTROL_ADMIN_TOKEN\"",
                }],
            },
            Step {
                title: "失効まで行う場合",
                body: "破壊的操作なので、この管理コンソールからは実行しない。\
                       レポートを確認したうえで CLI から明示的に行う。\
                       対象 Cloudflare account を Synergos 専用の管理境界とみなす点に注意 — \
                       他用途と同居する account では実行しないこと。",
                commands: &[Command {
                    os: None,
                    caption: "API (破壊的)",
                    body: "curl -s -X POST http://127.0.0.1:4250/v1/reconcile \\\n  \
                           -H \"Authorization: Bearer $SYNERGOS_CONTROL_ADMIN_TOKEN\" \\\n  \
                           -H \"Content-Type: application/json\" -d '{\"revoke_dark\":true}'",
                }],
            },
        ],
    },
];
