/**
 * Type-safe wrappers around the Tauri commands defined in src-tauri/src/lib.rs.
 * Frontend は ここを通してのみ daemon / config にアクセスする。
 */
import { invoke } from "@tauri-apps/api/core";

export type DaemonStatus = {
  pid: number;
  project_count: number;
  active_connections: number;
  active_transfers: number;
  // synergos-ipc 側で他フィールドが増える可能性があるので余白を残す
  [key: string]: unknown;
};

export type ProjectInfo = {
  project_id: string;
  display_name: string;
  root_path: string;
  peer_count: number;
  active_transfers: number;
};

export type PeerInfo = {
  peer_id: string;
  display_name: string;
  route: string;
  rtt_ms: number;
  bandwidth_bps: number;
  state: string;
};

export type NetworkStatusInfo = {
  primary_route: string;
  active_connections: number;
  max_connections: number;
  total_bandwidth_bps: number;
  avg_latency_ms: number;
};

export type TransferInfo = {
  transfer_id: string;
  project_id: string;
  file_id: string;
  file_name: string;
  file_size: number;
  bytes_transferred: number;
  speed_bps: number;
  state: string;
  direction: string;
};

export type InviteToken = {
  token: string;
  expires_at: number | null;
};

export type ConfigFile = {
  bootstrap_urls: string[];
  force_relay_only: boolean;
  auto_promote: boolean;
  quic_listen_addr: string | null;
  peer_info_listen_addr: string | null;
};

export type ConfigPaths = {
  config_path: string;
  identity_path: string;
  projects_path: string;
};

export type BridgeError = {
  kind:
    | "daemon_not_running"
    | "daemon"
    | "io"
    | "config"
    | "unexpected";
  message?: string;
};

// ── daemon ──
export const daemonStatus = () => invoke<DaemonStatus>("daemon_status");
export const daemonPing = () => invoke<boolean>("daemon_ping");

// ── project ──
export const projectList = () => invoke<ProjectInfo[]>("project_list");
export const projectOpen = (
  projectId: string,
  rootPath: string,
  displayName?: string,
) =>
  invoke<void>("project_open", {
    projectId,
    rootPath,
    displayName: displayName ?? null,
  });
export const projectClose = (projectId: string) =>
  invoke<void>("project_close", { projectId });
export const projectPublish = (projectId: string, files: string[]) =>
  invoke<void>("project_publish", { projectId, files });
export const projectInvite = (projectId: string, expiresInSecs?: number) =>
  invoke<InviteToken>("project_invite", {
    projectId,
    expiresInSecs: expiresInSecs ?? null,
  });

// ── peer ──
export const peerList = (projectId: string) =>
  invoke<PeerInfo[]>("peer_list", { projectId });
export const peerAddUrl = (projectId: string, url: string) =>
  invoke<void>("peer_add_url", { projectId, url });
export const peerDisconnect = (peerId: string) =>
  invoke<void>("peer_disconnect", { peerId });

// ── network / transfer ──
export const networkStatus = () => invoke<NetworkStatusInfo>("network_status");
export const transferList = (projectId?: string) =>
  invoke<TransferInfo[]>("transfer_list", {
    projectId: projectId ?? null,
  });

// ── config ──
export const configGet = () => invoke<ConfigFile>("config_get");
export const configSet = (cfg: ConfigFile) =>
  invoke<void>("config_set", { cfg });
export const configPaths = () => invoke<ConfigPaths>("config_paths");

/** BridgeError を `Error` に丸めるヘルパ。React Query の error path に流す前に使う。 */
export function bridgeMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err && typeof err === "object" && "message" in err) {
    const m = (err as { message?: unknown }).message;
    if (typeof m === "string") return m;
  }
  if (err && typeof err === "object" && "kind" in err) {
    const k = (err as { kind: string }).kind;
    switch (k) {
      case "daemon_not_running":
        return "Synergos daemon is not running. Start `synergos-core` first.";
      case "daemon":
        return (err as BridgeError).message ?? "daemon error";
      case "io":
        return (err as BridgeError).message ?? "io error";
      case "config":
        return (err as BridgeError).message ?? "config error";
      case "unexpected":
        return (err as BridgeError).message ?? "unexpected response";
    }
  }
  return String(err);
}
