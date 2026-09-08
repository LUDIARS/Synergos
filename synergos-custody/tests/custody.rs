// @implements SPEC-SY-opaque-custody
// @implements SPEC-SY-custody-acquisition
// @implements SPEC-SY-custody-durability
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use chrono::{SecondsFormat, Utc};
use serde_json::{json, Value};
use synergos_custody::{config::Config, router};
use tower::ServiceExt;
// Public dummy credentials belong only to the in-process test Router.
const TOKEN: &str = "dummy-server-token-for-cu-synergos-tests";
const SECRET: &str = "dummy-receipt-key-for-cu-synergos-tests";
const HASH: &str = "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
fn config(root: &std::path::Path) -> Config {
    Config {
        root: root.to_owned(),
        public_url: "https://sy.example.test".parse().unwrap(),
        node_id: "11111111-1111-4111-8111-111111111111".into(),
        token: TOKEN.into(),
        receipt_secret: SECRET.into(),
        max_object_bytes: 1024,
        client_origins: vec!["https://cu.example.test".into()],
    }
}
fn admission() -> Value {
    json!({"operationId":uuid::Uuid::new_v4().to_string(),"opaqueObjectId":"uploads/scene/object-1","contentHash":HASH,"byteLength":3,
        "expiresAt":(Utc::now()+chrono::Duration::hours(1)).to_rfc3339_opts(SecondsFormat::Millis,true),"contentType":"application/octet-stream"})
}
async fn command(app: &Router, path: &str, payload: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/custody/{path}"))
                .header("Authorization", format!("Bearer {TOKEN}"))
                .header("Content-Type", "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let data = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, serde_json::from_slice(&data).unwrap())
}
fn request_url(url: &str) -> String {
    let url = url::Url::parse(url).unwrap();
    format!("{}?{}", url.path(), url.query().unwrap())
}
async fn put(app: &Router, url: &str, bytes: &'static str) -> StatusCode {
    app.clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(request_url(url))
                .header("Content-Type", "application/octet-stream")
                .header("If-None-Match", "*")
                .header("X-Content-SHA256", HASH)
                .body(Body::from(bytes))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}
#[tokio::test]
async fn validates_bytes_before_receipt_and_retains_objects_across_restart() {
    let dir = tempfile::tempdir().unwrap();
    let app = router(config(dir.path())).await.unwrap();
    let input = admission();
    let (status, accepted) = command(&app, "admissions", input.clone()).await;
    assert_eq!(status, StatusCode::OK);
    let url = accepted["uploadUrl"].as_str().unwrap();
    let object = json!({"opaqueObjectId":input["opaqueObjectId"]});
    assert_eq!(
        command(&app, "verify", object.clone()).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(put(&app, url, "bad").await, StatusCode::CONFLICT);
    assert_eq!(
        command(&app, "verify", object.clone()).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(put(&app, url, "abc").await, StatusCode::CREATED);
    assert_eq!(put(&app, url, "abc").await, StatusCode::PRECONDITION_FAILED);
    let (status, verified) = command(&app, "verify", object.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(verified["receipt"]["byteLength"], 3);
    assert_eq!(
        verified["receipt"]["verifiedDigests"][0]["value"],
        &HASH[7..]
    );
    assert_eq!(verified["receipt"]["operationId"], input["operationId"]);
    assert_eq!(verified["receipt"]["durability"], "durable");
    assert!(verified["signature"].as_str().unwrap().len() == 64);
    assert!(!verified.to_string().contains(TOKEN));
    let (status,download)=command(&app,"download",json!({"opaqueObjectId":input["opaqueObjectId"],"expiresIn":300,"downloadName":"version.mb"})).await;
    assert_eq!(status, StatusCode::OK);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(request_url(download["downloadUrl"].as_str().unwrap()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        &to_bytes(response.into_body(), 32).await.unwrap()[..],
        b"abc"
    );
    assert!(
        router(config(dir.path())).await.is_err(),
        "a second store must not claim the same root"
    );
    drop(app);
    let reopened = router(config(dir.path())).await.unwrap();
    assert_eq!(
        command(&reopened, "head", object).await.1["contentHash"],
        HASH
    );
}
#[tokio::test]
async fn rejects_changed_intents_and_old_capabilities_after_reclaim() {
    let dir = tempfile::tempdir().unwrap();
    let app = router(config(dir.path())).await.unwrap();
    let input = admission();
    let accepted = command(&app, "admissions", input.clone()).await.1;
    let url = accepted["uploadUrl"].as_str().unwrap();
    assert_eq!(
        command(&app, "admissions", input.clone()).await.0,
        StatusCode::OK
    );
    let mut changed = input.clone();
    changed["opaqueObjectId"] = json!("another-object");
    assert_eq!(
        command(&app, "admissions", changed).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(put(&app, url, "abc").await, StatusCode::CREATED);
    let object = json!({"opaqueObjectId":input["opaqueObjectId"]});
    assert_eq!(
        command(&app, "reclaim", object.clone()).await.0,
        StatusCode::OK
    );
    assert_eq!(
        command(&app, "reclaim", object.clone()).await.0,
        StatusCode::OK
    );
    assert_eq!(put(&app, url, "abc").await, StatusCode::CONFLICT);
    drop(app);
    let reopened = router(config(dir.path())).await.unwrap();
    assert_eq!(put(&reopened, url, "abc").await, StatusCode::CONFLICT);
    assert_eq!(
        command(&reopened, "admissions", input).await.0,
        StatusCode::CONFLICT
    );
}
#[tokio::test]
async fn enforces_server_authority_capability_method_and_browser_origin() {
    let dir = tempfile::tempdir().unwrap();
    let app = router(config(dir.path())).await.unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/custody/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let accepted = command(&app, "admissions", admission()).await.1;
    let url = request_url(accepted["uploadUrl"].as_str().unwrap());
    let response = app
        .clone()
        .oneshot(Request::builder().uri(&url).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri(&url)
                .header("Origin", "https://evil.example.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri(&url)
                .header("Origin", "https://cu.example.test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        response.headers()["Access-Control-Allow-Origin"],
        "https://cu.example.test"
    );
}
