use std::sync::Mutex;
use regex::Regex;
use std::pin::Pin;
use futures_util::Stream;
use futures_util::StreamExt;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use ipnet::{IpNet, Ipv4Net, Ipv6Net};
use std::collections::HashMap;
use async_trait::async_trait;
use tokio_stream::wrappers::ReceiverStream;
use rayon::iter::ParallelIterator;
use rayon::iter::IntoParallelIterator;
use bitvec::prelude::Msb0;
use bitvec::view::BitView;
use patricia_tree::PatriciaMap;
use log::*;

use crate::table::*;
use crate::compressed_attrs::*;


macro_rules! encode_net {
    ($input:ident, $identifier:expr) => {
        {
            let mut key = vec![$identifier];
            key.extend($input.addr().octets()[..].view_bits::<Msb0>()[..($input.prefix_len() as usize)].iter().map(|x| u8::from(*x.as_ref())));
            key
        }
    }
}
macro_rules! decode_net {
    ($input:ident, $bytes:expr, $net_variant:ident, $net_type:ty, $addr_type:ty) => {
        {
            let mut addr = [0u8; $bytes];
            let addr_view = addr.view_bits_mut::<Msb0>();
            for (i, bit) in $input[1..].iter().enumerate() {
                *addr_view.get_mut(i).unwrap() = *bit != 0;
            }
            IpNet::$net_variant(<$net_type>::new(<$addr_type>::from(addr), ($input.len() - 1) as u8).unwrap())
        }
    }
}

fn to_key(net: &IpNet) -> Vec<u8> {
    match net {
        IpNet::V4(net) => encode_net!(net, 4),
        IpNet::V6(net) => encode_net!(net, 6),
    }
}

fn from_key(key: &[u8]) -> IpNet {
    match key[0] {
        4 => decode_net!(key, 4, V4, Ipv4Net, Ipv4Addr),
        6 => decode_net!(key, 16, V6, Ipv6Net, Ipv6Addr),
        _ => panic!("invalid key encoding"),
    }
}

type RouteMap = Arc<Mutex<PatriciaMap<Arc<CompressedRouteAttrs>>>>;

#[derive(Default, Clone)]
pub struct InMemoryTable {
    clients: Arc<Mutex<HashMap<SocketAddr, Client>>>,
    sessions: Arc<Mutex<HashMap<SessionId, Session>>>,
    tables: Arc<Mutex<HashMap<TableSelector, RouteMap>>>,

    caches: Arc<Mutex<Caches>>,
}

fn tables_for_client_fn(query_from_client: &SocketAddr) -> impl Fn(&(&TableSelector, &RouteMap)) -> bool + '_ {
    move |(k, _): &(_, _)| {
        k.client_addr() == query_from_client
    }
}
fn tables_for_session_fn(session_id: &SessionId) -> impl Fn(&(&TableSelector, &RouteMap)) -> bool + '_ {
    move |(k, _): &(_, _)| {
        k.session_id() == Some(session_id)
    }
}
impl InMemoryTable {
    fn get_table(&self, sel: TableSelector) -> RouteMap {
        self.tables.lock().unwrap().entry(sel).or_insert(Default::default()).clone()
    }
    fn get_tables_for_client(&self, client_addr: &SocketAddr) -> Vec<(TableSelector, RouteMap)> {
        self.tables.lock().unwrap()
            .iter()
            .filter(tables_for_client_fn(client_addr))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
    fn get_tables_for_session(&self, session_id: &SessionId) -> Vec<(TableSelector, RouteMap)> {
        self.tables.lock().unwrap()
            .iter()
            .filter(tables_for_session_fn(session_id))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

#[async_trait]
impl Table for InMemoryTable {
    async fn update_route(&self, net: IpNet, table: TableSelector, route: RouteAttrs) {
        let compressed = self.caches.lock().unwrap().compress_route_attrs(route);
        let table = self.get_table(table);
        let mut table = table.lock().unwrap();
        table.insert(to_key(&net), compressed);
    }

    async fn withdraw_route(&self, net: IpNet, table: TableSelector) {
        let table = self.get_table(table);
        let mut table = table.lock().unwrap();
        table.remove(to_key(&net));
    }

    fn get_routes(&self, query: Query) -> Pin<Box<dyn Stream<Item = QueryResult> + Send>> {

        let tables = match query.table_query {
            Some(TableQuery::Table(table)) => vec![(table.clone(), self.get_table(table))],
            Some(TableQuery::Router(client_addr)) => self.get_tables_for_client(&client_addr),
            Some(TableQuery::Session(session_id)) => self.get_tables_for_session(&session_id),
            None => self.tables.lock().unwrap().clone().into_iter().collect(),
        };

        let mut nets_filter_fn: Box<dyn Fn(&(TableSelector, IpNet, Arc<CompressedRouteAttrs>)) -> bool + Send + Sync> = Box::new(|_| true);

        if let Some(as_path_regex) = query.as_path_regex {
            let regex = Regex::new(&as_path_regex).unwrap(); // FIXME error handling
            let new_filter_fn = move |(_, _, route): &(TableSelector, IpNet, Arc<CompressedRouteAttrs>)| {
                let as_path_text = match &route.as_path {
                    Some(as_path) => as_path.iter().map(|asn| asn.to_string()).collect::<Vec<_>>().join(" "),
                    None => return false,
                };
                regex.is_match(&as_path_text)
            };
            nets_filter_fn = Box::new(move |i| nets_filter_fn(i) && new_filter_fn(i))
        };

        let (tx, rx) = tokio::sync::mpsc::channel(2);

        let limits = query.limits.unwrap_or_default();
        let max_results = if limits.max_results == 0 { usize::MAX } else { limits.max_results };
        let max_results_per_table = if limits.max_results_per_table == 0 { usize::MAX } else { limits.max_results_per_table };

        rayon::spawn(move || {
            match query.net_query {
                NetQuery::Exact(net) => {
                    tables.into_par_iter().filter_map(move |(table_sel, table)| {
                        let table = table.lock().unwrap();
                        table.get(to_key(&net))
                            .map(|has_route| (table_sel.clone(), net.clone(), has_route.clone()))
                    })
                    .filter(nets_filter_fn)
                    .for_each_with(tx, |tx, res| drop(tx.blocking_send(res)));
                },
                NetQuery::MostSpecific(net) => {
                    tables.into_par_iter().filter_map(move |(table_sel, table)| {
                        let table = table.lock().unwrap();

                        table.get_longest_common_prefix(&to_key(&net))
                            .map(move |(net, route)| (table_sel.clone(), from_key(&net), route.clone()))
                            .filter(&nets_filter_fn)
                    })
                    .for_each_with(tx, |tx, res| drop(tx.blocking_send(res)));
                },
                NetQuery::Contains(net) => {
                    tables.into_par_iter().flat_map(move |(table_sel, table)| {
                        let table = table.lock().unwrap();

                        table.common_prefixes(&to_key(&net))
                            .map(move |(net, route)| (table_sel.clone(), from_key(&net), route.clone()))
                        .filter(&nets_filter_fn)
                        .take(max_results_per_table)
                        .collect::<Vec<_>>()
                        .into_par_iter()
                    })
                    .for_each_with(tx, |tx, res| drop(tx.blocking_send(res)));
                },
                NetQuery::OrLonger(net) => {
                    tables.into_par_iter().flat_map(move |(table_sel, table)| {
                        let table = table.lock().unwrap();

                        table.iter_prefix(&to_key(&net))
                            .map(move |(net, route)| (table_sel.clone(), from_key(&net), route.clone()))
                        .filter(&nets_filter_fn)
                        .take(max_results_per_table)
                        .collect::<Vec<_>>()
                        .into_par_iter()
                    })
                    .for_each_with(tx, |tx, res| drop(tx.blocking_send(res)));
                },
            };
        });

        let clients = self.clients.clone();
        let sessions = self.sessions.clone();
        Box::pin(ReceiverStream::new(rx)
            .filter_map(move |(table, net, attrs)| {
                let clients = clients.clone();
                let sessions = sessions.clone();
                async move {
                    let client = match clients.lock().unwrap().get(&table.client_addr()) {
                        Some(v) => v.clone(),
                        None => {
                            warn!("client is not connected");
                            return None;
                        }
                    };
                    let session = table.session_id()
                        .and_then(|session_id| sessions.lock().unwrap().get(&session_id).cloned());
                    Some(QueryResult {
                        net,
                        table,
                        attrs: decompress_route_attrs(&attrs),
                        client,
                        session
                    })
                }
            })
            .take(max_results))
    }

    async fn client_up(&self, client_addr: SocketAddr, client_data: Client) {
        self.clients.lock().unwrap().insert(client_addr, client_data);
    }
    async fn client_down(&self, client_addr: SocketAddr) {
        self.clients.lock().unwrap().remove(&client_addr);
        self.sessions.lock().unwrap().retain(|k, _| k.from_client != client_addr);
        self.tables.lock().unwrap().retain(|k, v| !(tables_for_client_fn(&client_addr)(&(k, v))));
        self.caches.lock().unwrap().remove_expired();
    }

    async fn session_up(&self, session: SessionId, new_state: Session) {
        self.sessions.lock().unwrap().insert(session, new_state);
    }
    async fn session_down(&self, session: SessionId, new_state: Option<Session>) {
        if let Some(new_state) = new_state {
            self.sessions.lock().unwrap().insert(session.clone(), new_state);
        } else {
            self.sessions.lock().unwrap().remove(&session);
        }
        self.tables.lock().unwrap().retain(|k, v| !(tables_for_session_fn(&session)(&(k, v))));
        self.caches.lock().unwrap().remove_expired();
    }
}
