use axum::body::StreamBody;
use axum::http::StatusCode;
use axum::extract::{Query as AxumQuery, State};
use axum::response::IntoResponse;
use axum::Router;
use axum::routing::get;
use crate::store::{Query, QueryLimits, Store};
use futures_util::{StreamExt, FutureExt};
use std::sync::Arc;
use std::convert::Infallible;
use std::net::SocketAddr;
use serde::Deserialize;
use log::*;

#[derive(Debug, Clone, Deserialize)]
pub struct ApiServerConfig {
    bind: SocketAddr,
    #[serde(default)]
    query_limits: QueryLimits,
}

async fn query<T: Store>(State((cfg, store)): State<(Arc<ApiServerConfig>, T)>, AxumQuery(mut query): AxumQuery<Query>) -> impl IntoResponse {
    trace!("request: {}", serde_json::to_string_pretty(&query).unwrap());
    let mut limits = query.limits.take().unwrap_or(cfg.query_limits.clone());
    limits.max_results = std::cmp::min(limits.max_results, cfg.query_limits.max_results);
    limits.max_results_per_table = std::cmp::min(limits.max_results_per_table, cfg.query_limits.max_results_per_table);
    query.limits = Some(limits);
    let stream = store.get_routes(query)
        .map(|route| {
            let json = serde_json::to_string(&route).unwrap();
             Ok::<_, Infallible>(format!("{}\n", json))
        });
    StreamBody::new(stream)
}

fn make_api<T: Store>(cfg: ApiServerConfig, store: T) -> Router {
    Router::new()
        .route("/query", get(query::<T>))
        .with_state((Arc::new(cfg), store))
}

/// This handler serializes the metrics into a string for Prometheus to scrape
pub async fn get_metrics() -> (StatusCode, String) {
    match autometrics::encode_global_metrics() {
        Ok(metrics) => (StatusCode::OK, metrics),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{:?}", err)),
    }
}

pub async fn run_api_server<T: Store>(cfg: ApiServerConfig, store: T, mut shutdown: tokio::sync::watch::Receiver<bool>) -> anyhow::Result<()> {
    let make_service = Router::new()
        .nest("/api", make_api(cfg.clone(), store))
        .route("/metrics", get(get_metrics))
        .into_make_service();

    axum::Server::bind(&cfg.bind)
        .serve(make_service)
        .with_graceful_shutdown(shutdown.changed().map(|_| ()))
        .await?;

    Ok(())
}
