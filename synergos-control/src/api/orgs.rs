use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;

use crate::error::{ControlError, ControlResult};
use crate::store::{now_ms, Org};

use super::AppState;

#[derive(Debug, Deserialize)]
pub struct CreateOrgRequest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub members: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateOrgRequest {
    pub name: Option<String>,
    pub members: Option<Vec<String>>,
}

pub async fn create_org(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateOrgRequest>,
) -> ControlResult<Json<Org>> {
    validate_slug(&req.id)?;
    let org = Org {
        id: req.id,
        name: req.name,
        members: normalize_members(req.members)?,
        created_at_ms: now_ms(),
    };
    Ok(Json(state.store.insert_org(org).await?))
}

pub async fn list_orgs(State(state): State<Arc<AppState>>) -> Json<Vec<Org>> {
    Json(state.store.snapshot().await.orgs)
}

pub async fn get_org(
    State(state): State<Arc<AppState>>,
    Path(org_id): Path<String>,
) -> ControlResult<Json<Org>> {
    Ok(Json(state.store.get_org(&org_id).await?))
}

pub async fn update_org(
    State(state): State<Arc<AppState>>,
    Path(org_id): Path<String>,
    Json(req): Json<UpdateOrgRequest>,
) -> ControlResult<Json<Org>> {
    let mut org = state.store.get_org(&org_id).await?;
    if let Some(name) = req.name {
        org.name = name;
    }
    if let Some(members) = req.members {
        org.members = normalize_members(members)?;
    }
    Ok(Json(state.store.update_org(org).await?))
}

fn validate_slug(id: &str) -> ControlResult<()> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if ok {
        Ok(())
    } else {
        Err(ControlError::InvalidRequest(
            "org id must be a lowercase slug ([a-z0-9-], max 64 chars)".to_string(),
        ))
    }
}

fn normalize_members(members: Vec<String>) -> ControlResult<Vec<String>> {
    let mut out = Vec::with_capacity(members.len());
    for m in members {
        let m = m.trim().to_ascii_lowercase();
        if m.is_empty() || !m.contains('@') {
            return Err(ControlError::InvalidRequest(format!(
                "member must be an email address: {m:?}"
            )));
        }
        if !out.contains(&m) {
            out.push(m);
        }
    }
    Ok(out)
}
