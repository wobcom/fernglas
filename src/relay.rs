use log::{warn, info};
use zettabgp::prelude::*;
use tokio::io::AsyncWriteExt;
use serde::Deserialize;
use std::net::SocketAddr;
use std::net::Ipv4Addr;
use futures_util::{StreamExt, pin_mut};
use futures_util::future::join_all;
use ipnet::IpNet;
use std::cmp::Ordering;
use tokio::net::TcpListener;
use std::sync::Arc;
use tokio::net::TcpStream;

use crate::bgpdumper::BgpDumper;
use crate::store::*;
use crate::store_impl::*;
use crate::compressed_attrs::*;
use crate::table_impl::*;

#[derive(Debug, Deserialize, Clone)]
pub struct RelayConfig {
    table: TableSelector,

    pub bind: SocketAddr,

    pub asn: u32,
    pub router_id: Ipv4Addr,
    pub add_path: bool,
}

pub enum Action {
    Update(IpNet, u32, Arc<CompressedRouteAttrs>),
    Withdraw(IpNet, u32),
}

fn diff<S, T: Clone, U: Ord, F: Fn(&T) -> U, F1: Fn(&mut S, &T), F2: Fn(&mut S, &T), F3: Fn(&mut S, &T, &T)>(state: &mut S, table: &Vec<T>, rib_out: &Vec<T>, key_fn: F, update_fn: F1, withdraw_fn: F2, keep_fn: F3) {
    let mut table_iter = table.iter().peekable();
    let mut rib_out_iter = rib_out.iter().peekable();

    loop {
        match (table_iter.peek(), rib_out_iter.peek()) {
            (Some(a), Some(b)) => {
                match key_fn(a).cmp(&key_fn(b)) {
                    Ordering::Less => update_fn(state, table_iter.next().unwrap()),
                    Ordering::Greater => withdraw_fn(state, rib_out_iter.next().unwrap()),
                    Ordering::Equal => keep_fn(state, table_iter.next().unwrap(), rib_out_iter.next().unwrap()),
                }
            },
            (Some(_), None) => update_fn(state, table_iter.next().unwrap()),
            (None, Some(_)) => withdraw_fn(state, rib_out_iter.next().unwrap()),
            (None, None) => break,
        }
    }
}

pub async fn run(cfg: RelayConfig, store: InMemoryStore, mut shutdown: tokio::sync::watch::Receiver<bool>) -> anyhow::Result<()> {
    let listener = TcpListener::bind(cfg.bind).await?;
    loop {
        let mut running_tasks = vec![];
        tokio::select! {
            new_conn = listener.accept() => {
                let (io, client_addr) = new_conn?;
                info!("connected {:?}", client_addr);

                let store = store.clone();
                let mut shutdown = shutdown.clone();
                let cfg = cfg.clone();
                running_tasks.push(tokio::spawn(async move {
                    tokio::select! {
                        res = run_peer(cfg, store, io, client_addr) => {
                            match res {
                                Err(e) => warn!("disconnected {} {}", client_addr, e),
                                Ok(notification) => info!("disconnected {} {:?}", client_addr, notification),
                            }
                        }
                        _ = shutdown.changed() => {
                        }
                    };
                }));
            }
            _ = shutdown.changed() => {
                join_all(running_tasks).await;
                break Ok(());
            }
        }
    }
}
pub async fn run_peer(cfg: RelayConfig, store: InMemoryStore, stream: TcpStream, client_addr: SocketAddr) -> anyhow::Result<BgpNotificationMessage> {
    let rib_out = InMemoryTable::new(store.caches.clone());
    let mut caps = vec![
        BgpCapability::SafiIPv4u,
        BgpCapability::SafiIPv6u,
        BgpCapability::CapRR,
        BgpCapability::CapASN32(cfg.asn),
    ];
    if cfg.add_path {
        caps.push(BgpCapability::CapAddPath(vec![BgpCapAddPath::new_from_cap(BgpCapability::SafiIPv4u, true, true).unwrap(), BgpCapAddPath::new_from_cap(BgpCapability::SafiIPv6u, true, true).unwrap()]));
    }
    let mut dumper = BgpDumper::new(
        BgpSessionParams::new(
            cfg.asn,
            180,
            BgpTransportMode::IPv4,
            cfg.router_id,
            caps,
        ),
        stream,
    );
    let open_message = dumper.start_active().await?;
    let write = dumper.write.clone();
    let params = dumper.params.clone();
    let stream = dumper.lifecycle();
    pin_mut!(stream);
    loop {
        let mut changes = vec![];
        {
            let table = store.get_table(cfg.table.clone());
            let table = table.state.lock().unwrap();
            let rib_out = rib_out.state.lock().unwrap();
            let apply_nums = |s: &mut Vec<Action>, net: &IpNet, x: &Vec<(u32, Arc<CompressedRouteAttrs>)>, y: &Vec<(u32, Arc<CompressedRouteAttrs>)>| {
                diff(s, x, y, |x| x.0, |s, x| {
                    s.push(Action::Update(*net, x.0, x.1.clone()));
                }, |s, x| {
                    s.push(Action::Withdraw(*net, x.0));
                }, |s, x, y| {
                    if x != y {
                        s.push(Action::Update(*net, x.0, x.1.clone()));
                    }
                });
            };

            diff(
                &mut changes,
                &table.vec,
                &rib_out.vec,
                |x| *x,
                |s, x| {
                    apply_nums(s, x, table.table.exact(&x).unwrap(), &vec![]);
                },
                |s, x| {
                    apply_nums(s, x, &vec![], rib_out.table.exact(&x).unwrap());
                },
                |s, x, y| {
                    apply_nums(s, x, table.table.exact(&x).unwrap(), rib_out.table.exact(&y).unwrap());
                }
            );
        }

        let changes_empty = changes.is_empty();
        println!("{:?} changes", changes.len());
        let mut buf = [255 as u8; 4096];
        for change in changes {
            let p = match change {
                Action::Update(net, num, attrs) => {
                    rib_out.update_route_compressed(num, net, attrs.clone()).await;
                    BgpUpdateMessage {
                        updates: BgpAddrs::None,
                        withdraws: BgpAddrs::None,
                        attrs: vec![
                            BgpAttrItem::Origin(BgpOrigin {
                                value: match attrs.origin.as_ref().unwrap() {
                                    RouteOrigin::Igp => BgpAttrOrigin::Igp,
                                    RouteOrigin::Egp => BgpAttrOrigin::Egp,
                                    RouteOrigin::Incomplete => BgpAttrOrigin::Incomplete,
                                }
                            }),
                            BgpAttrItem::ASPath(BgpASpath {
                                value: attrs.as_path.as_ref().unwrap().iter().map(|x| BgpAS { value: *x }).collect()
                            }),
                            BgpAttrItem::MPUpdates(BgpMPUpdates {
                                nexthop: match net {
                                    IpNet::V4(v4) => BgpAddr::V4("127.0.0.1".parse().unwrap()),
                                    IpNet::V6(v6) => BgpAddr::V6("::1".parse().unwrap()),
                                },
                                addrs: match net {
                                    IpNet::V4(v4) => BgpAddrs::IPV4U(vec![BgpAddrV4 { addr: v4.addr(), prefixlen: v4.prefix_len() }]),
                                    IpNet::V6(v6) => BgpAddrs::IPV6U(vec![BgpAddrV6 { addr: v6.addr(), prefixlen: v6.prefix_len() }]),
                                },
                            })
                        ],
                    }
                }
                Action::Withdraw(net, num) => {
                    rib_out.withdraw_route(num, net).await;
                    BgpUpdateMessage {
                        updates: BgpAddrs::None,
                        withdraws: BgpAddrs::None,
                        attrs: vec![
                            BgpAttrItem::MPWithdraws(BgpMPWithdraws {
                                addrs: match net {
                                    IpNet::V4(v4) => BgpAddrs::IPV4U(vec![BgpAddrV4 { addr: v4.addr(), prefixlen: v4.prefix_len() }]),
                                    IpNet::V6(v6) => BgpAddrs::IPV6U(vec![BgpAddrV6 { addr: v6.addr(), prefixlen: v6.prefix_len() }]),
                                }
                            })
                        ],
                    }
                }
            };
            let messagelen = match p.encode_to(&params, &mut buf[19..]) {
                Err(e) => {
                    return Err(e.into());
                }
                Ok(sz) => sz,
            };
            let blen = params
                .prepare_message_buf(&mut buf, BgpMessageType::Update, messagelen)?;
            write.lock().await.write_all(&buf[0..blen]).await?;
        }
        println!("{:?} rib out", rib_out.state.lock().unwrap().vec.len());
        if changes_empty {
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_millis(500)) => {}
                //update = stream.next() => match update {
                //    Some(Ok(update)) => {},
                //    Some(Err(Ok(notification))) => {}, //break Ok(notification),
                //    Some(Err(Err(e))) => anyhow::bail!(e),
                //    None => panic!(),
                //}
            }
        }
    }

}
