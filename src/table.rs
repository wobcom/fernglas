use std::pin::Pin;
use futures_util::Stream;
use std::net::{Ipv4Addr, IpAddr};
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
pub struct Route {
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
    pub local_router_id: Ipv4Addr,
    pub remote_router_id: Ipv4Addr,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "table")]
pub enum TableSelector {
    PrePolicyAdjIn(SessionId),
    PostPolicyAdjIn(SessionId),
    LocRib {
        locrib_router_id: Ipv4Addr,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TableQuery {
    Table(TableSelector),
    Session(SessionId),
    Router { router_id: Ipv4Addr },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum NetQuery {
    Contains(IpAddr),
    ContainsMostSpecific(IpAddr),
    Exact(IpNet),
    OrLonger(IpNet),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    #[serde(flatten)]
    pub table_query: Option<TableQuery>,
    #[serde(flatten)]
    pub net_query: Option<NetQuery>,
    #[serde(default)]
    pub as_path_regex: Option<String>,
}

#[async_trait]
pub trait Table: Clone + Send + Sync + 'static {
    async fn update_route(&self, net: IpNet, table: TableSelector, route: Route);

    async fn withdraw_route(&self, net: IpNet, table: TableSelector);

    fn get_routes(&self, query: Query) -> Pin<Box<dyn Stream<Item = (TableSelector, IpNet, Route)> + Send>>;

    async fn clear_router_table(&self, router: Ipv4Addr);

    async fn clear_peer_table(&self, session: SessionId);
}
