//! IpcEvent → Ars EventBus ブリッジ
//!
//! synergos-core デーモンからの IPC イベントを受信し、
//! 対応する Ars EventBus イベントに変換して配信する。

use synergos_ipc::event::IpcEvent;

use crate::events::*;

/// IPC イベントを Ars EventBus イベントに変換する
///
/// Ars EventBus への emit は呼び出し元が行う。
/// この関数は変換ロジックのみを担当する。
pub enum BridgedEvent {
    PeerConnected(PeerConnected),
    PeerDisconnected(PeerDisconnected),
    TransferProgress(FileTransferProgress),
    TransferCompleted(FileTransferCompleted),
    NetworkStatus(NetworkStatusUpdated),
    ConflictDetected(ConflictDetected),
}

/// IPC イベントを Ars 向けイベントに変換
pub fn bridge_event(ipc_event: IpcEvent) -> Option<BridgedEvent> {
    match ipc_event {
        IpcEvent::PeerConnected {
            peer_id,
            display_name,
            route,
            rtt_ms,
            ..
        } => Some(BridgedEvent::PeerConnected(PeerConnected {
            peer_id,
            display_name,
            route,
            rtt_ms,
        })),

        IpcEvent::PeerDisconnected {
            peer_id, reason, ..
        } => Some(BridgedEvent::PeerDisconnected(PeerDisconnected {
            peer_id,
            reason: DisconnectReason::Remote(reason),
        })),

        IpcEvent::TransferProgress {
            transfer_id,
            file_name,
            bytes_transferred,
            total_bytes,
            speed_bps,
        } => Some(BridgedEvent::TransferProgress(FileTransferProgress {
            transfer_id,
            peer_id: String::new(), // TODO: IPC イベントにピアIDを含める
            resource_id: file_name,
            bytes_transferred,
            total_bytes,
            speed_bps,
        })),

        IpcEvent::TransferCompleted {
            transfer_id,
            file_name,
            file_path,
        } => Some(BridgedEvent::TransferCompleted(FileTransferCompleted {
            transfer_id,
            peer_id: String::new(),
            resource_id: file_name,
            file_path,
        })),

        IpcEvent::NetworkStatusUpdated {
            active_connections,
            total_bandwidth_bps,
            used_bandwidth_bps: _,
            avg_latency_ms,
        } => Some(BridgedEvent::NetworkStatus(NetworkStatusUpdated {
            active_connections,
            max_connections: 0,
            total_bandwidth_bps,
            used_bandwidth_bps: 0,
            avg_latency_ms,
        })),

        IpcEvent::ConflictDetected {
            file_id,
            file_path,
            involved_peers,
            ..
        } => Some(BridgedEvent::ConflictDetected(ConflictDetected {
            file_id,
            file_path,
            involved_peers,
        })),

        _ => None,
    }
}
