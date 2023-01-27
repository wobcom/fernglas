mod node;

pub use node::Node;
use bitvec::prelude::*;
use std::net::{Ipv4Addr, Ipv6Addr, IpAddr};

pub type Key = BitVec<usize, Lsb0>;
pub type KeyRef<'a> = &'a BitSlice<usize, Lsb0>;

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
