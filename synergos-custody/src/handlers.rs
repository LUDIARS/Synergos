// @implements SPEC-SY-opaque-custody
// @implements SPEC-SY-custody-acquisition
use crate::{
    contract::{hex_digest, Admission, Capability, DownloadRequest, ObjectRequest},
    error::{Error, Result},
    signature, State,
};
use axum::{
    body::Body,
    extract::{Path, Query, Request, State as ExtractState},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use chrono::{SecondsFormat, Utc};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio_util::io::ReaderStream;

pub async fn authenticate(
    ExtractState(state): ExtractState<Arc<State>>,
    request: Request,
    next: Next,
) -> Response {
    let provided = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    // Compare HMACs of both sides rather than the raw strings: equal digests imply an
    // equal token, and the comparison itself is constant time and length independent.
    let expected = signature::sign(&state.config.receipt_secret, &state.config.token);
    if !provided
        .is_some_and(|v| signature::verify(&state.config.receipt_secret, v, &expected))
    {
        return Error::Denied.into_response();
    }
    next.run(request).await
}
pub async fn no_cache(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}
pub async fn cors(
    ExtractState(state): ExtractState<Arc<State>>,
    request: Request,
    next: Next,
) -> Response {
    let origin = request.headers().get(header::ORIGIN).cloned();
    if let Some(origin) = &origin {
        if !origin
            .to_str()
            .ok()
            .is_some_and(|v| state.config.client_origins.iter().any(|s| s == v))
        {
            return Error::Denied.into_response();
        }
    }
    let mut response = next.run(request).await;
    if let Some(origin) = origin {
        response
            .headers_mut()
            .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        response
            .headers_mut()
            .insert(header::VARY, HeaderValue::from_static("Origin"));
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("GET, PUT, OPTIONS"),
        );
        response.headers_mut().insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("Content-Type, If-None-Match, X-Content-SHA256"),
        );
    }
    response
}
pub async fn health(ExtractState(state): ExtractState<Arc<State>>) -> Json<Value> {
    Json(
        json!({"ready":true,"backend":"synergos","durability":"durable","nodeId":state.config.node_id}),
    )
}
pub async fn admit(
    ExtractState(state): ExtractState<Arc<State>>,
    Json(input): Json<Admission>,
) -> Result<Json<Value>> {
    input.validate(state.config.max_object_bytes, Utc::now())?;
    let key = state.store.admit(&input).await?;
    let deadline = chrono::DateTime::parse_from_rfc3339(&input.expires_at)
        .map_err(|_| Error::Invalid)?
        .timestamp_millis();
    let expires = deadline.min(Utc::now().timestamp_millis() + 900_000);
    Ok(Json(
        json!({"uploadUrl":capability_url(&state,"PUT",&key,expires,""),"requiredHeaders":{
            "Content-Type":input.content_type,"If-None-Match":"*","X-Content-SHA256":input.content_hash
        }}),
    ))
}
pub async fn verify(
    ExtractState(state): ExtractState<Arc<State>>,
    Json(input): Json<ObjectRequest>,
) -> Result<Json<Value>> {
    let key = signature::object_key(&input.opaque_object_id);
    let _guard = state.store.lock(&key).await?;
    let admission = state.store.admission(&key).await?;
    if admission.expired(Utc::now()) {
        return Err(Error::Conflict);
    }
    if !state.store.verify_file(&key, &admission).await? {
        return Err(Error::NotFound);
    }
    let expires = chrono::DateTime::parse_from_rfc3339(&admission.expires_at)
        .map_err(|_| Error::Unavailable)?
        .with_timezone(&Utc)
        .min(Utc::now() + chrono::Duration::minutes(5));
    let receipt = json!({"operationId":admission.operation_id,"opaqueObjectId":admission.opaque_object_id,
        "verifiedDigests":[{"algorithm":"sha256","value":&admission.content_hash[7..]}],"byteLength":admission.byte_length,
        "durability":"durable","locations":[state.config.node_id],"expiresAt":expires.to_rfc3339_opts(SecondsFormat::Millis,true)});
    let signature = signature::sign(
        &state.config.receipt_secret,
        &signature::canonical(&receipt),
    );
    Ok(Json(json!({"receipt":receipt,"signature":signature})))
}
pub async fn head(
    ExtractState(state): ExtractState<Arc<State>>,
    Json(input): Json<ObjectRequest>,
) -> Result<Json<Value>> {
    let key = signature::object_key(&input.opaque_object_id);
    let _guard = state.store.lock(&key).await?;
    let admission = match state.store.admission(&key).await {
        Ok(v) => v,
        Err(Error::NotFound) => return Ok(Json(Value::Null)),
        Err(e) => return Err(e),
    };
    if !state.store.present(&key, &admission).await? {
        return Ok(Json(Value::Null));
    }
    Ok(Json(
        json!({"contentLength":admission.byte_length,"contentHash":admission.content_hash,"contentType":admission.content_type}),
    ))
}
pub async fn download(
    ExtractState(state): ExtractState<Arc<State>>,
    Json(input): Json<DownloadRequest>,
) -> Result<Json<Value>> {
    if !(1..=900).contains(&input.expires_in)
        || input.download_name.is_empty()
        || input.download_name.len() > 100
        || !input
            .download_name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        return Err(Error::Invalid);
    }
    let key = signature::object_key(&input.opaque_object_id);
    let _guard = state.store.lock(&key).await?;
    let admission = state.store.admission(&key).await?;
    if !state.store.present(&key, &admission).await? {
        return Err(Error::NotFound);
    }
    Ok(Json(
        json!({"downloadUrl":capability_url(&state,"GET",&key,Utc::now().timestamp_millis()+i64::from(input.expires_in)*1000,&input.download_name)}),
    ))
}
pub async fn reclaim(
    ExtractState(state): ExtractState<Arc<State>>,
    Json(input): Json<ObjectRequest>,
) -> Result<Json<Value>> {
    // Cu owns the no-version-reference decision; only its server credential reaches this route.
    state
        .store
        .reclaim(&signature::object_key(&input.opaque_object_id))
        .await?;
    Ok(Json(json!({"reclaimed":true})))
}
fn capability_url(state: &State, method: &str, key: &str, expires: i64, name: &str) -> String {
    let signature = signature::sign(
        &state.config.receipt_secret,
        &signature::capability_message(method, key, expires, name),
    );
    let mut url = state.config.public_url.clone();
    url.set_path(&format!("/v1/custody/transfer/{key}"));
    url.query_pairs_mut()
        .append_pair("expires", &expires.to_string())
        .append_pair("signature", &signature)
        .append_pair("name", name);
    url.to_string()
}
fn check_capability(state: &State, method: &str, key: &str, capability: &Capability) -> Result<()> {
    if !hex_digest(key)
        || capability.expires <= Utc::now().timestamp_millis()
        || !signature::verify(
            &state.config.receipt_secret,
            &signature::capability_message(method, key, capability.expires, &capability.name),
            &capability.signature,
        )
    {
        return Err(Error::Denied);
    }
    Ok(())
}
pub async fn put_bytes(
    ExtractState(state): ExtractState<Arc<State>>,
    Path(key): Path<String>,
    Query(capability): Query<Capability>,
    headers: HeaderMap,
    body: Body,
) -> Result<StatusCode> {
    check_capability(&state, "PUT", &key, &capability)?;
    let _guard = state.store.lock(&key).await?;
    let admission = state.store.admission(&key).await?;
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        != Some("*")
        || headers
            .get("X-Content-SHA256")
            .and_then(|v| v.to_str().ok())
            != Some(admission.content_hash.as_str())
        || headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            != Some(admission.content_type.as_str())
    {
        return Err(Error::Invalid);
    }
    tokio::time::timeout(
        std::time::Duration::from_secs(1800),
        state.store.put(&key, &admission, body),
    )
    .await
    .map_err(|_| Error::Unavailable)??;
    Ok(StatusCode::CREATED)
}
pub async fn get_bytes(
    ExtractState(state): ExtractState<Arc<State>>,
    Path(key): Path<String>,
    Query(capability): Query<Capability>,
) -> Result<Response> {
    check_capability(&state, "GET", &key, &capability)?;
    let _guard = state.store.lock(&key).await?;
    let admission = state.store.admission(&key).await?;
    if !state.store.present(&key, &admission).await? {
        return Err(Error::NotFound);
    }
    let file = tokio::fs::File::open(state.store.path(&key, "data")).await?;
    let mut response = Body::from_stream(ReaderStream::new(file)).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&admission.byte_length.to_string())
            .map_err(|_| Error::Unavailable)?,
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{}\"", capability.name))
            .map_err(|_| Error::Invalid)?,
    );
    Ok(response)
}
