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
pub struct SessionId {
    pub local_router_id: Ipv4Addr,
    pub remote_router_id: Ipv4Addr,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Serialize, Deserialize)]
pub enum TableSelector {
    PrePolicyAdjIn(SessionId),
    PostPolicyAdjIn(SessionId),
    LocRib(Ipv4Addr),
}

#[derive(Debug, Clone, Deserialize)]
pub enum NetQuery {
    AsPathRegex(String),
    Contains(IpAddr),
    ContainsMostSpecific(IpAddr),
    Exact(IpNet),
    OrLonger(IpNet),
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Query {
    pub router_id: Option<IpAddr>,
    pub table: Option<TableSelector>,
    pub net: Option<NetQuery>,
}

#[async_trait]
pub trait Table {
    async fn update_route(&self, net: IpNet, table: TableSelector, route: Route);

    async fn withdraw_route(&self, net: IpNet, table: TableSelector);

    fn get_routes(&self, query: Query) -> Pin<Box<dyn Stream<Item = (TableSelector, IpNet, Route)> + Send>>;

    async fn clear_router_table(&self, router: Ipv4Addr);

    async fn clear_peer_table(&self, session: SessionId);
}
