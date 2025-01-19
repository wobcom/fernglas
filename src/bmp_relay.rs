use crate::compressed_attrs::decompress_route_attrs;
use crate::store::make_bgp_withdraw;
use crate::store::TableSelector;
use crate::store_impl::InMemoryStore;
use crate::table_impl::Action;
use crate::table_stream::table_stream;
use futures_util::pin_mut;
use futures_util::StreamExt;
use serde::Deserialize;
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use zettabgp::bmp::prelude::*;
use zettabgp::prelude::*;

#[derive(Debug, Deserialize)]
pub struct RelayConfig {
    table: TableSelector,
    monitoring_station: SocketAddr,
    router_id: Ipv4Addr,
    asn: u32,
}

async fn run_(cfg: RelayConfig, store: InMemoryStore) -> ! {
    let table = store.get_table(cfg.table);
    let mut buf = [0; 10000];
    'outer: loop {
        let updates_stream = table_stream(&table);
        pin_mut!(updates_stream);

        let mut tcp_stream = match tokio::net::TcpStream::connect(cfg.monitoring_station).await {
            Err(_) => {
                log::info!("trying to connect {}", cfg.monitoring_station);
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue 'outer;
            }
            Ok(v) => v,
        };
        log::info!("connected {}", cfg.monitoring_station);

        let fake_open_message = BgpOpenMessage {
            as_num: cfg.asn,
            caps: vec![
                BgpCapability::SafiIPv4u,
                BgpCapability::SafiIPv6u,
                BgpCapability::SafiVPNv4u,
                BgpCapability::SafiVPNv6u,
                BgpCapability::CapRR,
                BgpCapability::CapASN32(cfg.asn),
            ],
            hold_time: 0,
            router_id: cfg.router_id,
        };
        let peer_hdr = BmpMessagePeerHeader {
            peertype: 3,
            flags: 0,
            peerdistinguisher: BgpRD::new(0, 0),
            peeraddress: "::".parse().unwrap(),
            asnum: cfg.asn,
            routerid: cfg.router_id,
            timestamp: 0,
        };
        let mut bmp_messages = futures_util::stream::iter([
            BmpMessage::Initiation(BmpMessageInitiation {
                str0: None,
                sys_descr: None,
                sys_name: None,
            }),
            BmpMessage::PeerUpNotification(BmpMessagePeerUp {
                peer: peer_hdr.clone(),
                localaddress: "::".parse().unwrap(),
                localport: 0,
                remoteport: 0,
                msg1: fake_open_message.clone(),
                msg2: fake_open_message,
            }),
        ])
        .chain(updates_stream.map(|action| {
            let update = match action {
                (net, num, Action::Withdraw) => {
                    if num != 0 {
                        //log::warn!("add-paths table is not yet implemented");
                    }
                    make_bgp_withdraw(net)
                }
                (net, num, Action::Update(attrs)) => {
                    if num != 0 {
                        //log::warn!("add-paths table is not yet implemented");
                    }
                    decompress_route_attrs(&attrs).to_bgp_update(net)
                }
            };

            BmpMessage::RouteMonitoring(BmpMessageRouteMonitoring {
                peer: peer_hdr.clone(),
                update,
            })
        }));

        'inner: while let Some(bmp_msg) = bmp_messages.next().await {
            log::trace!("sending message {}: {:?}", cfg.monitoring_station, bmp_msg);
            let mut len = 0;
            match bmp_msg.encode_to(&mut buf[5..]) {
                Ok(i) => len += i,
                Err(e) => {
                    log::warn!("error encoding BMP message {:?}: {}", bmp_msg, e);
                    continue 'inner;
                }
            }
            let msg_hdr = BmpMessageHeader {
                version: 3,
                msglength: len + 5,
            };
            len += msg_hdr.encode_to(&mut buf).unwrap();

            if let Err(e) = tcp_stream.write_all(&buf[..len]).await {
                log::warn!(
                    "resetting connection {:?}, reason: {}",
                    cfg.monitoring_station,
                    e
                );
                continue 'outer;
            }
        }
    }
}

pub async fn run(
    cfg: RelayConfig,
    store: InMemoryStore,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<()> {
    tokio::select! {
        _ = run_(cfg, store) => unreachable!(),
        _ = shutdown.changed() => Ok(()),
    }
}
