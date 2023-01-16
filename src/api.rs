use axum::body::StreamBody;
use axum::extract::{Query as AxumQuery, State};
use axum::response::IntoResponse;
use axum::Router;
use axum::routing::get;
use crate::table::{Query, QueryLimits, Table};
use futures_util::StreamExt;
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

async fn query<T: Table>(State((cfg, table)): State<(Arc<ApiServerConfig>, T)>, AxumQuery(mut query): AxumQuery<Query>) -> impl IntoResponse {
    trace!("request: {}", serde_json::to_string_pretty(&query).unwrap());
    let mut limits = query.limits.take().unwrap_or(cfg.query_limits.clone());
    limits.max_results = std::cmp::min(limits.max_results, cfg.query_limits.max_results);
    limits.max_results_per_table = std::cmp::min(limits.max_results_per_table, cfg.query_limits.max_results_per_table);
    query.limits = Some(limits);
    let stream = table.get_routes(query)
        .map(|route| {
            let json = serde_json::to_string(&route).unwrap();
             Ok::<_, Infallible>(format!("{}\n", json))
        });
    StreamBody::new(stream)
}

fn make_api<T: Table>(cfg: ApiServerConfig, table: T) -> Router {
    Router::new()
        .route("/query", get(query::<T>))
        .with_state((Arc::new(cfg), table))
}

pub async fn run_api_server<T: Table>(cfg: ApiServerConfig, table: T) -> anyhow::Result<()> {
    let make_service = Router::new()
        .nest("/api", make_api(cfg.clone(), table))
        .into_make_service();

    axum::Server::bind(&cfg.bind)
        .serve(make_service).await?;

    Ok(())
}
