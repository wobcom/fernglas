use std::net::Ipv4Addr;
use bitvec::prelude::*;
use nibbletree::node::{Node, Key};

fn to_key(bytes: &[u8], prefix_len: usize) -> Key {
    let mut key = Key::new();
    key.extend(bytes.view_bits::<Msb0>().iter().take(prefix_len));
    key
}

fn from_key(key: Key) -> (Ipv4Addr, usize) {
    let mut addr = [0u8; 4];
    let addr_view = addr.view_bits_mut::<Msb0>();
    for (i, bit) in key.iter().enumerate() {
        *addr_view.get_mut(i).unwrap() = *bit;
    }
    (Ipv4Addr::from(addr), key.len())
}

fn main() {

    let mut node = Node::new();

    let addrs: Vec<((Ipv4Addr, usize), &str)> = vec![
        (("0.0.0.0".parse().unwrap(), 0), "foo"),
        (("10.0.0.0".parse().unwrap(), 8), "bar"),
        (("11.0.0.0".parse().unwrap(), 8), "bar"),
        (("172.16.0.0".parse().unwrap(), 12), "baz1"),
        (("172.32.0.0".parse().unwrap(), 12), "baz2"),
        (("192.168.0.0".parse().unwrap(), 16), "quux1"),
        (("192.169.0.0".parse().unwrap(), 16), "quux2"),

        (("0.0.0.0".parse().unwrap(), 1), "2"),
        (("128.0.0.0".parse().unwrap(), 1), "3"),
        (("0.0.0.0".parse().unwrap(), 2), "4"),
        (("64.0.0.0".parse().unwrap(), 2), "5"),
        (("128.0.0.0".parse().unwrap(), 2), "6"),
        (("192.0.0.0".parse().unwrap(), 2), "7"),
        (("0.0.0.0".parse().unwrap(), 3), "8"),
        (("32.0.0.0".parse().unwrap(), 3), "9"),
        (("64.0.0.0".parse().unwrap(), 3), "10"),
        (("96.0.0.0".parse().unwrap(), 3), "11"),
        (("128.0.0.0".parse().unwrap(), 3), "12"),
        (("160.0.0.0".parse().unwrap(), 3), "13"),
        (("192.0.0.0".parse().unwrap(), 3), "14"),
        (("224.0.0.0".parse().unwrap(), 3), "15"),
        (("0.0.0.0".parse().unwrap(), 4), "16"),
        (("0.0.0.0".parse().unwrap(), 5), "17"),
        (("240.0.0.0".parse().unwrap(), 4), "18"),

        (("192.0.2.1".parse().unwrap(), 32), "32"),
    ];
    for (k,v) in &addrs {
        eprintln!("{:?} {:?}", k, v);
    }

    for ((addr, prefixlen), val) in addrs.into_iter() {
        let key = to_key(&addr.octets(), prefixlen);
        node.insert(key, val);
    }

    let results = node.iter().map(|(key, value)| (from_key(key), value)).collect::<Vec<_>>();

    for (k,v) in results {
        println!("{:?} {:?}", k, v);
    }
}
