use axum::body::StreamBody;
use axum::extract::{Query as AxumQuery, State};
use axum::response::IntoResponse;
use axum::Router;
use axum::routing::get;
use crate::table::Query;
use crate::table::Table;
use futures_util::StreamExt;
use std::convert::Infallible;
use std::net::SocketAddr;
use serde::Deserialize;
use log::*;

#[derive(Debug, Clone, Deserialize)]
pub struct ApiServerConfig {
    bind: SocketAddr,
}

async fn query<T: Table>(State(table): State<T>, AxumQuery(query): AxumQuery<Query>) -> impl IntoResponse {
    trace!("request: {}", serde_json::to_string_pretty(&query).unwrap());
    let stream = table.get_routes(query)
        .map(|route| {
            let json = serde_json::to_string(&route).unwrap();
             Ok::<_, Infallible>(format!("{}\n", json))
        });
    StreamBody::new(stream)
}

fn make_api<T: Table>(table: T) -> Router {
    Router::new()
        .route("/query", get(query::<T>))
        .with_state(table)
}

pub async fn run_api_server<T: Table>(cfg: ApiServerConfig, table: T) -> anyhow::Result<()> {
    let make_service = Router::new()
        .nest("/api", make_api(table))
        .into_make_service();

    axum::Server::bind(&cfg.bind)
        .serve(make_service).await?;

    Ok(())
}
