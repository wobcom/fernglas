use futures_util::{StreamExt, pin_mut};
use futures_util::future::join_all;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use std::net::{Ipv4Addr, SocketAddr, IpAddr};
use std::collections::HashMap;
use zettabgp::BgpSessionParams;
use zettabgp::BgpCapability;
use zettabgp::BgpTransportMode;
use zettabgp::BgpCapAddPath;
use zettabgp::prelude::BgpNotificationMessage;
use crate::bgpdumper::BgpDumper;
use crate::table::{Table, TableSelector, Client};
use serde::Deserialize;
use log::*;

pub async fn run_peer(cfg: PeerConfig, table: impl Table, stream: TcpStream, client_addr: SocketAddr) -> anyhow::Result<BgpNotificationMessage> {
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
                BgpCapability::CapAddPath(vec![BgpCapAddPath::new_from_cap(BgpCapability::SafiIPv4u, true, true).unwrap(), BgpCapAddPath::new_from_cap(BgpCapability::SafiIPv6u, true, true).unwrap()]),
                BgpCapability::CapASN32(cfg.asn),
            ]
            .into_iter()
            .collect(),
        ),
        stream,
    );
    let open_message = dumper.start_active().await?;
    let stream = dumper.lifecycle();
    pin_mut!(stream);
    let client_name = cfg.name_override
        .or(open_message.caps.iter().find_map(|x| {
            if let BgpCapability::CapFQDN(hostname, domainname) = x {
                let mut name = hostname.to_string();
                if domainname != "" {
                    name = format!("{}.{}", name, domainname);
                }
                Some(name)
            } else {
                None
            }
        }))
        .unwrap_or(client_addr.ip().to_string());
    table.client_up(client_addr, Client { client_name, ..Default::default() }).await;
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
pub struct PeerConfig {
    pub asn: u32,
    pub router_id: Ipv4Addr,
    pub name_override: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BgpCollectorConfig {
    pub bind: SocketAddr,
    #[serde(default)]
    pub peers: HashMap<IpAddr, PeerConfig>,
    pub default_peer_config: Option<PeerConfig>,
}

pub async fn run(cfg: BgpCollectorConfig, table: impl Table, mut shutdown: tokio::sync::watch::Receiver<bool>) -> anyhow::Result<()> {
    let listener = TcpListener::bind(cfg.bind).await?;
    let mut running_tasks = vec![];
    loop {
        tokio::select! {
            new_conn = listener.accept() => {
                let (io, client_addr) = new_conn?;
                info!("connected {:?}", client_addr);

                if let Some(peer_cfg) = cfg.peers.get(&client_addr.ip()).or(cfg.default_peer_config.as_ref()).cloned() {
                    let table = table.clone();
                    let mut shutdown = shutdown.clone();
                    running_tasks.push(tokio::spawn(async move {
                        tokio::select! {
                            res = run_peer(peer_cfg, table.clone(), io, client_addr) => {
                                match res {
                                    Err(e) => warn!("disconnected {} {}", client_addr, e),
                                    Ok(notification) => info!("disconnected {} {:?}", client_addr, notification),
                                }
                            }
                            _ = shutdown.changed() => {
                            }
                        };
                        table.client_down(client_addr).await;
                    }));
                } else {
                    info!("unexpected connection from {}", client_addr);
                }
            }
            _ = shutdown.changed() => {
                join_all(running_tasks).await;
                break Ok(());
            }
        }
    }
}
