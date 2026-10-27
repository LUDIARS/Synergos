//! Loopback-only HTTP fixtures, not the production service or its credentials.
use super::*;
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tokio::sync::Mutex;

#[derive(Default)]
struct Storage {
    blocks: Mutex<HashMap<String, Vec<u8>>>,
    failure: Mutex<Option<String>>,
    writes: Mutex<Vec<String>>,
    forbidden_hits: AtomicUsize,
}

struct Fixture {
    state: Arc<Storage>,
    origin: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn download(
    State(state): State<Arc<Storage>>,
    Path(cid): Path<String>,
    headers: HeaderMap,
) -> Response {
    if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some("Bearer fixture-only") {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if cid == "redirect" {
        return (StatusCode::FOUND, [("location", "/forbidden")]).into_response();
    }
    let mut failure = state.failure.lock().await;
    if failure.as_deref() == Some(&cid) {
        *failure = None;
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    drop(failure);
    match state.blocks.lock().await.get(&cid) {
        Some(bytes) => bytes.clone().into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn upload(
    State(state): State<Arc<Storage>>,
    Path(cid): Path<String>,
    headers: HeaderMap,
    bytes: Bytes,
) -> StatusCode {
    if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some("Bearer fixture-only") {
        return StatusCode::UNAUTHORIZED;
    }
    let mut blocks = state.blocks.lock().await;
    if blocks.contains_key(&cid) {
        return StatusCode::PRECONDITION_FAILED;
    }
    blocks.insert(cid.clone(), bytes.to_vec());
    state.writes.lock().await.push(cid);
    StatusCode::CREATED
}

impl Fixture {
    async fn start() -> Self {
        let state = Arc::new(Storage::default());
        let router = Router::new()
            .route("/blocks/:cid", get(download).put(upload))
            .route(
                "/forbidden",
                get(|State(s): State<Arc<Storage>>| async move {
                    s.forbidden_hits.fetch_add(1, Ordering::SeqCst);
                    StatusCode::OK
                }),
            )
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self {
            state,
            origin,
            task,
        }
    }

    fn options(&self) -> TransferOptions {
        TransferOptions {
            allowed_origins: vec![self.origin.clone()],
            ca_bundle: None,
            concurrency: 1,
            retries: 0,
        }
    }

    fn plan(&self, manifest: &Manifest) -> TransferPlan {
        TransferPlan {
            manifest: manifest.clone(),
            grants: manifest
                .blocks
                .iter()
                .map(|b| {
                    let headers =
                        BTreeMap::from([("Authorization".into(), "Bearer fixture-only".into())]);
                    let url = format!("{}/blocks/{}?capability=fixture-secret", self.origin, b.cid);
                    TransferGrant {
                        cid: b.cid.clone(),
                        get_url: url.clone(),
                        put_url: Some(url),
                        get_headers: headers.clone(),
                        put_headers: headers,
                    }
                })
                .collect(),
        }
    }
}

#[tokio::test]
async fn real_http_push_pull_interruption_and_cache_resume() {
    let server = Fixture::start().await;
    let dir = tempfile::tempdir().unwrap();
    let input: Vec<u8> = (0..5u8)
        .flat_map(|n| std::iter::repeat_n(n, 65536))
        .collect();
    let source = dir.path().join("source");
    tokio::fs::write(&source, &input).await.unwrap();
    let sender = Arc::new(BlockCache::open(&dir.path().join("sender")).unwrap());
    let manifest = prepare(&sender, &source, 65536).await.unwrap();
    let mut options = server.options();
    options.concurrency = 3;
    let first = push(
        sender.clone(),
        server.plan(&manifest),
        &manifest.root,
        options,
    )
    .await
    .unwrap();
    assert_eq!(first.transferred_blocks, manifest.blocks.len());
    assert_eq!(
        server.state.writes.lock().await.last().unwrap(),
        &manifest.root
    );
    let again = push(
        sender,
        server.plan(&manifest),
        &manifest.root,
        server.options(),
    )
    .await
    .unwrap();
    assert_eq!(again.transferred_blocks, 0);
    assert_eq!(
        server.state.writes.lock().await.len(),
        manifest.blocks.len()
    );

    let receiver_path = dir.path().join("receiver");
    let receiver = Arc::new(BlockCache::open(&receiver_path).unwrap());
    let fail_cid = manifest
        .blocks
        .iter()
        .filter(|b| b.cid != manifest.root)
        .last()
        .unwrap()
        .cid
        .clone();
    *server.state.failure.lock().await = Some(fail_cid);
    let destination = dir.path().join("received");
    assert!(pull(
        receiver.clone(),
        server.plan(&manifest),
        &manifest.root,
        &destination,
        server.options()
    )
    .await
    .is_err());
    assert!(!destination.exists());
    drop(receiver);
    let receiver = Arc::new(BlockCache::open(&receiver_path).unwrap());
    let resumed = pull(
        receiver,
        server.plan(&manifest),
        &manifest.root,
        &destination,
        server.options(),
    )
    .await
    .unwrap();
    assert!(resumed.reused_blocks >= 2);
    assert_eq!(resumed.transferred_blocks, 1);
    assert_eq!(tokio::fs::read(destination).await.unwrap(), input);
}

#[tokio::test]
async fn rejected_redirect_auth_and_corruption_do_not_publish_or_leak() {
    let server = Fixture::start().await;
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    tokio::fs::write(&source, b"hello").await.unwrap();
    let sender = Arc::new(BlockCache::open(&dir.path().join("sender")).unwrap());
    let manifest = prepare(&sender, &source, 65536).await.unwrap();
    push(
        sender,
        server.plan(&manifest),
        &manifest.root,
        server.options(),
    )
    .await
    .unwrap();
    let receiver = Arc::new(BlockCache::open(&dir.path().join("receiver")).unwrap());
    let destination = dir.path().join("output");
    let mut plan = server.plan(&manifest);
    plan.grants
        .iter_mut()
        .find(|g| g.cid == manifest.root)
        .unwrap()
        .get_url = format!("{}/blocks/redirect?secret=do-not-print", server.origin);
    let error = pull(
        receiver.clone(),
        plan,
        &manifest.root,
        &destination,
        server.options(),
    )
    .await
    .unwrap_err();
    assert!(!error.to_string().contains("do-not-print"));
    assert_eq!(server.state.forbidden_hits.load(Ordering::SeqCst), 0);
    assert!(!destination.exists());
    let mut plan = server.plan(&manifest);
    plan.grants
        .iter_mut()
        .find(|g| g.cid == manifest.root)
        .unwrap()
        .get_headers
        .clear();
    assert!(pull(
        receiver.clone(),
        plan,
        &manifest.root,
        &destination,
        server.options()
    )
    .await
    .is_err());
    server
        .state
        .blocks
        .lock()
        .await
        .get_mut(&manifest.root)
        .unwrap()[0] ^= 1;
    assert!(pull(
        receiver,
        server.plan(&manifest),
        &manifest.root,
        &destination,
        server.options()
    )
    .await
    .is_err());
    assert!(!destination.exists());
}

#[tokio::test]
async fn transient_failure_retries_and_missing_upload_grant_sends_nothing() {
    let server = Fixture::start().await;
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    tokio::fs::write(&source, vec![7; 65536]).await.unwrap();
    let sender = Arc::new(BlockCache::open(&dir.path().join("sender")).unwrap());
    let manifest = prepare(&sender, &source, 65536).await.unwrap();
    let mut incomplete = server.plan(&manifest);
    incomplete.grants.pop();
    assert!(
        push(sender.clone(), incomplete, &manifest.root, server.options())
            .await
            .is_err()
    );
    assert!(server.state.blocks.lock().await.is_empty());
    push(
        sender,
        server.plan(&manifest),
        &manifest.root,
        server.options(),
    )
    .await
    .unwrap();
    *server.state.failure.lock().await = Some(manifest.root.clone());
    let receiver = Arc::new(BlockCache::open(&dir.path().join("receiver")).unwrap());
    let mut options = server.options();
    options.retries = 1;
    let output = dir.path().join("output");
    let report = pull(
        receiver,
        server.plan(&manifest),
        &manifest.root,
        &output,
        options,
    )
    .await
    .unwrap();
    assert_eq!(report.transferred_blocks, manifest.blocks.len());
    assert_eq!(tokio::fs::read(output).await.unwrap(), vec![7; 65536]);
}
