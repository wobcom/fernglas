use crate::store::{Query, QueryLimits, Store, QueryResult, RouteAttrs, RouterId, ResolvedRouteAttrs, ResolvedNexthop, NetQuery};
use axum::body::StreamBody;
use axum::extract::{Query as AxumQuery, State};
use axum::http::StatusCode;
use axum::response::{Response, IntoResponse};
use axum::routing::get;
use axum::Router;
use hickory_resolver::config::LookupIpStrategy;
use hickory_resolver::TokioAsyncResolver;
use futures_util::{FutureExt, StreamExt};
use log::*;
use serde::{Serialize, Deserialize};
use ipnet::IpNet;
use std::net::IpAddr;
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

#[cfg(feature = "embed-static")]
static STATIC_DIR: include_dir::Dir<'_> = include_dir::include_dir!("$CARGO_MANIFEST_DIR/static");

#[derive(Debug, Clone, Deserialize)]
pub struct ApiServerConfig {
    bind: SocketAddr,
    #[serde(default)]
    query_limits: QueryLimits,
    #[cfg(feature = "embed-static")]
    #[serde(default)]
    serve_static: bool,
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

// Make our own error that wraps `anyhow::Error`.
struct AppError(anyhow::Error);

// Tell axum how to convert `AppError` into a response.
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Something went wrong: {}", self.0),
        )
            .into_response()
    }
}

// This enables using `?` on functions that return `Result<_, anyhow::Error>` to turn them into
// `Result<_, AppError>`. That way you don't need to do that manually.
impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

async fn parse_or_resolve(resolver: &TokioAsyncResolver, name: String) -> anyhow::Result<IpNet> {
    if let Ok(net) = name.parse() {
        return Ok(net);
    }
    if let Ok(addr) = name.parse::<IpAddr>() {
        return Ok(addr.into());
    }

    Ok(resolver.lookup_ip(&name).await?.iter().next().ok_or(anyhow::anyhow!("Name resolution failure"))?.into())
}

async fn query<T: Store>(
    State((cfg, resolver, store)): State<(Arc<ApiServerConfig>, TokioAsyncResolver, T)>,
    AxumQuery(query): AxumQuery<Query<String>>,
) -> Result<impl IntoResponse, AppError> {
    trace!("request: {}", serde_json::to_string_pretty(&query).unwrap());

    let net_query = match query.net_query {
        NetQuery::Contains(name) => NetQuery::Contains(parse_or_resolve(&resolver, name).await?),
        NetQuery::MostSpecific(name) => NetQuery::MostSpecific(parse_or_resolve(&resolver, name).await?),
        NetQuery::Exact(name) => NetQuery::Exact(parse_or_resolve(&resolver, name).await?),
        NetQuery::OrLonger(name) => NetQuery::OrLonger(parse_or_resolve(&resolver, name).await?),
    };

    let mut query = Query {
        table_query: query.table_query,
        net_query,
        limits: query.limits,
        as_path_regex: query.as_path_regex
    };

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
    Ok(StreamBody::new(stream))
}

async fn routers<T: Store>(
    State((_, _, store)): State<(Arc<ApiServerConfig>, TokioAsyncResolver, T)>,
) -> impl IntoResponse {
    serde_json::to_string(&store.get_routers()).unwrap()
}

fn make_api<T: Store>(cfg: ApiServerConfig, store: T) -> anyhow::Result<Router> {
    let resolver = {
        let (rcfg, mut ropts) = hickory_resolver::system_conf::read_system_conf()?;
        ropts.ip_strategy = LookupIpStrategy::Ipv6thenIpv4; // strange people set strange default settings
        TokioAsyncResolver::tokio(rcfg, ropts)
    };
    Ok(Router::new()
        .route("/query", get(query::<T>))
        .route("/routers", get(routers::<T>))
        .with_state((Arc::new(cfg), resolver, store)))
}

/// This handler serializes the metrics into a string for Prometheus to scrape
pub async fn get_metrics() -> (StatusCode, String) {
    match autometrics::encode_global_metrics() {
        Ok(metrics) => (StatusCode::OK, metrics),
        Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{:?}", err)),
    }
}

#[cfg(feature = "embed-static")]
async fn static_path(axum::extract::Path(path): axum::extract::Path<String>) -> impl IntoResponse {
    use axum::body::Full;
    use axum::body::Empty;
    use axum::http::header;
    use axum::http::header::HeaderValue;

    let path = path.trim_start_matches('/');
    let mime_type = mime_guess::from_path(path).first_or_text_plain();

    match STATIC_DIR.get_file(path) {
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(axum::body::boxed(Empty::new()))
            .unwrap(),
        Some(file) => Response::builder()
            .status(StatusCode::OK)
            .header(
                header::CONTENT_TYPE,
                HeaderValue::from_str(mime_type.as_ref()).unwrap(),
            )
            .body(axum::body::boxed(Full::from(file.contents())))
            .unwrap(),
    }
}

pub async fn run_api_server<T: Store>(
    cfg: ApiServerConfig,
    store: T,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let mut router = Router::new();

    #[cfg(feature = "embed-static")]
    if cfg.serve_static {
        router = router.route("/*path", get(static_path))
    }

    router = router
        .nest("/api", make_api(cfg.clone(), store)?)
        .route("/metrics", get(get_metrics));

    let make_service = router
        .into_make_service();

    axum::Server::bind(&cfg.bind)
        .serve(make_service)
        .with_graceful_shutdown(shutdown.changed().map(|_| ()))
        .await?;

    Ok(())
}
