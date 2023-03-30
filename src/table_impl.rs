use std::sync::Mutex;
use std::sync::Arc;
use ipnet::IpNet;
use nibbletree::Node;
use crate::store::*;
use crate::compressed_attrs::*;

#[derive(Clone)]
pub struct InMemoryTable {
    pub table: Arc<Mutex<Node<IpNet, Vec<(u32, Arc<CompressedRouteAttrs>)>>>>,
    caches: Arc<Mutex<Caches>>,
}
impl InMemoryTable {
    pub fn new(caches: Arc<Mutex<Caches>>) -> Self {
        Self {
            table: Default::default(),
            caches,
        }
    }

    pub async fn update_route(&self, path_id: u32, net: IpNet, route: RouteAttrs) {
        let compressed = self.caches.lock().unwrap().compress_route_attrs(route);

        let mut table = self.table.lock().unwrap();

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

    pub async fn withdraw_route(&self, path_id: u32, net: IpNet) {
        let mut table = self.table.lock().unwrap();

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
}
