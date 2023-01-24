use std::hash::Hash;
use std::net::IpAddr;
use std::sync::{Arc, Weak};
use weak_table::WeakHashSet;
use weak_table::traits::WeakKey;

use crate::table::*;

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct CompressedRouteAttrs {
    pub origin: Option<RouteOrigin>,
    pub as_path: Option<Arc<Vec<u32>>>,
    pub communities: Option<Arc<Vec<(u16, u16)>>>,
    pub large_communities: Option<Arc<Vec<Arc<(u32, u32, u32)>>>>,
    pub med: Option<u32>,
    pub local_pref: Option<u32>,
    pub nexthop: Option<IpAddr>,
}

#[derive(Default)]
pub struct Caches {
    large_communities_cache: WeakHashSet<Weak<(u32, u32, u32)>>,
    large_communities_list_cache: WeakHashSet<Weak<Vec<Arc<(u32, u32, u32)>>>>,
    communities_list_cache: WeakHashSet<Weak<Vec<(u16, u16)>>>,
    as_path_cache: WeakHashSet<Weak<Vec<u32>>>,
    route_attrs_cache: WeakHashSet<Weak<CompressedRouteAttrs>>,
}

trait WeakHashSetExt<T: Clone + 'static> {
    fn get_or_insert(&mut self, val: T) -> Arc<T>;
}
impl<T: Clone + Eq + Hash + 'static> WeakHashSetExt<T> for WeakHashSet<Weak<T>>
where
    Weak<T>: WeakKey<Key = T, Strong = Arc<T>>,
{
    fn get_or_insert(&mut self, val: T) -> Arc<T> {
        self.get(&val).unwrap_or_else(|| {
            let arc = Arc::new(val);
            self.insert(arc.clone());
            arc
        })
    }
}

impl Caches {
    pub fn compress_route_attrs(&mut self, route: RouteAttrs) -> Arc<CompressedRouteAttrs> {
        let route = CompressedRouteAttrs {
            as_path: route.as_path.map(|x| self.as_path_cache.get_or_insert(x).clone()),
            communities: route.communities.map(|x| self.communities_list_cache.get_or_insert(x).clone()),
            large_communities: route.large_communities.map(|x| {
                let list = x.into_iter().map(|c| self.large_communities_cache.get_or_insert(c)).collect();
                self.large_communities_list_cache.get_or_insert(list).clone()
            }),
            local_pref: route.local_pref,
            med: route.med,
            origin: route.origin,
            nexthop: route.nexthop
        };
        self.route_attrs_cache.get_or_insert(route)
    }

    pub fn remove_expired(&mut self) {
        self.large_communities_cache.remove_expired();
        self.large_communities_list_cache.remove_expired();
        self.communities_list_cache.remove_expired();
        self.as_path_cache.remove_expired();
        self.route_attrs_cache.remove_expired();
    }
}

pub fn decompress_route_attrs(route: &CompressedRouteAttrs) -> RouteAttrs {
    RouteAttrs {
        as_path: route.as_path.as_ref().map(|x| (**x).clone()),
        communities: route.communities.as_ref().map(|x| (**x).clone()),
        large_communities: route.large_communities.as_ref().map(|x| x.iter().map(|x| **x).collect()),
        local_pref: route.local_pref,
        med: route.med,
        origin: route.origin.clone(),
        nexthop: route.nexthop
    }
}
