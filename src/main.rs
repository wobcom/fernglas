mod table;

use futures_util::StreamExt;
use std::convert::Infallible;
use hyper::{Body, Response, Server};
use hyper::service::{make_service_fn, service_fn};
use std::net::IpAddr;
use ipnet::{IpNet, Ipv4Net, Ipv6Net};
use tokio_util::codec::length_delimited::LengthDelimitedCodec;
use tokio::net::TcpListener;
use zettabgp::prelude::BgpAddrs;
use zettabgp::prelude::BgpAttrOrigin;
use zettabgp::prelude::BgpOrigin;
use zettabgp::prelude::BgpMED;
use zettabgp::prelude::BgpASpath;
use zettabgp::prelude::BgpNextHop;
use zettabgp::prelude::BgpLargeCommunityList;
use zettabgp::prelude::BgpCommunityList;
use zettabgp::prelude::BgpAddr;
use zettabgp::bmp::BmpMessage;
use zettabgp::prelude::BgpAttrItem;
use table::{Table, TableSelector, InMemoryTable, Route, RouteOrigin, SessionId};

fn bgp_addrs_to_nets(addrs: &BgpAddrs) -> Vec<IpNet> {
    let mut res = vec![];
    match addrs {
        BgpAddrs::IPV4U(ref addrs) => {
            for addr in addrs {
                match Ipv4Net::new(addr.addr, addr.prefixlen) {
                    Ok(net) => res.push(IpNet::V4(net)),
                    Err(e) => eprintln!("invalid BgpAddrs prefixlen"),
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

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let listener = TcpListener::bind("[::]:11019").await?;

    let table: InMemoryTable = Default::default();

    {
        let table = table.clone();

        std::thread::spawn(move || {

            loop {
                let table = table.clone();
                let make_service = make_service_fn(move |_conn| {
                    let table = table.clone();
                    async move {
                        Ok::<_, Infallible>(service_fn(move |req| {
                            let net_str = req.uri().path().chars().skip(1).collect::<String>();
                            let net = net_str.parse().unwrap();
                            let table = table.clone();
                            async move {
                                let resp = {
                                    let mut stream = table.get_routes(net);
                                    let mut res = vec![];
                                    while let Some(next) = stream.next().await {
                                        res.push(next);
                                    }
                                    let data = serde_json::to_string(&res).unwrap();
                                    Response::new(Body::from(data))
                                };
                                Ok::<_, Infallible>(resp)
                            }
                        }))
                    }
                });

                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                let local = tokio::task::LocalSet::new();
                local.spawn_local(async move {
                    let server = Server::bind(&"[::]:3000".parse().unwrap())
                        .executor(LocalExec)
                        .serve(make_service);

                    if let Err(e) = server.await {
                        eprintln!("server error: {}", e);
                    }
                });
                rt.block_on(local);

                println!("Restarting server after error");
            }

        });
    }

    loop {
        let (io, so) = listener.accept().await?;
        println!("connected {:?}", so);

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

                if let BmpMessage::RouteMonitoring(rm) = msg {
                    let session = match (rm.peer.peertype, (rm.peer.flags & 64) != 0) {
                        (0, false) => TableSelector::PrePolicyAdjIn(SessionId {
                            local_router_id: "0.0.0.0".parse().unwrap(), // FIXME
                            remote_router_id: rm.peer.routerid,
                        }),
                        (0, true) => TableSelector::PostPolicyAdjIn(SessionId {
                            local_router_id: "0.0.0.0".parse().unwrap(), // FIXME
                            remote_router_id: rm.peer.routerid,
                        }),
                        (3, _) => TableSelector::LocRib(rm.peer.routerid),
                        _ => {
                            println!("unknown peer type {} flags {:x}", rm.peer.peertype, rm.peer.flags);
                            continue;
                        }
                    };

                    let mut route: Route = Default::default();
                    let mut nexthop = None;
                    let mut update_nets = vec![];
                    let mut withdraw_nets = vec![];
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
                                route.communities = Some(communities);
                            }
                            BgpAttrItem::MED(BgpMED { value }) => {
                                route.med = Some(value);
                            }
                            BgpAttrItem::Origin(BgpOrigin { value }) => {
                                route.origin = Some(match value {
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
                                route.as_path = Some(as_path);
                            }
                            BgpAttrItem::LargeCommunityList(BgpLargeCommunityList { value }) => {
                                let mut communities = vec![];
                                for community in value.into_iter() {
                                    communities.push((community.ga, community.ldp1, community.ldp2));
                                }
                                route.large_communities = Some(communities);
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

                    for (net, nexthop) in update_nets {
                        let mut route = route.clone();
                        route.nexthop = nexthop;
                        table.update_route(net, session.clone(), route).await;
                    }
                    for net in withdraw_nets {
                        table.withdraw_route(net, session.clone()).await;
                    }
                    continue;
                }

                println!("{:#?}", msg);
            }
        });

    }
}


#[derive(Clone, Copy, Debug)]
struct LocalExec;

impl<F> hyper::rt::Executor<F> for LocalExec
where
    F: std::future::Future + 'static, // not requiring `Send`
{
    fn execute(&self, fut: F) {
        // This will spawn into the currently running `LocalSet`.
        tokio::task::spawn_local(fut);
    }
}
