use crate::store::{Query, QueryLimits, Store, QueryResult, RouteAttrs, RouterId, ResolvedRouteAttrs, ResolvedNexthop};
use axum::body::StreamBody;
use axum::extract::{Query as AxumQuery, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use hickory_resolver::TokioAsyncResolver;
use futures_util::{FutureExt, StreamExt};
use log::*;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

#[derive(Debug, Clone, Deserialize)]
pub struct ApiServerConfig {
    bind: SocketAddr,
    #[serde(default)]
    query_limits: QueryLimits,
}

#[derive(Debug, Clone, Serialize)]
pub enum NexthopResolved {
    ReverseDns(String),
    RouterId(RouterId),
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RouteAttrsResolved {
    #[serde(flatten)]
    inner: RouteAttrs,
    communities_resolved: HashMap<(u16, u16), String>,
    large_communities_resolved: HashMap<(u32, u32, u32), String>,
    nexthop_resolved: Option<NexthopResolved>,
}


async fn query<T: Store>(
    State((cfg, resolver, store)): State<(Arc<ApiServerConfig>, TokioAsyncResolver, T)>,
    AxumQuery(mut query): AxumQuery<Query>,
) -> impl IntoResponse {
    trace!("request: {}", serde_json::to_string_pretty(&query).unwrap());
    let mut limits = query.limits.take().unwrap_or(cfg.query_limits.clone());
    limits.max_results = std::cmp::min(limits.max_results, cfg.query_limits.max_results);
    limits.max_results_per_table = std::cmp::min(
        limits.max_results_per_table,
        cfg.query_limits.max_results_per_table,
    );
    query.limits = Some(limits);
    let stream = store.get_routes(query)
    .then(move |route| {
        let resolver = resolver.clone();
        async move {
            QueryResult {
                client: route.client, net: route.net, session: route.session,
                state: route.state, table: route.table,
                attrs: ResolvedRouteAttrs {
                    resolved_communities: Default::default(),
                    resolved_large_communities: Default::default(),
                    resolved_nexthop: match route.attrs.nexthop.as_ref() {
                        Some(nexthop) => match resolver.reverse_lookup(*nexthop).await.ok().and_then(|reverse| reverse.iter().next().map(|x| x.0.clone())) {
                            Some(reverse) => ResolvedNexthop::ReverseDns(reverse.to_string()),
                            None => ResolvedNexthop::None,
                        }
                        None => ResolvedNexthop::None,
                    },
                    inner: route.attrs,
                },
            }
        }
    })
    .map(|route| {
        let json = serde_json::to_string(&route).unwrap();
        Ok::<_, Infallible>(format!("{}\n", json))
    });
    StreamBody::new(stream)
}

async fn routers<T: Store>(
    State((cfg, _, store)): State<(Arc<ApiServerConfig>, TokioAsyncResolver, T)>,
) -> impl IntoResponse {
    serde_json::to_string(&store.get_routers()).unwrap()
}

fn make_api<T: Store>(cfg: ApiServerConfig, store: T) -> Router {
    Router::new()
        .route("/query", get(query::<T>))
        .route("/routers", get(routers::<T>))
        .with_state((Arc::new(cfg), TokioAsyncResolver::tokio_from_system_conf().unwrap(), store))
}

/// This handler serializes the metrics into a string for Prometheus to scrape
pub async fn get_metrics() -> (StatusCode, String) {
    match autometrics::encode_global_metrics() {
        Ok(metrics) => (StatusCode::OK, metrics),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{:?}", err)),
    }
}

pub async fn run_api_server<T: Store>(
    cfg: ApiServerConfig,
    store: T,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<()> {
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
