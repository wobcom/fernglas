use crate::compressed_attrs::decompress_route_attrs;
use crate::store::make_bgp_withdraw;
use crate::store::TableSelector;
use crate::store_impl::InMemoryStore;
use crate::table_impl::Action;
use crate::table_impl::InMemoryTable;
use crate::table_stream::table_stream;
use futures_util::pin_mut;
use futures_util::StreamExt;
use serde::Deserialize;
use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpSocket;
use tokio::net::TcpStream;
use zettabgp::bmp::prelude::*;
use zettabgp::prelude::*;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayConfig {
    table: TableSelector,

    /// For LocRIB fake BGP open message
    router_id: Ipv4Addr,
    asn: u32,

    monitoring_station: SocketAddr,

    bind_addr: Option<IpAddr>,
    bind_port: Option<u16>,
}

async fn connect(cfg: &RelayConfig) -> std::io::Result<TcpStream> {
    let (sock, default_bind_addr) = match cfg.monitoring_station.ip() {
        IpAddr::V4(_) => (TcpSocket::new_v4()?, "0.0.0.0".parse().unwrap()),
        IpAddr::V6(_) => (TcpSocket::new_v6()?, "::".parse().unwrap()),
    };
    let bind_addr = SocketAddr::new(
        cfg.bind_addr.unwrap_or(default_bind_addr),
        cfg.bind_port.unwrap_or(0),
    );
    sock.bind(bind_addr)?;
    sock.connect(cfg.monitoring_station).await
}

async fn run_connection(cfg: &RelayConfig, table: &InMemoryTable, mut tcp_stream: TcpStream) {
    let mut buf = [0; 10000];
    let updates_stream = table_stream(table);
    pin_mut!(updates_stream);

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
                    log::warn!("add-paths table is not yet implemented");
                }
                make_bgp_withdraw(net)
            }
            (net, num, Action::Update(attrs)) => {
                if num != 0 {
                    log::warn!("add-paths table is not yet implemented");
                }
                decompress_route_attrs(&attrs).to_bgp_update(net)
            }
        };

        BmpMessage::RouteMonitoring(BmpMessageRouteMonitoring {
            peer: peer_hdr.clone(),
            update,
        })
    }));

    while let Some(bmp_msg) = bmp_messages.next().await {
        log::trace!("sending message {}: {:?}", cfg.monitoring_station, bmp_msg);
        let mut len = 0;
        match bmp_msg.encode_to(&mut buf[5..]) {
            Ok(i) => len += i,
            Err(e) => {
                log::warn!("error encoding BMP message {:?}: {}", bmp_msg, e);
                continue;
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
            return;
        }
    }
}

async fn run_(cfg: RelayConfig, store: InMemoryStore) -> ! {
    let table = store.get_table(cfg.table.clone());
    loop {
        let tcp_stream = match connect(&cfg).await {
            Err(_) => {
                log::info!("trying to connect {}", cfg.monitoring_station);
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
            Ok(v) => v,
        };
        log::info!("connected {}", cfg.monitoring_station);

        run_connection(&cfg, &table, tcp_stream).await;
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
