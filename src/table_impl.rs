use crate::compressed_attrs::*;
use crate::store::*;
use ipnet::IpNet;
use nibbletree::Node;
use std::sync::Arc;
use std::sync::Mutex;

#[derive(Clone)]
pub struct InMemoryTable {
    pub table: Arc<Mutex<Node<IpNet, Vec<(PathId, Arc<CompressedRouteAttrs>)>>>>,
    caches: Arc<Mutex<Caches>>,
}

pub trait NodeExt {
    fn get_routes(
        &self,
        net_query: Option<&NetQuery>,
    ) -> Box<dyn Iterator<Item = (IpNet, PathId, Arc<CompressedRouteAttrs>)> + Send + '_>;
}

impl NodeExt for Node<IpNet, Vec<(PathId, Arc<CompressedRouteAttrs>)>> {
    fn get_routes(
        &self,
        net_query: Option<&NetQuery>,
    ) -> Box<dyn Iterator<Item = (IpNet, PathId, Arc<CompressedRouteAttrs>)> + Send + '_> {
        let iter: Box<
            dyn Iterator<Item = (IpNet, &Vec<(PathId, Arc<CompressedRouteAttrs>)>)> + Send + '_,
        > = match net_query {
            None => Box::new(self.iter()),
            Some(NetQuery::Exact(net)) => Box::new(self.exact(&net).map(|x| (*net, x)).into_iter()),
            Some(NetQuery::MostSpecific(net)) => Box::new(self.longest_match(&net).into_iter()),
            Some(NetQuery::Contains(net)) => Box::new(self.matches(&net)),
            Some(NetQuery::OrLonger(net)) => Box::new(self.or_longer(&net)),
        };
        Box::new(iter.flat_map(move |(net, routes)| {
            routes
                .iter()
                .map(move |(path_id, route)| (net, *path_id, route.clone()))
        }))
    }
}

impl InMemoryTable {
    pub fn new(caches: Arc<Mutex<Caches>>) -> Self {
        Self {
            table: Default::default(),
            caches,
        }
    }

    pub async fn update_route(&self, path_id: PathId, net: IpNet, route: RouteAttrs) {
        let compressed = self.caches.lock().unwrap().compress_route_attrs(route);

        let mut table = self.table.lock().unwrap();

        let mut new_insert = None;
        let entry = table.exact_mut(&net).unwrap_or_else(|| {
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

    pub async fn withdraw_route(&self, path_id: PathId, net: IpNet) {
        let mut table = self.table.lock().unwrap();

        let is_empty = match table.exact_mut(&net) {
            Some(entry) => {
                if let Ok(index) = entry.binary_search_by_key(&path_id, |(k, _)| *k) {
                    entry.remove(index);
                }
                entry.is_empty()
            }
            None => return,
        };
        if is_empty {
            table.remove(&net);
        }
    }
}
