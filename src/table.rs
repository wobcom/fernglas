use std::pin::Pin;
use futures_util::Stream;
use std::net::{Ipv4Addr, IpAddr, SocketAddr};
use ipnet::IpNet;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub enum RouteOrigin {
    Igp,
    Egp,
    Incomplete,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RouteAttrs {
    pub origin: Option<RouteOrigin>,
    pub as_path: Option<Vec<u32>>,
    pub communities: Option<Vec<(u16, u16)>>,
    pub large_communities: Option<Vec<(u32, u32, u32)>>,
    pub med: Option<u32>,
    pub nexthop: Option<IpAddr>,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionId {
    pub from_client: SocketAddr,
    pub remote_router_id: Ipv4Addr,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "table")]
pub enum TableSelector {
    PrePolicyAdjIn(SessionId),
    PostPolicyAdjIn(SessionId),
    LocRib {
        from_client: SocketAddr,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TableQuery {
    Table(TableSelector),
    Session(SessionId),
    Router(SocketAddr),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum NetQuery {
    Contains(IpNet),
    MostSpecific(IpNet),
    Exact(IpNet),
    OrLonger(IpNet),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    #[serde(flatten)]
    pub table_query: Option<TableQuery>,
    #[serde(flatten)]
    pub net_query: NetQuery,
    #[serde(default)]
    pub as_path_regex: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QueryResult {
    pub net: IpNet,
    #[serde(flatten)]
    pub table: TableSelector,
    #[serde(flatten)]
    pub attrs: RouteAttrs,
}

#[async_trait]
pub trait Table: Clone + Send + Sync + 'static {
    async fn update_route(&self, net: IpNet, table: TableSelector, attrs: RouteAttrs);

    async fn withdraw_route(&self, net: IpNet, table: TableSelector);

    fn get_routes(&self, query: Query) -> Pin<Box<dyn Stream<Item = QueryResult> + Send>>;

    async fn clear_router_table(&self, router: SocketAddr);

    async fn clear_peer_table(&self, session: SessionId);
}
