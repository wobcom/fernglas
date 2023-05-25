use std::sync::Mutex;
use std::sync::Arc;
use ipnet::IpNet;
use nibbletree::Node;
use crate::store::*;
use crate::compressed_attrs::*;

#[derive(Default)]
pub struct InMemoryTableState {
    // provide efficient prefix tree lookups
    pub table: Node<IpNet, Vec<(u32, Arc<CompressedRouteAttrs>)>>,
    // provide a defined ordering for diffs
    pub vec: Vec<IpNet>,
}

#[derive(Clone)]
pub struct InMemoryTable {
    pub state: Arc<Mutex<InMemoryTableState>>,
    caches: Arc<Mutex<Caches>>,
}

pub trait NodeExt {
    fn get_routes(&self, net_query: Option<&NetQuery>) -> Box<dyn Iterator<Item = (IpNet, u32, Arc<CompressedRouteAttrs>)> + Send + '_>;
}

impl NodeExt for Node<IpNet, Vec<(u32, Arc<CompressedRouteAttrs>)>> {
    fn get_routes(&self, net_query: Option<&NetQuery>) -> Box<dyn Iterator<Item = (IpNet, u32, Arc<CompressedRouteAttrs>)> + Send + '_> {
        let iter: Box<dyn Iterator<Item = (IpNet, &Vec<(u32, Arc<CompressedRouteAttrs>)>)> + Send + '_> = match net_query {
            None => {
                Box::new(self.iter())
            },
            Some(NetQuery::Exact(net)) => {
                Box::new(self.exact(&net).map(|x| (*net, x)).into_iter())
            },
            Some(NetQuery::MostSpecific(net)) => {
                Box::new(self.longest_match(&net).into_iter())
            },
            Some(NetQuery::Contains(net)) => {
                Box::new(self.matches(&net))
            },
            Some(NetQuery::OrLonger(net)) => {
                Box::new(self.or_longer(&net))
            },
        };
        Box::new(iter
            .flat_map(move |(net, routes)| {
                routes.iter().map(move |(path_id, route)| {
                    (net, *path_id, route.clone())
                })
            }))
    }
}

impl InMemoryTable {
    pub fn new(caches: Arc<Mutex<Caches>>) -> Self {
        Self {
            state: Default::default(),
            caches,
        }
    }

    pub async fn update_route_compressed(&self, path_id: u32, net: IpNet, compressed: Arc<CompressedRouteAttrs>) {
        let mut state = self.state.lock().unwrap();

        let mut new_insert = None;
        let entry = state.table.exact_mut(&net)
            .unwrap_or_else(|| {
                new_insert = Some(Vec::new());
                new_insert.as_mut().unwrap()
            });

        match entry.binary_search_by_key(&path_id, |(k, _)| *k) {
            Ok(index) => drop(std::mem::replace(&mut entry[index], (path_id, compressed))),
            Err(index) => entry.insert(index, (path_id, compressed)),
        };

        if let Some(insert) = new_insert {
            state.table.insert(&net, insert);

            let index = state.vec.partition_point(|&x| x < net);
            state.vec.insert(index, net);
        }
    }
    pub async fn update_route(&self, path_id: u32, net: IpNet, route: RouteAttrs) {
        let compressed = self.caches.lock().unwrap().compress_route_attrs(route);
        self.update_route_compressed(path_id, net, compressed).await;
    }

    pub async fn withdraw_route(&self, path_id: u32, net: IpNet) {
        let mut state = self.state.lock().unwrap();

        let is_empty = match state.table.exact_mut(&net) {
            Some(entry) => {
                if let Ok(index) = entry.binary_search_by_key(&path_id, |(k, _)| *k) {
                    entry.remove(index);
                }
                entry.is_empty()
            },
            None => return,
        };
        if is_empty {
            state.table.remove(&net);
            if let Ok(index) = state.vec.binary_search(&net) {
                state.vec.remove(index);
            }
        }
    }
}
