use std::sync::Mutex;
use regex::Regex;
use std::pin::Pin;
use futures_util::Stream;
use futures_util::StreamExt;
use std::net::SocketAddr;
use std::sync::Arc;
use ipnet::IpNet;
use std::collections::HashMap;
use async_trait::async_trait;
use tokio_stream::wrappers::ReceiverStream;
use rayon::iter::ParallelIterator;
use rayon::iter::IntoParallelIterator;
use nibbletree::Node;
use log::*;

use crate::table::*;
use crate::compressed_attrs::*;

type RouteMap = Arc<Mutex<Node<IpNet, Vec<(u32, Arc<CompressedRouteAttrs>)>>>>;

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
    #[autometrics::autometrics]
    async fn update_route(&self, path_id: u32, net: IpNet, table: TableSelector, route: RouteAttrs) {
        let compressed = self.caches.lock().unwrap().compress_route_attrs(route);
        let table = self.get_table(table);
        let mut table = table.lock().unwrap();

        let mut new_insert = None;
        let entry = table.exact_mut(&net)
            .unwrap_or_else(|| {
                new_insert = Some(Vec::new());
                new_insert.as_mut().unwrap()
            });

        match entry.binary_search_by_key(&path_id, |(k, _)| *k) {
            Ok(index) => drop(std::mem::replace(&mut entry[index], (path_id, compressed))),
            Err(index) => entry.insert(index, (path_id, compressed)),
        };

        if let Some(insert) = new_insert {
            table.insert(&net, insert);
        }
    }

    #[autometrics::autometrics]
    async fn withdraw_route(&self, path_id: u32, net: IpNet, table: TableSelector) {
        let table = self.get_table(table);
        let mut table = table.lock().unwrap();

        let is_empty = match table.exact_mut(&net) {
            Some(entry) => {
                if let Ok(index) = entry.binary_search_by_key(&path_id, |(k, _)| *k) {
                    entry.remove(index);
                }
                entry.is_empty()
            },
            None => return,
        };
        if is_empty {
            table.remove(&net);
        }
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
                    tables.into_par_iter().flat_map(move |(table_sel, table)| {
                        let table = table.lock().unwrap();
                        table.exact(&net)
                            .into_iter()
                            .flat_map(move |routes| {
                                let table_sel = table_sel.clone();
                                routes.iter().map(move |(_path_id, route)| {
                                    (table_sel.clone(), net, route.clone())
                                })
                            })
                            .filter(&nets_filter_fn)
                            .take(max_results_per_table)
                            .collect::<Vec<_>>()
                            .into_par_iter()
                    })
                    .for_each_with(tx, |tx, res| drop(tx.blocking_send(res)));
                },
                NetQuery::MostSpecific(net) => {
                    tables.into_par_iter().flat_map(move |(table_sel, table)| {
                        let table = table.lock().unwrap();

                        table.longest_match(&net)
                            .into_iter()
                            .flat_map(move |(net, routes)| {
                                let table_sel = table_sel.clone();
                                routes.iter().map(move |(_path_id, route)| {
                                    (table_sel.clone(), net, route.clone())
                                })
                            })
                            .filter(&nets_filter_fn)
                            .take(max_results_per_table)
                            .collect::<Vec<_>>()
                            .into_par_iter()
                    })
                    .for_each_with(tx, |tx, res| drop(tx.blocking_send(res)));
                },
                NetQuery::Contains(net) => {
                    tables.into_par_iter().flat_map(move |(table_sel, table)| {
                        let table = table.lock().unwrap();

                        table.matches(&net)
                            .flat_map(move |(net, routes)| {
                                let table_sel = table_sel.clone();
                                routes.iter().map(move |(_path_id, route)| {
                                    (table_sel.clone(), net, route.clone())
                                })
                            })
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

                        table.or_longer(&net)
                            .flat_map(move |(net, routes)| {
                                let table_sel = table_sel.clone();
                                routes.iter().map(move |(_path_id, route)| {
                                    (table_sel.clone(), net, route.clone())
                                })
                            })
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
                        state: table.route_state(),
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

    async fn client_up(&self, client_addr: SocketAddr, _route_state: RouteState, client_data: Client) {
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
