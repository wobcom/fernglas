use std::sync::Mutex;
use regex::Regex;
use std::pin::Pin;
use futures_util::Stream;
use futures_util::StreamExt;
use std::net::Ipv4Addr;
use std::sync::Arc;
use ipnet::IpNet;
use std::collections::HashMap;
use async_trait::async_trait;
use tokio_stream::wrappers::ReceiverStream;
use rayon::iter::ParallelIterator;
use rayon::iter::IntoParallelIterator;
use bitvec::prelude::Msb0;
use bitvec::view::BitView;
use radix_trie::{Trie, TrieKey, TrieCommon};

use crate::table::{Route, Query, SessionId, TableSelector, Table, NetQuery, TableQuery};

#[derive(PartialEq, Eq)]
struct IpNetKey(IpNet);

impl TrieKey for IpNetKey {
    fn encode_bytes(&self) -> Vec<u8> {
        let (ip, prefixlen) = match self.0 {
            IpNet::V4(v4) => (v4.addr().to_ipv6_mapped(), v4.prefix_len() + 96),
            IpNet::V6(v6) => (v6.addr(), v6.prefix_len()),
        };
        ip.octets()[..].view_bits::<Msb0>()[..(prefixlen as usize)].iter().map(|x| (*x.as_ref()).into()).collect()
    }
}

type RouteMap = Arc<Mutex<Trie<IpNetKey, Route>>>;

#[derive(Default, Clone)]
pub struct InMemoryTable {
    tables: Arc<Mutex<HashMap<TableSelector, RouteMap>>>,
}

fn tables_for_router_fn(router_id: Ipv4Addr) -> impl Fn(&(&TableSelector, &RouteMap)) -> bool {
    move |(k, _): &(_, _)| {
        match &k {
            TableSelector::LocRib { locrib_router_id } => *locrib_router_id == router_id,
            TableSelector::PostPolicyAdjIn(session) => session.local_router_id == router_id,
            TableSelector::PrePolicyAdjIn(session) => session.local_router_id == router_id,
        }
    }
}
fn tables_for_session_fn(session_id: SessionId) -> impl Fn(&(&TableSelector, &RouteMap)) -> bool {
    move |(k, _): &(_, _)| {
        match &k {
            TableSelector::LocRib { .. } => false,
            TableSelector::PostPolicyAdjIn(session) => *session == session_id,
            TableSelector::PrePolicyAdjIn(session) => *session == session_id,
        }
    }
}

impl InMemoryTable {
    fn get_table(&self, sel: TableSelector) -> RouteMap {
        self.tables.lock().unwrap().entry(sel).or_insert(Default::default()).clone()
    }
    fn get_tables_for_router(&self, router_id: Ipv4Addr) -> Vec<(TableSelector, RouteMap)> {
        self.tables.lock().unwrap()
            .iter()
            .filter(tables_for_router_fn(router_id))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
    fn get_tables_for_session(&self, session_id: SessionId) -> Vec<(TableSelector, RouteMap)> {
        self.tables.lock().unwrap()
            .iter()
            .filter(tables_for_session_fn(session_id))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

#[async_trait]
impl Table for InMemoryTable {
    async fn update_route(&self, net: IpNet, table: TableSelector, route: Route) {
        let table = self.get_table(table);
        let mut table = table.lock().unwrap();
        table.insert(IpNetKey(net), route);
    }

    async fn withdraw_route(&self, net: IpNet, table: TableSelector) {
        let table = self.get_table(table);
        let mut table = table.lock().unwrap();
        table.remove(&IpNetKey(net));
    }

    fn get_routes(&self, query: Query) -> Pin<Box<dyn Stream<Item = (TableSelector, IpNet, Route)> + Send>> {

        let tables = match query.table_query {
            Some(TableQuery::Table(table)) => vec![(table.clone(), self.get_table(table))],
            Some(TableQuery::Router { router_id }) => self.get_tables_for_router(router_id),
            Some(TableQuery::Session(session_id)) => self.get_tables_for_session(session_id),
            None => self.tables.lock().unwrap().clone().into_iter().collect(),
        };

        let mut nets_filter_fn: Box<dyn Fn(&(TableSelector, IpNet, Route)) -> bool + Send + Sync> = Box::new(|_| true);

        if let Some(as_path_regex) = query.as_path_regex {
            let regex = Regex::new(&as_path_regex).unwrap(); // FIXME error handling
            let new_filter_fn = move |(_, _, route): &(TableSelector, IpNet, Route)| {
                let as_path_text = match &route.as_path {
                    Some(as_path) => as_path.iter().map(|asn| asn.to_string()).collect::<Vec<_>>().join(" "),
                    None => return false,
                };
                regex.is_match(&as_path_text)
            };
            nets_filter_fn = Box::new(move |i| nets_filter_fn(i) && new_filter_fn(i))
        };

        let (tx, rx) = tokio::sync::mpsc::channel(2);

        rayon::spawn(move || {
            match query.net_query {
                Some(NetQuery::Exact(net)) => {
                    tables.into_par_iter().filter_map(move |(table_sel, table)| {
                        let table = table.lock().unwrap();
                        table.get(&IpNetKey(net))
                            .map(|has_route| (table_sel.clone(), net.clone(), has_route.clone()))
                    })
                    .filter(nets_filter_fn)
                    .for_each_with(tx, |tx, res| drop(tx.blocking_send(res)));
                },
                Some(NetQuery::Contains(net)) => {
                    tables.into_par_iter().flat_map(move |(table_sel, table)| {
                        let table = table.lock().unwrap();

                        let mut next_net = Some(net);
                        let mut nets = vec![];
                        while let Some(net) = next_net.take() {
                            nets.push(net);
                            next_net = net.supernet();
                        }
                        nets.into_iter().filter_map(|net| {
                            table.subtrie(&IpNetKey(net))
                                .and_then(|has_route| {
                                    match (has_route.key(), has_route.value()) {
                                        (Some(k), Some(v)) => Some((table_sel.clone(), k.0.clone(), v.clone())),
                                        _ => None,
                                    }
                                })
                        })
                        .filter(&nets_filter_fn)
                        .take(200)
                        .collect::<Vec<_>>()
                        .into_par_iter()
                    })
                    .for_each_with(tx, |tx, res| drop(tx.blocking_send(res)));
                },
                Some(NetQuery::ContainsMostSpecific(net)) => {
                    tables.into_par_iter().filter_map(move |(table_sel, table)| {
                        let table = table.lock().unwrap();
                        table.get_ancestor(&IpNetKey(net))
                            .and_then(|has_route| {
                                match (has_route.key(), has_route.value()) {
                                    (Some(k), Some(v)) => Some((table_sel.clone(), k.0.clone(), v.clone())),
                                    _ => None,
                                }
                            })
                    })
                    .filter(nets_filter_fn)
                    .for_each_with(tx, |tx, res| drop(tx.blocking_send(res)));
                },
                Some(_) => todo!(),
                None => {
                    tables.into_par_iter().flat_map(move |(table_sel, table)| {
                        let table = table.lock().unwrap();
                        table.iter()
                            .map(move |(net, route)| (table_sel.clone(), net.0.clone(), route.clone()))
                            .filter(&nets_filter_fn)
                            .take(200)
                            .collect::<Vec<_>>()
                            .into_par_iter()
                    })
                    .for_each_with(tx, |tx, res| drop(tx.blocking_send(res)));

                }
            };
        });

        Box::pin(ReceiverStream::new(rx).take(500))
    }

    async fn clear_router_table(&self, router: Ipv4Addr) {
        self.tables.lock().unwrap().retain(|k, v| !(tables_for_router_fn(router)(&(k, v))));
    }

    async fn clear_peer_table(&self, session: SessionId) {
        self.tables.lock().unwrap().retain(|k, v| !(tables_for_session_fn(session.clone())(&(k, v))));
    }
}
