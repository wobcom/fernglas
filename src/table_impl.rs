use regex::Regex;
use std::borrow::Cow;
use crate::rayon_take::ParallelIteratorExt;
use std::pin::Pin;
use futures_util::Stream;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use ipnet::{IpNet, Ipv4Net, Ipv6Net};
use std::collections::HashMap;
use async_trait::async_trait;
use tokio_stream::wrappers::ReceiverStream;
use rayon::iter::ParallelIterator;
use rayon::iter::IntoParallelRefIterator;
use rayon::iter::ParallelBridge;
use bitvec::prelude::Msb0;
use bitvec::view::BitView;
use patricia_tree::PatriciaMap;
use std::sync::RwLock;
use log::*;

use crate::table::{RouteAttrs, Query, SessionId, TableSelector, Table, NetQuery, TableQuery, QueryResult, Client, Session};

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

#[derive(Default)]
pub struct State {
    clients: HashMap<SocketAddr, Client>,
    sessions: HashMap<SessionId, Session>,
    table: PatriciaMap<HashMap<TableSelector, RouteAttrs>>,
}

#[derive(Default, Clone)]
pub struct InMemoryTable {
    state: Arc<RwLock<State>>,
}

fn tables_for_client_fn(query_from_client: SocketAddr) -> impl Fn(&TableSelector) -> bool {
    move |k| {
        k.client_addr() == &query_from_client
    }
}
fn tables_for_session_fn(session_id: SessionId) -> impl Fn(&TableSelector) -> bool {
    move |k| {
        k.session_id() == Some(&session_id)
    }
}

#[async_trait]
impl Table for InMemoryTable {
    async fn update_route(&self, net: IpNet, table: TableSelector, route: RouteAttrs) {
        let mut state = self.state.write().unwrap();
        let key = to_key(&net);

        if let Some(entry) = state.table.get_mut(&key) {
            entry.insert(table, route);
        } else {
            let mut entry = HashMap::new();
            entry.insert(table, route);
            state.table.insert(&key, entry);
        }
    }

    async fn withdraw_route(&self, net: IpNet, table: TableSelector) {
        let mut state = self.state.write().unwrap();
        let key = to_key(&net);
        if let Some(entry) = state.table.get_mut(key) {
            entry.remove(&table);
        }
    }

    fn get_routes(&self, query: Query) -> Pin<Box<dyn Stream<Item = QueryResult> + Send>> {

        let table_filter_fn: Box<dyn Fn(&TableSelector) -> bool + Send + Sync> = match query.table_query {
            Some(TableQuery::Table(table)) => Box::new(move |t| t == &table),
            Some(TableQuery::Router(client_addr)) => Box::new(tables_for_client_fn(client_addr)),
            Some(TableQuery::Session(session_id)) => Box::new(tables_for_session_fn(session_id)),
            None => Box::new(|_| true),
        };
        let mut nets_filter_fn: Box<dyn Fn(&(TableSelector, IpNet, RouteAttrs)) -> bool + Send + Sync> =
            Box::new(move |(t, _, _)| table_filter_fn(t));

        if let Some(as_path_regex) = query.as_path_regex {
            let regex = Regex::new(&as_path_regex).unwrap(); // FIXME error handling
            let new_filter_fn = move |(_, _, route): &(TableSelector, IpNet, RouteAttrs)| {
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

        let state = self.state.clone();

        rayon::spawn(move || {
            let state = state.read().unwrap();
            let add_client_info_fn = |(table, net, attrs): (TableSelector, IpNet, RouteAttrs)| {
                let client = match state.clients.get(&table.client_addr()) {
                    Some(v) => v.clone(),
                    None => {
                        warn!("client is not connected");
                        return None;
                    }
                };
                let session = table.session_id()
                    .and_then(|session_id| state.sessions.get(&session_id).cloned());
                Some(QueryResult {
                    net,
                    table,
                    attrs,
                    client,
                    session
                })
            };
            let key;
            let nets_iter: Box<dyn Iterator<Item = (Cow<'_, [u8]>, &HashMap<TableSelector, RouteAttrs>)> + Send> = match query.net_query {
                NetQuery::Exact(net) => {
                    key = to_key(&net).to_vec();
                    Box::new(state.table.get(&key).map(|v| (Cow::from(key), v)).into_iter())
                }
                NetQuery::MostSpecific(net) => {
                    key = to_key(&net).to_vec();
                    Box::new(state.table.get_longest_common_prefix(&key).into_iter().map(|(k, v)| (Cow::from(k), v)))
                }
                NetQuery::Contains(net) => {
                    key = to_key(&net).to_vec();
                    Box::new(state.table.common_prefixes(&key).map(|(k, v)| (Cow::from(k), v)))
                },
                NetQuery::OrLonger(net) => {
                    key = to_key(&net).to_vec();
                    Box::new(state.table.iter_prefix(&key).map(|(k, v)| (Cow::from(k), v)))
                },
            };
            nets_iter.par_bridge()
                .flat_map(move |(net, entry)| {
                    let net = from_key(&net);
                    entry.par_iter()
                        .map(move |(k, v)| (k.clone(), net.clone(), v.clone()))
                })
                .filter(&nets_filter_fn)
                .take2(max_results)
                .filter_map(add_client_info_fn)
                .for_each_with(tx, |tx, res| drop(tx.blocking_send(res)));
        });

        Box::pin(ReceiverStream::new(rx))
    }

    async fn client_up(&self, client_addr: SocketAddr, client_data: Client) {
        let mut state = self.state.write().unwrap();
        state.clients.insert(client_addr, client_data);
    }
    async fn client_down(&self, client_addr: SocketAddr) {
        let mut state = self.state.write().unwrap();
        state.clients.remove(&client_addr);
        state.sessions.retain(|k, _| k.from_client != client_addr);
        let filter_fn = tables_for_client_fn(client_addr);
        for (_, entry) in state.table.iter_mut() {
            entry.retain(|k, _| !(filter_fn(&k)));
        }
    }

    async fn session_up(&self, session_id: SessionId, new_state: Session) {
        let mut state = self.state.write().unwrap();
        state.sessions.insert(session_id, new_state);
    }
    async fn session_down(&self, session_id: SessionId, new_state: Option<Session>) {
        let mut state = self.state.write().unwrap();
        if let Some(new_state) = new_state {
            state.sessions.insert(session_id.clone(), new_state);
        } else {
            state.sessions.remove(&session_id);
        }
        let filter_fn = tables_for_session_fn(session_id);
        for (_, entry) in state.table.iter_mut() {
            entry.retain(|k, _| !(filter_fn(&k)));
        }
    }
}
