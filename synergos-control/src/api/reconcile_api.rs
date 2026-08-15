use std::sync::Arc;

use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use tracing::{info, warn};

use crate::error::ControlResult;
use crate::reconcile::{classify, ReconcileReport};
use crate::store::now_ms;

use super::AppState;

#[derive(Debug, Default, Deserialize)]
pub struct ReconcileRequest {
    /// true にすると dark device を失効・dark connector を削除する。
    /// 既定 false (レポートのみ)。破壊的操作は明示要求時に限る。
    #[serde(default)]
    pub revoke_dark: bool,
}

/// Cloudflare の実態とレジストリを突合し、dark node を検出する。
pub async fn run_reconcile(
    State(state): State<Arc<AppState>>,
    body: Option<Json<ReconcileRequest>>,
) -> ControlResult<Json<ReconcileReport>> {
    let req = body.map(|Json(r)| r).unwrap_or_default();

    let snapshot = state.store.snapshot().await;
    let connectors = state.cloudflare.list_mesh_connectors().await?;
    let registrations = state.cloudflare.list_device_registrations().await?;

    let mut report = classify(
        &snapshot.orgs,
        &snapshot.nodes,
        &connectors,
        &registrations,
        now_ms(),
    );

    info!(
        dark_connectors = report.dark_connectors.len(),
        dark_devices = report.dark_devices.len(),
        missing_connectors = report.missing_connectors.len(),
        mesh_ip_mismatches = report.mesh_ip_mismatches.len(),
        "reconcile completed"
    );

    if req.revoke_dark {
        let device_ids: Vec<String> = report.dark_devices.iter().map(|d| d.id.clone()).collect();
        if !device_ids.is_empty() {
            match state
                .cloudflare
                .revoke_device_registrations(&device_ids)
                .await
            {
                Ok(()) => report
                    .actions
                    .push(format!("revoked {} dark device(s)", device_ids.len())),
                Err(err) => {
                    warn!(error = %err, "failed to revoke dark devices");
                    report.actions.push(format!(
                        "FAILED to revoke {} dark device(s)",
                        device_ids.len()
                    ));
                }
            }
        }
        for connector in &report.dark_connectors {
            match state.cloudflare.delete_mesh_connector(&connector.id).await {
                Ok(_) => report
                    .actions
                    .push(format!("deleted dark connector {}", connector.id)),
                Err(err) => {
                    // 1 件の失敗で全体を止めず、結果に失敗を明記する
                    warn!(connector = %connector.id, error = %err, "failed to delete dark connector");
                    report
                        .actions
                        .push(format!("FAILED to delete dark connector {}", connector.id));
                }
            }
        }
    }

    Ok(Json(report))
}
