use std::sync::Mutex;
use regex::Regex;
use std::pin::Pin;
use futures_util::Stream;
use std::net::{Ipv4Addr, IpAddr};
use std::sync::Arc;
use ipnet::IpNet;
use std::collections::HashMap;
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

#[derive(Default, Clone)]
pub struct InMemoryTable {
    pre_policy_adj_in: Arc<Mutex<HashMap<SessionId, Arc<Mutex<HashMap<IpNet, Route>>>>>>,
    post_policy_adj_in: Arc<Mutex<HashMap<SessionId, Arc<Mutex<HashMap<IpNet, Route>>>>>>,
    loc_rib: Arc<Mutex<HashMap<Ipv4Addr, Arc<Mutex<HashMap<IpNet, Route>>>>>>,
}

impl InMemoryTable {
    fn get_table(&self, sel: TableSelector) -> Arc<Mutex<HashMap<IpNet, Route>>> {
        match sel {
            TableSelector::PrePolicyAdjIn(session) => {
                self.pre_policy_adj_in.lock().unwrap().entry(session).or_insert(Default::default()).clone()
            }
            TableSelector::PostPolicyAdjIn(session) => {
                self.post_policy_adj_in.lock().unwrap().entry(session).or_insert(Default::default()).clone()
            }
            TableSelector::LocRib(router_id) => {
                self.loc_rib.lock().unwrap().entry(router_id).or_insert(Default::default()).clone()
            }
        }
    }
}

#[async_trait]
impl Table for InMemoryTable {
    async fn update_route(&self, net: IpNet, table: TableSelector, route: Route) {
        let table = self.get_table(table);
        let mut table = table.lock().unwrap();
        table.insert(net, route);
    }

    async fn withdraw_route(&self, net: IpNet, table: TableSelector) {
        let table = self.get_table(table);
        let mut table = table.lock().unwrap();
        table.remove(&net);
    }

    fn get_routes(&self, query: Query) -> Pin<Box<dyn Stream<Item = (TableSelector, IpNet, Route)> + Send>> {
        let tables_filter_fn: Box<dyn FnMut(&(TableSelector, Arc<Mutex<HashMap<IpNet, Route>>>)) -> bool> = if let Some(table) = query.table {
            Box::new(move |(k, _)| *k == table)
        } else if let Some(router_id) = query.router_id {
            Box::new(move |(k, _)| match k {
                TableSelector::LocRib(r) => *r == router_id,
                TableSelector::PostPolicyAdjIn(session) => session.local_router_id == router_id,
                TableSelector::PrePolicyAdjIn(session) => session.local_router_id == router_id,
            })
        } else {
            Box::new(|_| true)
        };
        let tables = {
            let pre_policy_adj_in = self.pre_policy_adj_in.lock().unwrap();
            let post_policy_adj_in = self.post_policy_adj_in.lock().unwrap();
            let loc_rib = self.loc_rib.lock().unwrap();

            loc_rib.iter().map(|(k, v)| (TableSelector::LocRib(k.clone()), v.clone()))
                .chain(post_policy_adj_in.iter().map(|(k, v)| (TableSelector::PostPolicyAdjIn(k.clone()), v.clone())))
                .chain(pre_policy_adj_in.iter().map(|(k, v)| (TableSelector::PrePolicyAdjIn(k.clone()), v.clone())))
                .filter(tables_filter_fn)
                .collect::<Vec<_>>()
        };

        let nets_filter_fn: Box<dyn Fn(&(TableSelector, IpNet, Route)) -> bool + Send> = match query.net {
            Some(NetQuery::AsPathRegex(ref as_path_regex)) => {
                let regex = Regex::new(as_path_regex).unwrap(); // FIXME error handling
                Box::new(move |(_, _, route)| {
                    let as_path_text = match &route.as_path {
                        Some(as_path) => as_path.iter().map(|asn| asn.to_string()).collect::<Vec<_>>().join(" "),
                        None => return false,
                    };
                    regex.is_match(&as_path_text)
                })
            }
            Some(NetQuery::Contains(addr)) => {
                Box::new(move |(_, net, _)| net.contains(&addr))
            },
            Some(NetQuery::ContainsMostSpecific(addr)) => {
                todo!()
            },
            Some(NetQuery::OrLonger(net)) => {
                todo!()
            },
            None | Some(NetQuery::Exact(_)) => {
                Box::new(|_| true)
            }
        };

        let nets_iter: Box<dyn Iterator<Item = (TableSelector, IpNet, Route)> + Send> = match query.net {
            Some(NetQuery::Exact(net)) => {
                Box::new(tables.into_iter().filter_map(move |(table_sel, table)| {
                    let table = table.lock().unwrap();
                    table.get(&net)
                        .map(|has_route| (table_sel.clone(), net.clone(), has_route.clone()))
                })
                        .filter(nets_filter_fn)
                         )
            },
            _ => {
                Box::new(tables.into_iter().flat_map(move |(table_sel, table)| {
                    let table = table.lock().unwrap();
                    table.iter()
                        .map(move |(net, route)| (table_sel.clone(), net.clone(), route.clone()))
                        .filter(&nets_filter_fn)
                        .take(200)
                        .collect::<Vec<_>>()
                        .into_iter()
                }))
            }
        };

        Box::pin(futures_util::stream::iter(
            nets_iter
                .take(200)
        ))
    }

    async fn clear_router_table(&self, router: Ipv4Addr) {
        self.loc_rib.lock().unwrap().remove(&router);
        self.pre_policy_adj_in.lock().unwrap().retain(|k, _| k.local_router_id != router);
        self.post_policy_adj_in.lock().unwrap().retain(|k, _| k.local_router_id != router);
    }

    async fn clear_peer_table(&self, session: SessionId) {
        self.pre_policy_adj_in.lock().unwrap().remove(&session);
        self.post_policy_adj_in.lock().unwrap().remove(&session);
    }
}
