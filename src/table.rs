use std::pin::Pin;
use futures_util::Stream;
use std::net::{IpAddr, SocketAddr};
use ipnet::{IpNet, Ipv4Net, Ipv6Net};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use log::*;

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
    pub local_pref: Option<u32>,
    pub nexthop: Option<IpAddr>,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionId {
    pub from_client: SocketAddr,
    pub peer_address: IpAddr,
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

impl TableSelector {
    pub fn client_addr(&self) -> &SocketAddr {
        match self {
            TableSelector::LocRib { from_client } => from_client,
            TableSelector::PostPolicyAdjIn(session) => &session.from_client,
            TableSelector::PrePolicyAdjIn(session) => &session.from_client,
        }
    }
    pub fn session_id(&self) -> Option<&SessionId> {
        match self {
            TableSelector::LocRib { .. } => None,
            TableSelector::PostPolicyAdjIn(session) => Some(session),
            TableSelector::PrePolicyAdjIn(session) => Some(session),
        }
    }
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
    pub limits: Option<QueryLimits>,
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
    pub client: Client,
    #[serde(flatten)]
    pub session: Option<Session>,
    #[serde(flatten)]
    pub attrs: RouteAttrs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryLimits {
    pub max_results_per_table: usize,
    pub max_results: usize,
}

/// information saved about a connected router
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Client {
    pub client_name: String,
}

/// information saved about a connected peer
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Session {
}

impl Default for QueryLimits {
    fn default() -> Self {
        Self {
            max_results_per_table: 200,
            max_results: 500
        }
    }
}

#[async_trait]
pub trait Table: Clone + Send + Sync + 'static {
    async fn update_route(&self, net: IpNet, table: TableSelector, attrs: RouteAttrs);

    async fn withdraw_route(&self, net: IpNet, table: TableSelector);

    fn get_routes(&self, query: Query) -> Pin<Box<dyn Stream<Item = QueryResult> + Send>>;

    async fn client_up(&self, client_addr: SocketAddr, client_data: Client);

    async fn client_down(&self, client_addr: SocketAddr);

    async fn session_up(&self, session: SessionId, session_data: Session);

    async fn session_down(&self, session: SessionId, new_state: Option<Session>);

    async fn insert_bgp_update(&self, session: TableSelector, update: zettabgp::prelude::BgpUpdateMessage) {
        use zettabgp::prelude::*;
        let mut attrs: RouteAttrs = Default::default();
        let mut nexthop = None;
        let mut update_nets = vec![];
        let mut withdraw_nets = vec![];
        for attr in update.attrs {
            match attr {
                BgpAttrItem::MPUpdates(updates) => {
                    let nexthop = match updates.nexthop {
                        BgpAddr::V4(v4) => Some(IpAddr::from(v4)),
                        BgpAddr::V6(v6) => Some(IpAddr::from(v6)),
                        _ => None,
                    };
                    for net in bgp_addrs_to_nets(&updates.addrs) {
                        update_nets.push((net, nexthop));
                    }
                }
                BgpAttrItem::MPWithdraws(withdraws) => {
                    for net in bgp_addrs_to_nets(&withdraws.addrs) {
                        withdraw_nets.push(net);
                    }
                }
                BgpAttrItem::NextHop(BgpNextHop { value }) => {
                    nexthop = Some(value);
                }
                BgpAttrItem::CommunityList(BgpCommunityList { value }) => {
                    let mut communities = vec![];
                    for community in value.into_iter() {
                        communities.push(((community.value >> 16) as u16, (community.value & 0xff) as u16));
                    }
                    attrs.communities = Some(communities);
                }
                BgpAttrItem::MED(BgpMED { value }) => {
                    attrs.med = Some(value);
                }
                BgpAttrItem::LocalPref(BgpLocalpref { value }) => {
                    attrs.local_pref = Some(value);
                }
                BgpAttrItem::Origin(BgpOrigin { value }) => {
                    attrs.origin = Some(match value {
                        BgpAttrOrigin::Igp => RouteOrigin::Igp,
                        BgpAttrOrigin::Egp => RouteOrigin::Egp,
                        BgpAttrOrigin::Incomplete => RouteOrigin::Incomplete,
                    })
                }
                BgpAttrItem::ASPath(BgpASpath { value }) => {
                    let mut as_path = vec![];
                    for asn in value {
                        as_path.push(asn.value);
                    }
                    attrs.as_path = Some(as_path);
                }
                BgpAttrItem::LargeCommunityList(BgpLargeCommunityList { value }) => {
                    let mut communities = vec![];
                    for community in value.into_iter() {
                        communities.push((community.ga, community.ldp1, community.ldp2));
                    }
                    attrs.large_communities = Some(communities);
                }
                _ => {},
            }
        }
        for net in bgp_addrs_to_nets(&update.updates).into_iter() {
            update_nets.push((net, nexthop));
        }
        for net in bgp_addrs_to_nets(&update.withdraws).into_iter() {
            withdraw_nets.push(net);
        }

        for (net, nexthop) in update_nets {
            let mut attrs = attrs.clone();
            attrs.nexthop = nexthop;
            self.update_route(net, session.clone(), attrs).await;
        }
        for net in withdraw_nets {
            self.withdraw_route(net, session.clone()).await;
        }
    }
}
fn bgp_addrs_to_nets(addrs: &zettabgp::prelude::BgpAddrs) -> Vec<IpNet> {
    use zettabgp::prelude::*;
    let mut res = vec![];
    match addrs {
        BgpAddrs::IPV4U(ref addrs) => {
            for addr in addrs {
                match Ipv4Net::new(addr.addr, addr.prefixlen) {
                    Ok(net) => res.push(IpNet::V4(net)),
                    Err(_) => warn!("invalid BgpAddrs prefixlen"),
                }
            }
        }
        BgpAddrs::IPV6U(ref addrs) => {
            for addr in addrs {
                res.push(IpNet::V6(Ipv6Net::new(addr.addr, addr.prefixlen).unwrap()));
            }
        }
        _ => {}
    }
    res
}
