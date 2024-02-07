use crate::store::{NetQuery, Query, QueryLimits, QueryResult, Store};
use axum::body::Body;
use axum::extract::FromRef;
use axum::extract::{Query as AxumQuery, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures_util::{FutureExt, StreamExt};
use hickory_resolver::config::LookupIpStrategy;
use hickory_resolver::TokioAsyncResolver;
use ipnet::IpNet;
use log::*;
use regex::Regex;
use regex::RegexSet;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::collections::HashSet;
use std::convert::Infallible;
use std::net::IpAddr;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

#[cfg(feature = "embed-static")]
static STATIC_DIR: include_dir::Dir<'_> = include_dir::include_dir!("$CARGO_MANIFEST_DIR/static");

static COMMUNITIES_LIST: &[u8] = include_bytes!("communities.json");

fn default_asn_dns_zone() -> Option<String> {
    Some("as{}.asn.cymru.com.".to_string())
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiServerConfig {
    bind: SocketAddr,
    #[serde(default)]
    query_limits: QueryLimits,
    #[cfg(feature = "embed-static")]
    #[serde(default)]
    serve_static: bool,
    /// Dns zone used for ASN lookups
    #[serde(default = "default_asn_dns_zone")]
    pub asn_dns_zone: Option<String>,
    /// Path to alternative communities.json
    communities_file: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub enum ApiResult {
    Route(QueryResult),
    ReverseDns {
        nexthop: IpAddr,
        nexthop_resolved: String,
    },
    AsnName {
        asn: u32,
        asn_name: String,
    },
    CommunityDescription {
        community: String,
        community_description: String,
    },
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

#[derive(Clone)]
struct AppState<T: Clone> {
    cfg: Arc<ApiServerConfig>,
    resolver: TokioAsyncResolver,
    community_lists: Arc<CompiledCommunitiesLists>,
    store: T,
}

impl<T: Clone> FromRef<AppState<T>> for Arc<ApiServerConfig> {
    fn from_ref(app_state: &AppState<T>) -> Self {
        app_state.cfg.clone()
    }
}

impl<T: Clone> FromRef<AppState<T>> for TokioAsyncResolver {
    fn from_ref(app_state: &AppState<T>) -> Self {
        app_state.resolver.clone()
    }
}

impl<T: Clone> FromRef<AppState<T>> for Arc<CompiledCommunitiesLists> {
    fn from_ref(app_state: &AppState<T>) -> Self {
        app_state.community_lists.clone()
    }
}

async fn parse_or_resolve(resolver: &TokioAsyncResolver, name: String) -> anyhow::Result<IpNet> {
    if let Ok(net) = name.parse() {
        return Ok(net);
    }
    if let Ok(addr) = name.parse::<IpAddr>() {
        return Ok(addr.into());
    }

    Ok(resolver
        .lookup_ip(&format!("{}.", name))
        .await?
        .iter()
        .next()
        .ok_or(anyhow::anyhow!("Name resolution failure"))?
        .into())
}

#[derive(Deserialize)]
struct CommunitiesLists {
    regular: CommunitiesList,
    large: CommunitiesList,
}
impl CommunitiesLists {
    fn compile(self) -> anyhow::Result<CompiledCommunitiesLists> {
        Ok(CompiledCommunitiesLists {
            regular: self.regular.compile()?,
            large: self.large.compile()?,
        })
    }
}

struct CompiledCommunitiesLists {
    regular: CompiledCommunitiesList,
    large: CompiledCommunitiesList,
}

#[derive(Deserialize)]
struct CommunitiesList(HashMap<String, String>);

impl CommunitiesList {
    fn compile(self) -> anyhow::Result<CompiledCommunitiesList> {
        let mut sorted = self.0.into_iter().collect::<Vec<_>>();
        sorted.sort_by(|a, b| a.0.len().cmp(&b.0.len()));
        Ok(CompiledCommunitiesList {
            regex_set: RegexSet::new(sorted.iter().map(|(regex, _desc)| format!("^{}$", regex)))?,
            list: sorted
                .into_iter()
                .map(|(key, value)| Ok((Regex::new(&format!("^{}$", key))?, value)))
                .collect::<anyhow::Result<_>>()?,
        })
    }
}

struct CompiledCommunitiesList {
    regex_set: RegexSet,
    list: Vec<(Regex, String)>,
}
impl CompiledCommunitiesList {
    fn lookup(&self, community: &str) -> Option<Cow<str>> {
        self.regex_set
            .matches(community)
            .iter()
            .next()
            .map(|index| {
                let (regex, desc) = &self.list[index];
                let mut desc_templated: Cow<str> = desc.into();
                for (i, subcapture) in regex
                    .captures(community)
                    .unwrap()
                    .iter()
                    .skip(1)
                    .enumerate()
                {
                    if let Some(subcapture) = subcapture {
                        let searchstr = format!("${}", i);
                        if desc_templated.contains(&searchstr) {
                            desc_templated =
                                desc_templated.replace(&searchstr, subcapture.into()).into()
                        }
                    }
                }
                desc_templated
            })
    }
}

async fn query<T: Store>(
    State(AppState {
        cfg,
        resolver,
        store,
        community_lists,
    }): State<AppState<T>>,
    AxumQuery(query): AxumQuery<Query<String>>,
) -> Result<impl IntoResponse, AppError> {
    trace!("request: {}", serde_json::to_string_pretty(&query).unwrap());

    let net_query = match query.net_query {
        NetQuery::Contains(name) => NetQuery::Contains(parse_or_resolve(&resolver, name).await?),
        NetQuery::MostSpecific(name) => {
            NetQuery::MostSpecific(parse_or_resolve(&resolver, name).await?)
        }
        NetQuery::Exact(name) => NetQuery::Exact(parse_or_resolve(&resolver, name).await?),
        NetQuery::OrLonger(name) => NetQuery::OrLonger(parse_or_resolve(&resolver, name).await?),
    };

    let mut query = Query {
        table_query: query.table_query,
        net_query,
        limits: query.limits,
        as_path_regex: query.as_path_regex,
    };

    let mut limits = query.limits.take().unwrap_or(cfg.query_limits.clone());
    limits.max_results = std::cmp::min(limits.max_results, cfg.query_limits.max_results);
    limits.max_results_per_table = std::cmp::min(
        limits.max_results_per_table,
        cfg.query_limits.max_results_per_table,
    );
    query.limits = Some(limits);

    // for deduplicating the nexthop resolutions
    let mut have_resolved = HashSet::new();
    let mut have_asn = HashSet::new();
    let mut have_community = HashSet::new();
    let mut have_large_community = HashSet::new();

    let stream = store
        .get_routes(query)
        .flat_map_unordered(None, move |route| {
            let futures = futures_util::stream::FuturesUnordered::<
                Pin<Box<dyn std::future::Future<Output = Option<ApiResult>> + Send>>,
            >::new();

            futures.push(Box::pin(futures_util::future::ready(Some(
                ApiResult::Route(route.clone()),
            ))));

            if let Some(nexthop) = route.attrs.nexthop {
                if have_resolved.insert(nexthop) {
                    let resolver = resolver.clone();
                    futures.push(Box::pin(async move {
                        resolver
                            .reverse_lookup(nexthop)
                            .await
                            .ok()
                            .and_then(|reverse| reverse.iter().next().map(|x| x.0.to_string()))
                            .map(|nexthop_resolved| ApiResult::ReverseDns {
                                nexthop,
                                nexthop_resolved,
                            })
                    }))
                }
            }
            if let Some(asn_dns_zone) = &cfg.asn_dns_zone {
                for asn in route.attrs.as_path.into_iter().flat_map(|x| x) {
                    if have_asn.insert(asn) {
                        let resolver = resolver.clone();
                        let asn_dns_zone = asn_dns_zone.clone();
                        futures.push(Box::pin(async move {
                            resolver
                                .txt_lookup(asn_dns_zone.replace("{}", &asn.to_string()))
                                .await
                                .ok()
                                .and_then(|txt| {
                                    txt.iter().next().and_then(|x| {
                                        x.iter()
                                            .next()
                                            .and_then(|data| std::str::from_utf8(data).ok())
                                            .and_then(|s| {
                                                s.split(" | ")
                                                    .skip(4)
                                                    .next()
                                                    .map(|name| name.to_string())
                                            })
                                    })
                                })
                                .map(|asn_name| ApiResult::AsnName { asn, asn_name })
                        }))
                    }
                }
            }
            for community in route.attrs.communities.into_iter().flat_map(|x| x) {
                if have_community.insert(community) {
                    let community_str = format!("{}:{}", community.0, community.1);
                    if let Some(lookup) = community_lists.regular.lookup(&community_str) {
                        futures.push(Box::pin(futures_util::future::ready(Some(
                            ApiResult::CommunityDescription {
                                community: community_str,
                                community_description: lookup.to_string(),
                            },
                        ))));
                    }
                }
            }
            for large_community in route.attrs.large_communities.into_iter().flat_map(|x| x) {
                if have_large_community.insert(large_community) {
                    let large_community_str = format!(
                        "{}:{}:{}",
                        large_community.0, large_community.1, large_community.2
                    );
                    if let Some(lookup) = community_lists.large.lookup(&large_community_str) {
                        futures.push(Box::pin(futures_util::future::ready(Some(
                            ApiResult::CommunityDescription {
                                community: large_community_str,
                                community_description: lookup.to_string(),
                            },
                        ))));
                    }
                }
            }

            futures
        })
        .filter_map(|x| futures_util::future::ready(x))
        .map(|result| {
            let json = serde_json::to_string(&result).unwrap();
            Ok::<_, Infallible>(format!("{}\n", json))
        });

    Ok(Body::from_stream(stream))
}

async fn routers<T: Store>(State(AppState { store, .. }): State<AppState<T>>) -> impl IntoResponse {
    serde_json::to_string(&store.get_routers()).unwrap()
}

async fn make_api<T: Store>(cfg: ApiServerConfig, store: T) -> anyhow::Result<Router> {
    let resolver = {
        let (rcfg, mut ropts) = hickory_resolver::system_conf::read_system_conf()?;
        ropts.ip_strategy = LookupIpStrategy::Ipv6thenIpv4; // strange people set strange default settings
        TokioAsyncResolver::tokio(rcfg, ropts)
    };

    let community_lists: CommunitiesLists = if let Some(ref path) = cfg.communities_file {
        let path = path.clone();
        serde_json::from_slice(&tokio::task::spawn_blocking(move || std::fs::read(path)).await??)?
    } else {
        serde_json::from_slice(COMMUNITIES_LIST)?
    };

    let community_lists = Arc::new(community_lists.compile()?);

    Ok(Router::new()
        .route("/query", get(query::<T>))
        .route("/routers", get(routers::<T>))
        .with_state(AppState {
            cfg: Arc::new(cfg),
            resolver,
            store,
            community_lists,
        }))
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
    use axum::http::header;
    use axum::http::header::HeaderValue;

    let path = path.trim_start_matches('/');
    let mime_type = mime_guess::from_path(path).first_or_text_plain();

    match STATIC_DIR.get_file(path) {
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap(),
        Some(file) => Response::builder()
            .status(StatusCode::OK)
            .header(
                header::CONTENT_TYPE,
                HeaderValue::from_str(mime_type.as_ref()).unwrap(),
            )
            .body(Body::from(file.contents()))
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
        .nest("/api", make_api(cfg.clone(), store).await?)
        .route("/metrics", get(get_metrics));

    let make_service = router.into_make_service();

    // run our app with hyper, listening globally on port 3000
    let listener = tokio::net::TcpListener::bind(&cfg.bind).await?;
    axum::serve(listener, make_service)
        .with_graceful_shutdown(async move { shutdown.changed().map(|_| ()).await })
        .await?;

    Ok(())
}
