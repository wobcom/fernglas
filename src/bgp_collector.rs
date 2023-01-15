use futures_util::StreamExt;
use futures_util::pin_mut;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use std::net::{Ipv4Addr, SocketAddr};
use zettabgp::BgpSessionParams;
use zettabgp::BgpCapability;
use zettabgp::BgpTransportMode;
use zettabgp::prelude::BgpNotificationMessage;
use crate::bgpdumper::BgpDumper;
use crate::table::{Table, TableSelector};
use serde::Deserialize;
use log::*;

pub async fn run_peer(cfg: BgpCollectorConfig, table: impl Table, stream: TcpStream, client_addr: SocketAddr) -> anyhow::Result<BgpNotificationMessage> {
    let mut dumper = BgpDumper::new(
        BgpSessionParams::new(
            cfg.asn,
            180,
            BgpTransportMode::IPv4,
            cfg.router_id,
            vec![
                BgpCapability::SafiIPv4u,
                BgpCapability::SafiIPv6u,
                BgpCapability::CapRR,
//use zettabgp::BgpCapAddPath;
                //BgpCapability::CapAddPath(vec![BgpCapAddPath::new_from_cap(BgpCapability::SafiIPv4u, true, true).unwrap()/*, BgpCapAddPath::new_from_cap(BgpCapability::SafiIPv6u, true, true).unwrap()*/]),
                BgpCapability::CapASN32(cfg.asn),
            ]
            .into_iter()
            .collect(),
        ),
        stream,
    );
    dumper.start_active().await?;
    let stream = dumper.lifecycle();
    pin_mut!(stream);
    loop {
        let update = match stream.next().await {
            Some(Ok(update)) => update,
            Some(Err(Ok(notification))) => break Ok(notification),
            Some(Err(Err(e))) => anyhow::bail!(e),
            None => panic!(),
        };
        table.insert_bgp_update(TableSelector::LocRib { from_client: client_addr }, update).await;
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct BgpCollectorConfig {
    pub asn: u32,
    pub router_id: Ipv4Addr,
    pub bind: SocketAddr,
}

pub async fn run(cfg: BgpCollectorConfig, table: impl Table) -> anyhow::Result<()> {
    let listener = TcpListener::bind(cfg.bind).await?;
    loop {
        let (io, client_addr) = listener.accept().await?;
        info!("connected {:?}", client_addr);

        let table = table.clone();
        let cfg = cfg.clone();
        tokio::spawn(async move {
            match run_peer(cfg.clone(), table.clone(), io, client_addr).await {
                Err(e) => warn!("disconnected {} {}", client_addr, e),
                Ok(notification) => info!("disconnected {} {:?}", client_addr, notification),
            };
            table.clear_router_table(client_addr).await;
        });
    }
}
