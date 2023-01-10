use futures_util::StreamExt;
use std::convert::Infallible;
use hyper::{Body, Response, Server};
use hyper::service::{make_service_fn, service_fn};
use std::sync::Mutex;
use std::net::IpAddr;
use std::sync::Arc;
use ipnet::{IpNet, Ipv4Net, Ipv6Net};
use std::collections::HashMap;
use tokio_util::codec::length_delimited::LengthDelimitedCodec;
use tokio::net::TcpListener;
use zettabgp::bmp::BmpMessage;
use zettabgp::prelude::BgpAttrItem;

#[derive(Debug, PartialEq)]
enum RouteStatus {
    Seen,
    Accepted,
    Selected,
}

#[derive(Debug)]
struct Route {
    attrs: Vec<BgpAttrItem>,
    status: RouteStatus,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let listener = TcpListener::bind("[::]:11019").await?;

    let (tx, mut rx) = tokio::sync::mpsc::channel(10_000_000);

    let map: Arc<Mutex<HashMap<IpNet, HashMap<IpAddr, Vec<Arc<Route>>>>>> = Default::default();
    {
        let map = map.clone();
        tokio::spawn(async move {
            loop {
                // simple version, but re-locks the map for each route
                /*
                let (net, peeraddr, route) = rx.recv().await.unwrap();
                let mut locked = map.lock().unwrap();
                let map2 = locked.entry(net).or_insert(HashMap::new());
                let e = map2.entry(peeraddr).or_insert(Vec::new());
                e.push(route);
                */

                let mut next_route = rx.recv().await;

                while let Some((net, peeraddr, route)) = next_route.take() {
                    let mut locked = map.lock().unwrap();
                    let map2 = locked.entry(net).or_insert(HashMap::new());
                    let e = map2.entry(peeraddr).or_insert(Vec::new());
                    e.push(route);

                    next_route = rx.try_recv().ok()
                }
                
            }
        });
    }
    {
        let map = map.clone();

        tokio::spawn(async move {
            let make_service = make_service_fn(|_conn| {
                let map = map.clone();
                async move {
                    Ok::<_, Infallible>(service_fn(move |req| {
                        let net_str = req.uri().path().chars().skip(1).collect::<String>();
                        let net = net_str.parse().unwrap();
                        let resp = {
                            let data = format!("{:#?}", &map.lock().unwrap().get(&net));
                            Response::new(Body::from(data))
                        };
                        async move {
                            Ok::<_, Infallible>(resp)
                        }
                    }))
                }
            });

            let server = Server::bind(&"[::]:3000".parse().unwrap()).serve(make_service);

            if let Err(e) = server.await {
                eprintln!("server error: {}", e);
            }

        });
    }

    loop {
        let (io, _) = listener.accept().await?;

        let tx = tx.clone();
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
                    let status = match (rm.peer.peertype, (rm.peer.flags & 128) != 0) {
                        (0, false) => RouteStatus::Seen,
                        (0, true) => RouteStatus::Accepted,
                        (3, _) => RouteStatus::Selected,
                        _ => continue,
                    };
                    if status == RouteStatus::Selected {
                        println!("{:?}, {:#?}", status, rm);
                    }
                    let route = Arc::new(Route {
                        attrs: rm.update.attrs,
                        status,
                    });

/*                                                                                       
Accepted, BmpMessageRouteMonitoring {                                                          
    peer: BmpMessagePeerHeader {                                                               
        peertype: 0,                                                                           
        flags: 128,                                                                            
        peerdistinguisher: BgpRD {                                                             
            rdh: 0,                                                                            
            rdl: 0,                                                                            
        },                                                                                     
        peeraddress: 2a0f:4ac4:80:f000::20:7671,                                               
        asnum: 207671,                                                                         
        routerid: 1.2.3.5,                                                                     
        timestamp: 7186787386490350546,                                                        
    },                                                                                         
    update: BgpUpdateMessage {                                                                 
        updates: IPV6U(                                                                        
            [],                                                                                
        ),                                                                                     
        withdraws: IPV6U(                                                                      
            [],                                                                                
        ),                                                                                     
        attrs: [                                                                               
            MPUpdates(                                                                         
                BgpMPUpdates {                                                                 
                    nexthop: V6(                                                               
                        2a0f:4ac4:80:f000::20:7671,                                            
                    ),                                                                         
                    addrs: IPV6U(                                                              
                        [                                                                      
                            BgpAddrV6 {                                                        
                                addr: 2a0e:b105:530::,                                         
                                prefixlen: 48,                                                 
                            },                                                                 
                        ],                                                                     
                    ),                                                                         
                },                                                                             
            ),                   */
/*
                    let mut remaining_attrs = vec![];
                    for attr in rm.attrs {
					    if BgpAttrItem::MPUpdates(updates) = attr {

                        } else {
                            remaining_attrs.push(attr);
                        }
					}*/
                    match rm.update.updates {
                        zettabgp::afi::BgpAddrs::IPV4U(ref addrs) => {
                            for addr in addrs {
                                let net = IpNet::V4(Ipv4Net::new(addr.addr, addr.prefixlen).unwrap());
                                if tx.try_send((net, rm.peer.peeraddress.clone(), route.clone())).is_err() {
                                    eprintln!("too slow! consider increasing buffer size");
                                }
                            }
                        },
                        zettabgp::afi::BgpAddrs::IPV6U(ref addrs) => {
                            for addr in addrs {
                                let net = IpNet::V6(Ipv6Net::new(addr.addr, addr.prefixlen).unwrap());
                                if tx.try_send((net, rm.peer.peeraddress.clone(), route.clone())).is_err() {
                                    eprintln!("too slow! consider increasing buffer size");
                                }
                            }
                        },
                        _ => continue,
                    }
                    continue;
                }

                println!("{:#?}", msg);
            }
        });

//        RouteMonitoring(BmpMessageRouteMonitoring { peer: BmpMessagePeerHeader { peertype: 0, flags: 0, peerdistinguisher: BgpRD { rdh: 0, rdl: 0 }, peeraddress: 192.168.56.2, asnum: 207671, routerid: 1.2.3.5, timestamp: 7186750183483599786 }, update: BgpUpdateMessage { updates: IPV4U([BgpAddrV4 { addr: 124.150.32.0, prefixlen: 19 }]), withdraws: IPV4U([]), attrs: [Origin(BgpOrigin { value: Igp }), ASPath(BgpASpath { value: [BgpAS { value: 207671 }, BgpAS { value: 50629 }, BgpAS { value: 1299 }, BgpAS { value: 7545 }, BgpAS { value: 7545 }, BgpAS { value: 7545 }, BgpAS { value: 7545 }] }), NextHop(BgpNextHop { value: 192.168.56.2 }), LocalPref(BgpLocalpref { value: 100 }), CommunityList(BgpCommunityList { value: {Community { value: 85166264 }, Community { value: 3318022324 }, Community { value: 3318022449 }, Community { value: 3318023164 }, Community { value: 3318032145 }, Community { value: 3318032248 }, Community { value: 3318032352 }} })] } })
//        RouteMonitoring(BmpMessageRouteMonitoring { peer: BmpMessagePeerHeader { peertype: 0, flags: 0, peerdistinguisher: BgpRD { rdh: 0, rdl: 0 }, peeraddress: 192.168.56.2, asnum: 207671, routerid: 1.2.3.5, timestamp: 7186750183483599786 }, update: BgpUpdateMessage { updates: IPV4U([BgpAddrV4 { addr: 14.202.44.0, prefixlen: 24 }]), withdraws: IPV4U([]), attrs: [Origin(BgpOrigin { value: Igp }), ASPath(BgpASpath { value: [BgpAS { value: 207671 }, BgpAS { value: 50629 }, BgpAS { value: 1299 }, BgpAS { value: 7545 }, BgpAS { value: 7545 }, BgpAS { value: 7545 }, BgpAS { value: 7545 }] }), NextHop(BgpNextHop { value: 192.168.56.2 }), LocalPref(BgpLocalpref { value: 100 }), CommunityList(BgpCommunityList { value: {Community { value: 85166264 }, Community { value: 3318022324 }, Community { value: 3318022449 }, Community { value: 3318023164 }, Community { value: 3318032145 }, Community { value: 3318032248 }, Community { value: 3318032352 }} })] } })

    }
}
