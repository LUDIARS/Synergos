// @implements SPEC-SY-opaque-custody
pub mod config;
mod contract;
mod durable_file;
mod error;
mod handlers;
mod signature;
mod store;

use axum::{
    extract::DefaultBodyLimit,
    middleware,
    routing::{get, post},
    Router,
};
use config::Config;
use std::sync::Arc;
use store::Store;
pub(crate) struct State {
    config: Config,
    store: Store,
}

/// Embedded in synergos-control; no second listener or implicit backend fallback.
pub async fn router(config: Config) -> std::io::Result<Router> {
    config.validate()?;
    let store = Store::open(config.root.clone()).await?;
    let state = Arc::new(State { config, store });
    let admin = Router::new()
        .route("/v1/custody/health", get(handlers::health))
        .route("/v1/custody/admissions", post(handlers::admit))
        .route("/v1/custody/verify", post(handlers::verify))
        .route("/v1/custody/head", post(handlers::head))
        .route("/v1/custody/download", post(handlers::download))
        .route("/v1/custody/reclaim", post(handlers::reclaim))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            handlers::authenticate,
        ))
        .layer(DefaultBodyLimit::max(64 * 1024));
    let transfers = Router::new()
        .route(
            "/v1/custody/transfer/:key",
            get(handlers::get_bytes)
                .put(handlers::put_bytes)
                .options(|| async { axum::http::StatusCode::NO_CONTENT }),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            handlers::cors,
        ));
    Ok(admin
        .merge(transfers)
        .with_state(state)
        .layer(middleware::from_fn(handlers::no_cache)))
}
