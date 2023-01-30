use super::node::Node;
use super::{Key, KeyRef};
use std::marker::PhantomData;
use std::fmt::Debug;
use std::net::{Ipv4Addr, Ipv6Addr, IpAddr};
use bitvec::prelude::*;

pub trait FromKey {
    fn from_key(key: KeyRef) -> Self;
    fn from_key_owned(key: Key) -> Self where Self: Sized {
        Self::from_key(key.as_bitslice())
    }
}
pub trait ToKey {
    fn to_key(&self) -> Key;
}

impl FromKey for (Ipv4Addr, usize) {
    fn from_key(key: KeyRef) -> (Ipv4Addr, usize) {
        let mut addr = [0u8; 4];
        let addr_view = addr.view_bits_mut::<Msb0>();
        for (i, bit) in key.iter().enumerate() {
            *addr_view.get_mut(i).unwrap() = *bit;
        }
        (Ipv4Addr::from(addr), key.len())
    }
}

impl ToKey for (Ipv4Addr, usize) {
    fn to_key(&self) -> Key {
        let mut key = Key::new();
        key.extend(self.0.octets().view_bits::<Msb0>().iter().take(self.1));
        key
    }
}

impl FromKey for (Ipv6Addr, usize) {
    fn from_key(key: KeyRef) -> (Ipv6Addr, usize) {
        let mut addr = [0u8; 16];
        let addr_view = addr.view_bits_mut::<Msb0>();
        for (i, bit) in key.iter().enumerate() {
            *addr_view.get_mut(i).unwrap() = *bit;
        }
        (Ipv6Addr::from(addr), key.len())
    }
}

impl ToKey for (Ipv6Addr, usize) {
    fn to_key(&self) -> Key {
        let mut key = Key::new();
        key.extend(self.0.octets().view_bits::<Msb0>().iter().take(self.1));
        key
    }
}

impl FromKey for (IpAddr, usize) {
    fn from_key(key: KeyRef) -> (IpAddr, usize) {
        let is_ipv6 = key[0];
        let key = &key[1..];

        if is_ipv6 {
            let (addr, len) = <(Ipv6Addr, usize)>::from_key(key);
            (addr.into(), len)
        } else {
            let (addr, len) = <(Ipv4Addr, usize)>::from_key(key);
            (addr.into(), len)
        }
    }
}

impl ToKey for (IpAddr, usize) {
    fn to_key(&self) -> Key {
        let mut key = Key::new();
        key.push(self.0.is_ipv6());
        key.extend(match self.0 {
            IpAddr::V4(v4) => (v4, self.1).to_key(),
            IpAddr::V6(v6) => (v6, self.1).to_key(),
        });
        key
    }
}

#[cfg(feature = "ipnet")]
impl FromKey for ipnet::Ipv4Net {
    fn from_key(key: KeyRef) -> ipnet::Ipv4Net {
        let (addr, prefix_len) = <(Ipv4Addr, usize)>::from_key(key);
        ipnet::Ipv4Net::new(addr, prefix_len as u8).unwrap()
    }
}

#[cfg(feature = "ipnet")]
impl ToKey for ipnet::Ipv4Net {
    fn to_key(&self) -> Key {
        (self.addr(), self.prefix_len() as usize).to_key()
    }
}

#[cfg(feature = "ipnet")]
impl FromKey for ipnet::Ipv6Net {
    fn from_key(key: KeyRef) -> ipnet::Ipv6Net {
        let (addr, prefix_len) = <(Ipv6Addr, usize)>::from_key(key);
        ipnet::Ipv6Net::new(addr, prefix_len as u8).unwrap()
    }
}

#[cfg(feature = "ipnet")]
impl ToKey for ipnet::Ipv6Net {
    fn to_key(&self) -> Key {
        (self.addr(), self.prefix_len() as usize).to_key()
    }
}

#[cfg(feature = "ipnet")]
impl FromKey for ipnet::IpNet {
    fn from_key(key: KeyRef) -> ipnet::IpNet {
        let (addr, prefix_len) = <(IpAddr, usize)>::from_key(key);
        ipnet::IpNet::new(addr, prefix_len as u8).unwrap()
    }
}

#[cfg(feature = "ipnet")]
impl ToKey for ipnet::IpNet {
    fn to_key(&self) -> Key {
        (self.addr(), self.prefix_len() as usize).to_key()
    }
}

#[cfg(feature = "ip_network")]
impl FromKey for ip_network::Ipv4Network {
    fn from_key(key: KeyRef) -> ip_network::Ipv4Network {
        let (addr, netmask) = <(Ipv4Addr, usize)>::from_key(key);
        ip_network::Ipv4Network::new(addr, netmask as u8).unwrap()
    }
}

#[cfg(feature = "ip_network")]
impl ToKey for ip_network::Ipv4Network {
    fn to_key(&self) -> Key {
        (self.network_address(), self.netmask() as usize).to_key()
    }
}

#[cfg(feature = "ip_network")]
impl FromKey for ip_network::Ipv6Network {
    fn from_key(key: KeyRef) -> ip_network::Ipv6Network {
        let (addr, netmask) = <(Ipv6Addr, usize)>::from_key(key);
        ip_network::Ipv6Network::new(addr, netmask as u8).unwrap()
    }
}

#[cfg(feature = "ip_network")]
impl ToKey for ip_network::Ipv6Network {
    fn to_key(&self) -> Key {
        (self.network_address(), self.netmask() as usize).to_key()
    }
}

#[cfg(feature = "ip_network")]
impl FromKey for ip_network::IpNetwork {
    fn from_key(key: KeyRef) -> ip_network::IpNetwork {
        let (addr, netmask) = <(IpAddr, usize)>::from_key(key);
        ip_network::IpNetwork::new(addr, netmask as u8).unwrap()
    }
}

#[cfg(feature = "ip_network")]
impl ToKey for ip_network::IpNetwork {
    fn to_key(&self) -> Key {
        (self.network_address(), self.netmask() as usize).to_key()
    }
}

impl FromKey for Key {
    fn from_key(key: KeyRef) -> Key {
        key.to_bitvec()
    }
}

impl ToKey for Key {
    fn to_key(&self) -> Key {
        self.clone()
    }
}

#[derive(Debug)]
pub struct NodeWithKey<K, T> {
    node: Node<T>,
    _key_type: PhantomData<K>,
}

impl<K, T> Default for NodeWithKey<K, T> {
    fn default() -> Self {
        NodeWithKey {
            node: Default::default(),
            _key_type: Default::default(),
        }
    }
}

impl<K: FromKey + ToKey + Debug, T: Debug> NodeWithKey<K, T> {
    pub fn insert(&mut self, key: &K, value: T) -> Option<T> {
        self.node.insert(&key.to_key(), value)
    }
    pub fn remove(&mut self, key: &K) -> Option<T> {
        self.node.remove(&key.to_key())
    }

    pub fn iter(&self) -> impl Iterator<Item = (K, &T)> + '_ {
        self.node.iter()
            .map(|(k, v)| (K::from_key_owned(k), v))
    }
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (K, &mut T)> + '_ {
        self.node.iter_mut()
            .map(|(k, v)| (K::from_key_owned(k), v))
    }

    pub fn values(&self) -> impl Iterator<Item = &T> + '_ {
        self.node.values()
    }
    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut T> + '_ {
        self.node.values_mut()
    }

    pub fn keys(&self) -> impl Iterator<Item = K> + '_ {
        self.node.keys()
            .map(K::from_key_owned)
    }
    pub fn exact(&self, key: &K) -> Option<&T> {
        self.node.exact(&key.to_key())
    }
    pub fn exact_mut(&mut self, key: &K) -> Option<&mut T> {
        self.node.exact_mut(&key.to_key())
    }

    pub fn longest_match(&self, key: &K) -> Option<(K, &T)> {
        self.node.longest_match(&key.to_key())
            .map(|(k, v)| (K::from_key_owned(k), v))
    }

    pub fn or_longer(&self, key: &K) -> impl Iterator<Item = (K, &T)> + '_ {
        self.node.or_longer(key.to_key())
            .map(|(k, v)| (K::from_key_owned(k), v))
    }

    pub fn matches(&self, key: &K) -> impl Iterator<Item = (K, &T)> + '_ {
        self.node.matches(key.to_key())
            .map(|(k, v)| (K::from_key_owned(k), v))
    }
}
