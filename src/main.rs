use std::net::IpAddr;
use nibbletree::Node;

fn main() {

    let mut node = Node::new();

    let addrs: Vec<((IpAddr, usize), &str)> = vec![
        (("0.0.0.0".parse().unwrap(), 0), "foo"),
        (("10.0.0.0".parse().unwrap(), 8), "bar"),
        //(("11.0.0.0".parse().unwrap(), 8), "bar"),
        (("172.16.0.0".parse().unwrap(), 12), "baz1"),
        //(("172.32.0.0".parse().unwrap(), 12), "baz2"),
        (("192.168.0.0".parse().unwrap(), 16), "quux1"),
        //(("192.169.0.0".parse().unwrap(), 16), "quux2"),

        //(("0.0.0.0".parse().unwrap(), 0), "2"),
        //(("0.0.0.0".parse().unwrap(), 1), "2"),
        //(("128.0.0.0".parse().unwrap(), 1), "3"),
        //(("0.0.0.0".parse().unwrap(), 2), "4"),
        //(("64.0.0.0".parse().unwrap(), 2), "5"),
        //(("128.0.0.0".parse().unwrap(), 2), "6"),
        //(("192.0.0.0".parse().unwrap(), 2), "7"),
        //(("0.0.0.0".parse().unwrap(), 3), "8"),
        //(("32.0.0.0".parse().unwrap(), 3), "9"),
        //(("64.0.0.0".parse().unwrap(), 3), "10"),
        //(("96.0.0.0".parse().unwrap(), 3), "11"),
        //(("128.0.0.0".parse().unwrap(), 3), "12"),
        //(("160.0.0.0".parse().unwrap(), 3), "13"),
        //(("192.0.0.0".parse().unwrap(), 3), "14"),
        //(("224.0.0.0".parse().unwrap(), 3), "15"),
        //(("0.0.0.0".parse().unwrap(), 4), "16"),
        //(("0.0.0.0".parse().unwrap(), 5), "17"),
        //(("240.0.0.0".parse().unwrap(), 4), "18"),

        //(("192.0.2.1".parse().unwrap(), 32), "32"),
    ];
    for (k,v) in &addrs {
        eprintln!("{:?} {:?}", k, v);
    }

    for (key, val) in addrs.into_iter() {
        node.insert(&key, val);
    }

    println!("! {:?}", node.longest_match(&("172.15.0.1".parse().unwrap(), 32)));
    println!("! {:?}", node.longest_match(&("10.0.0.1".parse().unwrap(), 32)));
    println!("! {:?}", node.longest_match(&("172.16.0.1".parse().unwrap(), 32)));
    println!("! {:?}", node.longest_match(&("192.168.0.1".parse().unwrap(), 32)));
    println!("! {:?}", node.longest_match(&("192.168.1.0".parse().unwrap(), 24)));

    let results = node.or_longer(&("128.0.0.0".parse().unwrap(), 1)).collect::<Vec<_>>();

    for (k,v) in results {
        println!("{:?} {:?}", k, v);
    }
}
