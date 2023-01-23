use std::net::Ipv4Addr;
use bitvec::prelude::*;
use nibbletree::node::Node;

fn into_nibbles(bytes: &[u8]) -> Vec<u8> {
    bytes.view_bits::<Msb0>().chunks_exact(4).map(|x| x.load_be()).collect()
}

fn from_nibbles(nibbles: &[u8]) -> Vec<u8> {
    let mut iter = nibbles.chunks_exact(2);
    let remainder = iter.remainder();
    let mut res = iter
        .map(|i| (i[0] << 4) + i[1])
        .collect::<Vec<_>>();
    if !remainder.is_empty() {
        res.push(remainder[0] << 4);
    }
    res
}

fn main() {

    let mut node = Node::new();

    let addrs: Vec<(Ipv4Addr, u32, &str)> = vec![
        //("0.0.0.0".parse().unwrap(), 0, "foo"),
        //("10.0.0.0".parse().unwrap(), 8, "bar"),
        //("11.0.0.0".parse().unwrap(), 8, "bar"),
        //("172.16.0.0".parse().unwrap(), 12, "baz1"),
        //("172.32.0.0".parse().unwrap(), 12, "baz2"),
        //("192.168.0.0".parse().unwrap(), 16, "quux1"),
        //("192.169.0.0".parse().unwrap(), 16, "quux2"),

        ("0.0.0.0".parse().unwrap(), 0, "1"),
        ("0.0.0.0".parse().unwrap(), 1, "2"),
        ("128.0.0.0".parse().unwrap(), 1, "3"),
        ("0.0.0.0".parse().unwrap(), 2, "4"),
        ("64.0.0.0".parse().unwrap(), 2, "5"),
        ("128.0.0.0".parse().unwrap(), 2, "6"),
        ("192.0.0.0".parse().unwrap(), 2, "7"),
        ("0.0.0.0".parse().unwrap(), 3, "8"),
        ("32.0.0.0".parse().unwrap(), 3, "9"),
        ("64.0.0.0".parse().unwrap(), 3, "10"),
        ("96.0.0.0".parse().unwrap(), 3, "11"),
        ("128.0.0.0".parse().unwrap(), 3, "12"),
        ("160.0.0.0".parse().unwrap(), 3, "13"),
        ("192.0.0.0".parse().unwrap(), 3, "14"),
        ("224.0.0.0".parse().unwrap(), 3, "15"),
        ("0.0.0.0".parse().unwrap(), 4, "16"),
        ("0.0.0.0".parse().unwrap(), 5, "17"),
        ("240.0.0.0".parse().unwrap(), 4, "18"),
    ];
    println!("{:?}", addrs);
    let nibbles: Vec<_> = addrs.into_iter().map(|(addr, prefixlen, val)| (into_nibbles(&addr.octets()), prefixlen, val)).collect();

    for (nibbles, prefixlen, val) in nibbles.into_iter() {
        node.insert(&nibbles, prefixlen, val);
    }

    println!("{:#x?}", node);

    for ((nibbles, prefix_len), val) in node.iter() {
        println!("{:?} {} {}", from_nibbles(&nibbles), prefix_len, val);
    }
}
