//! GUI スナップショット / レンダリングテスト (Issue #9 PR-13)。
//!
//! `egui_kittest::Harness` を使って各タブの UI を headless で描画し、
//! ラベルや値が期待通り出ているかを確認する。実デーモンには繋がないので
//! `CoreConnection::new()` は "未接続" 状態で構築される。`cache` を
//! テストから直接書き換えて描画内容を検証する。

use egui_kittest::kittest::Queryable;
use synergos_gui::app::UiInputs;
use synergos_gui::connection::{ConnectionCache, CoreConnection};
use synergos_gui::ui;
use synergos_ipc::response::{
    ConflictInfoDto, DaemonStatus, NetworkStatusInfo, PeerInfo, TransferInfo,
};

fn seeded_connection(populate: impl FnOnce(&mut ConnectionCache)) -> CoreConnection {
    let conn = CoreConnection::new();
    {
        let mut cache = conn.cache.lock().unwrap();
        populate(&mut cache);
    }
    conn
}

#[test]
fn overview_renders_without_data() {
    let conn = seeded_connection(|_| {});
    let mut harness = egui_kittest::Harness::new_ui(|ui| {
        let mut inputs = UiInputs::default();
        ui::overview::show(ui, &conn, &mut inputs);
    });
    harness.run();
    // "Overview" ヘディングが描画されることを確認
    let _ = harness.get_by_label("Network Overview");
}

#[test]
fn overview_shows_network_info_when_populated() {
    let conn = seeded_connection(|cache| {
        cache.status = Some(DaemonStatus {
            pid: 4242,
            started_at: 0,
            project_count: 3,
            active_connections: 2,
            active_transfers: 1,
        });
        cache.network = Some(NetworkStatusInfo {
            primary_route: "DirectIPv6".into(),
            total_bandwidth_bps: 1_000_000,
            used_bandwidth_bps: 0,
            active_connections: 2,
            max_connections: 8,
            avg_latency_ms: 42,
        });
    });
    let mut harness = egui_kittest::Harness::new_ui(|ui| {
        let mut inputs = UiInputs::default();
        ui::overview::show(ui, &conn, &mut inputs);
    });
    harness.run();
    // network.primary_route が overview で表示されるはず
    let _ = harness.get_by_label("DirectIPv6");
}

#[test]
fn transfers_empty_state() {
    let conn = seeded_connection(|_| {});
    let mut harness = egui_kittest::Harness::new_ui(|ui| {
        let mut inputs = UiInputs::default();
        ui::transfers::show(ui, &conn, &mut inputs);
    });
    harness.run();
    let _ = harness.get_by_label("Transfers");
}

#[test]
fn transfers_lists_rows() {
    let conn = seeded_connection(|cache| {
        cache.transfers = vec![TransferInfo {
            transfer_id: "t-1".into(),
            file_name: "report.pdf".into(),
            file_size: 2048,
            bytes_transferred: 1024,
            speed_bps: 512,
            direction: "Receive".into(),
            peer_id: "peer-xyz".into(),
            state: "Running".into(),
        }];
    });
    let mut harness = egui_kittest::Harness::new_ui(|ui| {
        let mut inputs = UiInputs::default();
        ui::transfers::show(ui, &conn, &mut inputs);
    });
    harness.run();
    let _ = harness.get_by_label_contains("report.pdf");
}

#[test]
fn peers_empty_state() {
    let conn = seeded_connection(|_| {});
    let mut harness = egui_kittest::Harness::new_ui(|ui| {
        let mut inputs = UiInputs::default();
        ui::peers::show(ui, &conn, &mut inputs);
    });
    harness.run();
    let _ = harness.get_by_label("Peers");
}

#[test]
fn peers_lists_rows() {
    let conn = seeded_connection(|cache| {
        cache.peers = vec![PeerInfo {
            peer_id: "peer-abc".into(),
            display_name: "Alice".into(),
            route: "Direct".into(),
            rtt_ms: 42,
            bandwidth_bps: 1_000_000,
            state: "Connected".into(),
            synergos_version: "0.1.0".into(),
        }];
    });
    let mut harness = egui_kittest::Harness::new_ui(|ui| {
        let mut inputs = UiInputs::default();
        ui::peers::show(ui, &conn, &mut inputs);
    });
    harness.run();
    let _ = harness.get_by_label_contains("Alice");
}

#[test]
fn conflicts_empty_state_shows_help() {
    let conn = seeded_connection(|_| {});
    let mut harness = egui_kittest::Harness::new_ui(|ui| {
        ui::conflicts::show(ui, &conn);
    });
    harness.run();
    let _ = harness.get_by_label_contains("No active conflicts");
}

#[test]
fn conflicts_lists_rows_with_resolve_buttons() {
    let conn = seeded_connection(|cache| {
        cache.conflicts = vec![ConflictInfoDto {
            file_id: "f-1".into(),
            file_path: "src/main.rs".into(),
            project_id: "proj".into(),
            local_version: 3,
            local_author: "aaaabbbbccccdddd".into(),
            remote_version: 4,
            remote_author: "11112222333344445".into(),
            detected_at: 0,
            state: "active".into(),
        }];
    });
    let mut harness = egui_kittest::Harness::new_ui(|ui| {
        ui::conflicts::show(ui, &conn);
    });
    harness.run();
    // ファイルパスと解決ボタンが描画される
    let _ = harness.get_by_label_contains("src/main.rs");
    let _ = harness.get_by_label("Keep Local");
    let _ = harness.get_by_label("Accept Remote");
    let _ = harness.get_by_label("Manual");
}

#[test]
fn settings_renders_network_tuning_form() {
    let conn = seeded_connection(|cache| {
        cache.network = Some(NetworkStatusInfo {
            primary_route: "Direct".into(),
            total_bandwidth_bps: 1_000_000,
            used_bandwidth_bps: 256_000,
            active_connections: 2,
            max_connections: 8,
            avg_latency_ms: 42,
        });
    });
    let mut harness = egui_kittest::Harness::new_ui(|ui| {
        ui::settings::show(ui, &conn);
    });
    harness.run();
    // Settings タブは collapsing header が初期状態で閉じている。
    // ヘッダ "Daemon" / "About" などは常に見えるのでそれらで検証する。
    let _ = harness.get_by_label("Daemon");
    let _ = harness.get_by_label("About");
}
