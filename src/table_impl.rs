use crate::compressed_attrs::*;
use crate::store::*;
use ipnet::IpNet;
use nibbletree::Node;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::Weak;

pub type Subscriber<T> = dyn Fn(IpNet, u32, Action<T>) + Send + Sync;

#[derive(Clone, Debug, PartialEq)]
pub enum Action<T> {
    Update(T),
    Withdraw,
}

pub trait Compressable {
    type Compressed;
    fn compress(self, caches: &Arc<Mutex<Caches>>) -> Self::Compressed;
}

impl Compressable for RouteAttrs {
    type Compressed = Arc<CompressedRouteAttrs>;
    fn compress(self, caches: &Arc<Mutex<Caches>>) -> Self::Compressed {
        caches.lock().unwrap().compress_route_attrs(self)
    }
}

impl<T: Compressable> Compressable for Action<T> {
    type Compressed = Action<T::Compressed>;
    fn compress(self, caches: &Arc<Mutex<Caches>>) -> Self::Compressed {
        match self {
            Action::Withdraw => Action::Withdraw,
            Action::Update(attrs) => Action::Update(attrs.compress(caches)),
        }
    }
}

pub struct InMemoryTableState<T = Arc<CompressedRouteAttrs>> {
    pub table: Node<IpNet, Vec<(PathId, T)>>,
}

impl<T> Default for InMemoryTableState<T> {
    fn default() -> Self {
        Self {
            table: Default::default(),
        }
    }
}

#[derive(Clone)]
pub struct InMemoryTable<T: Compressable = RouteAttrs> {
    pub state: Arc<Mutex<InMemoryTableState<T::Compressed>>>,
    subscribers: Arc<Mutex<Vec<Weak<Subscriber<T::Compressed>>>>>,
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
            Some(NetQuery::Exact(net)) => Box::new(self.exact(net).map(|x| (*net, x)).into_iter()),
            Some(NetQuery::MostSpecific(net)) => Box::new(self.longest_match(net).into_iter()),
            Some(NetQuery::Contains(net)) => Box::new(self.matches(net)),
            Some(NetQuery::OrLonger(net)) => Box::new(self.or_longer(net)),
        };
        Box::new(iter.flat_map(move |(net, routes)| {
            routes
                .iter()
                .map(move |(path_id, route)| (net, *path_id, route.clone()))
        }))
    }
}

impl<T: Clone + Send + Sync> std::iter::Extend<(IpNet, PathId, T)> for InMemoryTableState<T> {
    fn extend<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = (IpNet, PathId, T)>,
    {
        for (net, num, attrs) in iter {
            self.update_route(num, net, attrs);
        }
    }
}
impl<T: Clone + Send + Sync> std::iter::FromIterator<(IpNet, PathId, T)> for InMemoryTableState<T> {
    fn from_iter<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = (IpNet, PathId, T)>,
    {
        let mut table = Self::default();
        table.extend(iter);
        table
    }
}
impl<T: Clone + Send + Sync> std::iter::Extend<(IpNet, PathId, Action<T>)>
    for InMemoryTableState<T>
{
    fn extend<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = (IpNet, PathId, Action<T>)>,
    {
        for (net, num, action) in iter {
            match action {
                Action::Update(attrs) => self.update_route(num, net, attrs),
                Action::Withdraw => self.withdraw_route(num, net),
            }
        }
    }
}

impl<T: Clone + Send + Sync> InMemoryTableState<T> {
    pub fn update_route(&mut self, path_id: PathId, net: IpNet, attrs: T) {
        let mut new_insert = None;
        let entry = self.table.exact_mut(&net).unwrap_or_else(|| {
            new_insert = Some(Vec::new());
            new_insert.as_mut().unwrap()
        });

        match entry.binary_search_by_key(&path_id, |(k, _)| *k) {
            Ok(index) => drop(std::mem::replace(&mut entry[index], (path_id, attrs))),
            Err(index) => entry.insert(index, (path_id, attrs)),
        };

        if let Some(insert) = new_insert {
            self.table.insert(&net, insert);
        }
    }

    pub fn withdraw_route(&mut self, path_id: PathId, net: IpNet) {
        let is_empty = match self.table.exact_mut(&net) {
            Some(entry) => {
                if let Ok(index) = entry.binary_search_by_key(&path_id, |(k, _)| *k) {
                    entry.remove(index);
                }
                entry.is_empty()
            }
            None => return,
        };
        if is_empty {
            self.table.remove(&net);
        }
    }
}

impl<C: Clone + Send + Sync, T: Compressable<Compressed = C>> InMemoryTable<T> {
    pub fn new(caches: Arc<Mutex<Caches>>) -> Self {
        Self {
            state: Default::default(),
            subscribers: Default::default(),
            caches,
        }
    }

    pub fn subscribe(&self, cb: Weak<Subscriber<C>>) {
        self.subscribers.lock().unwrap().push(cb);
    }

    pub fn update_route(&self, path_id: PathId, net: IpNet, route: T) {
        let compressed = route.compress(&self.caches);
        for subscriber in self
            .subscribers
            .lock()
            .unwrap()
            .iter()
            .filter_map(Weak::upgrade)
        {
            (*subscriber)(net, path_id, Action::Update(compressed.clone()));
        }

        let mut state = self.state.lock().unwrap();
        state.update_route(path_id, net, compressed);
    }

    pub fn withdraw_route(&self, path_id: PathId, net: IpNet) {
        for subscriber in self
            .subscribers
            .lock()
            .unwrap()
            .iter()
            .filter_map(Weak::upgrade)
        {
            (*subscriber)(net, path_id, Action::Withdraw);
        }

        let mut state = self.state.lock().unwrap();
        state.withdraw_route(path_id, net);
    }
}
