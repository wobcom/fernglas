use std::sync::Mutex;
use regex::Regex;
use std::pin::Pin;
use futures_util::Stream;
use std::net::Ipv4Addr;
use std::sync::Arc;
use ipnet::IpNet;
use std::collections::HashMap;
use async_trait::async_trait;
use tokio_stream::wrappers::ReceiverStream;
use rayon::iter::ParallelIterator;
use rayon::iter::IntoParallelIterator;

use crate::table::{Route, Query, SessionId, TableSelector, Table, NetQuery};

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

        let nets_filter_fn: Box<dyn Fn(&(TableSelector, IpNet, Route)) -> bool + Send + Sync> = match query.net {
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

        let (tx, rx) = tokio::sync::mpsc::channel(200);

        rayon::spawn(move || {
            match query.net {
                Some(NetQuery::Exact(net)) => {
                    tables.into_par_iter().filter_map(move |(table_sel, table)| {
                        let table = table.lock().unwrap();
                        table.get(&net)
                            .map(|has_route| (table_sel.clone(), net.clone(), has_route.clone()))
                    })
                            // filter is accepting anything anyways
                            //.filter(nets_filter_fn)
                    //.take(200)
                    .for_each_with(tx, |tx, res| tx.blocking_send(res).unwrap());
                },
                _ => {
                    tables.into_par_iter().flat_map(move |(table_sel, table)| {
                        let table = table.lock().unwrap();
                        table.iter()
                            .map(move |(net, route)| (table_sel.clone(), net.clone(), route.clone()))
                            .filter(&nets_filter_fn)
                            .take(200)
                            .collect::<Vec<_>>()
                            .into_par_iter()
                    })
                    //.take(200)
                    .for_each_with(tx, |tx, res| tx.blocking_send(res).unwrap());

                }
            };
        });

        Box::pin(ReceiverStream::new(rx))
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
