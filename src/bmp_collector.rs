use futures_util::StreamExt;
use bitvec::view::BitView;
use bitvec::prelude::Msb0;
use std::net::IpAddr;
use std::net::SocketAddr;
use ipnet::{IpNet, Ipv4Net, Ipv6Net};
use tokio_util::codec::length_delimited::LengthDelimitedCodec;
use tokio::net::TcpListener;
use zettabgp::prelude::BgpAddrs;
use zettabgp::prelude::BgpAttrOrigin;
use zettabgp::prelude::BgpOrigin;
use zettabgp::prelude::BgpMED;
use zettabgp::prelude::BgpLocalpref;
use zettabgp::prelude::BgpASpath;
use zettabgp::prelude::BgpNextHop;
use zettabgp::prelude::BgpLargeCommunityList;
use zettabgp::prelude::BgpCommunityList;
use zettabgp::prelude::BgpAddr;
use zettabgp::bmp::BmpMessage;
use zettabgp::bmp::prelude::BmpMessageRouteMonitoring;
use zettabgp::bmp::prelude::BmpMessagePeerHeader;
use zettabgp::prelude::BgpAttrItem;
use crate::table::{Table, TableSelector, RouteAttrs, RouteOrigin, SessionId};

fn bgp_addrs_to_nets(addrs: &BgpAddrs) -> Vec<IpNet> {
    let mut res = vec![];
    match addrs {
        BgpAddrs::IPV4U(ref addrs) => {
            for addr in addrs {
                match Ipv4Net::new(addr.addr, addr.prefixlen) {
                    Ok(net) => res.push(IpNet::V4(net)),
                    Err(_) => eprintln!("invalid BgpAddrs prefixlen"),
                }
            }
        }
        BgpAddrs::IPV6U(ref addrs) => {
            for addr in addrs {
                res.push(IpNet::V6(Ipv6Net::new(addr.addr, addr.prefixlen).unwrap()));
            }
        }
        _ => {}
    }
    res
}

fn table_selector_for_peer(client_addr: SocketAddr, peer: &BmpMessagePeerHeader) -> Option<TableSelector> {
    match (peer.peertype, peer.flags.view_bits::<Msb0>()[7]) {
        (0, false) => Some(TableSelector::PrePolicyAdjIn(SessionId {
            from_client: client_addr,
            peer_address: peer.peeraddress,
        })),
        (0, true) => Some(TableSelector::PostPolicyAdjIn(SessionId {
            from_client: client_addr,
            peer_address: peer.peeraddress,
        })),
        (3, _) => Some(TableSelector::LocRib { from_client: client_addr }),
        _ => None,
    }
}

async fn process_route_monitoring(table: &impl Table, client_addr: SocketAddr, rm: BmpMessageRouteMonitoring) {
    let session = match table_selector_for_peer(client_addr, &rm.peer) {
        Some(session) => session,
        None => {
            eprintln!("unknown peer type {} flags {:x}", rm.peer.peertype, rm.peer.flags);
            return;
        }
    };

    let mut attrs: RouteAttrs = Default::default();
    let mut nexthop = None;
    let mut update_nets = vec![];
    let mut withdraw_nets = vec![];
    let log = format!("{:#?}", rm);
    for attr in rm.update.attrs {
        match attr {
            BgpAttrItem::MPUpdates(updates) => {
                let nexthop = match updates.nexthop {
                    BgpAddr::V4(v4) => Some(IpAddr::from(v4)),
                    BgpAddr::V6(v6) => Some(IpAddr::from(v6)),
                    _ => None,
                };
                for net in bgp_addrs_to_nets(&updates.addrs) {
                    update_nets.push((net, nexthop));
                }
            }
            BgpAttrItem::MPWithdraws(withdraws) => {
                for net in bgp_addrs_to_nets(&withdraws.addrs) {
                    withdraw_nets.push(net);
                }
            }
            BgpAttrItem::NextHop(BgpNextHop { value }) => {
                nexthop = Some(value);
            }
            BgpAttrItem::CommunityList(BgpCommunityList { value }) => {
                let mut communities = vec![];
                for community in value.into_iter() {
                    communities.push(((community.value >> 16) as u16, (community.value & 0xff) as u16));
                }
                attrs.communities = Some(communities);
            }
            BgpAttrItem::MED(BgpMED { value }) => {
                attrs.med = Some(value);
            }
            BgpAttrItem::LocalPref(BgpLocalpref { value }) => {
                attrs.local_pref = Some(value);
            }
            BgpAttrItem::Origin(BgpOrigin { value }) => {
                attrs.origin = Some(match value {
                    BgpAttrOrigin::Igp => RouteOrigin::Igp,
                    BgpAttrOrigin::Egp => RouteOrigin::Egp,
                    BgpAttrOrigin::Incomplete => RouteOrigin::Incomplete,
                })
            }
            BgpAttrItem::ASPath(BgpASpath { value }) => {
                let mut as_path = vec![];
                for asn in value {
                    as_path.push(asn.value);
                }
                attrs.as_path = Some(as_path);
            }
            BgpAttrItem::LargeCommunityList(BgpLargeCommunityList { value }) => {
                let mut communities = vec![];
                for community in value.into_iter() {
                    communities.push((community.ga, community.ldp1, community.ldp2));
                }
                attrs.large_communities = Some(communities);
            }
            _ => {},
        }
    }
    for net in bgp_addrs_to_nets(&rm.update.updates).into_iter() {
        update_nets.push((net, nexthop));
    }
    for net in bgp_addrs_to_nets(&rm.update.withdraws).into_iter() {
        withdraw_nets.push(net);
    }

    if update_nets.len() == 0 && withdraw_nets.len() == 0 {
        println!("{}", log);
    }
    for (net, nexthop) in update_nets {
        let mut attrs = attrs.clone();
        attrs.nexthop = nexthop;
        table.update_route(net, session.clone(), attrs).await;
    }
    for net in withdraw_nets {
        table.withdraw_route(net, session.clone()).await;
    }
}

pub async fn run(table: impl Table) -> anyhow::Result<()> {
    let listener = TcpListener::bind("[::]:11019").await?;
    loop {
        let (io, client_addr) = listener.accept().await?;
        eprintln!("connected {:?}", client_addr);

        let table = table.clone();
        tokio::spawn(async move {
            let mut read = LengthDelimitedCodec::builder()
                .length_field_offset(1)
                .length_field_type::<u32>()
                .num_skip(0)
                .new_read(io);
            while let Some(msg) = read.next().await {
                let orig_msg = match msg {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("BMP Codec Error: {:?}", e);
                        continue;
                    }
                };
                let msg = match BmpMessage::decode_from(&orig_msg[5..]) {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("BMP Parse Error: {:?}", e);
                        eprintln!("{:x?}", &orig_msg);
                        continue;
                    }
                };

                match msg {
                    BmpMessage::RouteMonitoring(rm) => {
                        process_route_monitoring(&table, client_addr, rm).await;
                    }
                    BmpMessage::PeerUpNotification(n) => {
                        println!("{} {:#?}", client_addr, n);
                    }
                    BmpMessage::PeerDownNotification(n) => {
                        let session = match table_selector_for_peer(client_addr, &n.peer) {
                            Some(TableSelector::PrePolicyAdjIn(session)) => session,
                            _ => {
                                eprintln!("could not process peer down for peer type {} flags {:x}", n.peer.peertype, n.peer.flags);
                                continue;
                            }
                        };
                        table.clear_peer_table(session).await;
                    }
                    BmpMessage::Termination(n) => {
                        println!("{} {:#?}", client_addr, n);
                        break;
                    }
                    msg => println!("{} {:#?}", client_addr, msg),
                }
            }

            table.clear_router_table(client_addr).await;
        });

    }
}
