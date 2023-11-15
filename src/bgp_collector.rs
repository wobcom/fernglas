use crate::bgpdumper::BgpDumper;
use crate::store::{Client, RouteState, Store, TableSelector};
use futures_util::future::join_all;
use futures_util::{pin_mut, StreamExt};
use log::*;
use serde::Deserialize;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use zettabgp::prelude::BgpNotificationMessage;
use zettabgp::BgpCapAddPath;
use zettabgp::BgpCapability;
use zettabgp::BgpSessionParams;
use zettabgp::BgpTransportMode;

pub async fn run_peer(
    cfg: PeerConfig,
    store: impl Store,
    stream: TcpStream,
    client_addr: SocketAddr,
) -> anyhow::Result<BgpNotificationMessage> {
    let mut caps = vec![
        BgpCapability::SafiIPv4u,
        BgpCapability::SafiIPv6u,
        BgpCapability::CapRR,
        BgpCapability::CapASN32(cfg.asn),
    ];
    if cfg.add_path {
        caps.push(BgpCapability::CapAddPath(vec![
            BgpCapAddPath::new_from_cap(BgpCapability::SafiIPv4u, true, true).unwrap(),
            BgpCapAddPath::new_from_cap(BgpCapability::SafiIPv6u, true, true).unwrap(),
        ]));
    }

    let mut dumper = BgpDumper::new(
        BgpSessionParams::new(cfg.asn, 180, BgpTransportMode::IPv4, cfg.router_id, caps),
        stream,
    );
    let open_message = dumper.start_active().await?;
    let stream = dumper.lifecycle();
    pin_mut!(stream);
    let client_name = cfg
        .name_override
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
    store
        .client_up(
            client_addr,
            cfg.route_state,
            Client {
                client_name,
                ..Default::default()
            },
        )
        .await;
    loop {
        let update = match stream.next().await {
            Some(Ok(update)) => update,
            Some(Err(Ok(notification))) => break Ok(notification),
            Some(Err(Err(e))) => anyhow::bail!(e),
            None => panic!(),
        };
        store
            .insert_bgp_update(
                TableSelector::LocRib {
                    from_client: client_addr,
                    route_state: cfg.route_state,
                },
                update,
            )
            .await;
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PeerConfig {
    pub asn: u32,
    pub router_id: Ipv4Addr,
    pub name_override: Option<String>,
    pub route_state: RouteState,
    pub add_path: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BgpCollectorConfig {
    pub bind: SocketAddr,
    #[serde(default)]
    pub peers: HashMap<IpAddr, PeerConfig>,
    pub default_peer_config: Option<PeerConfig>,
}

pub async fn run(
    cfg: BgpCollectorConfig,
    store: impl Store,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(cfg.bind).await?;
    let mut running_tasks = vec![];
    loop {
        tokio::select! {
            new_conn = listener.accept() => {
                let (io, client_addr) = new_conn?;
                info!("connected {:?}", client_addr);

                if let Some(peer_cfg) = cfg.peers.get(&client_addr.ip()).or(cfg.default_peer_config.as_ref()).cloned() {
                    let store = store.clone();
                    let mut shutdown = shutdown.clone();
                    running_tasks.push(tokio::spawn(async move {
                        tokio::select! {
                            res = run_peer(peer_cfg, store.clone(), io, client_addr) => {
                                match res {
                                    Err(e) => warn!("disconnected {} {}", client_addr, e),
                                    Ok(notification) => info!("disconnected {} {:?}", client_addr, notification),
                                }
                            }
                            _ = shutdown.changed() => {
                            }
                        };
                        store.client_down(client_addr).await;
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
