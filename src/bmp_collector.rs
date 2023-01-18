use futures_util::{StreamExt, pin_mut};
use bitvec::view::BitView;
use bitvec::prelude::Msb0;
use std::net::SocketAddr;
use tokio_util::codec::length_delimited::LengthDelimitedCodec;
use tokio::net::{TcpListener, TcpStream};
use zettabgp::bmp::BmpMessage;
use zettabgp::bmp::prelude::BmpMessageRouteMonitoring;
use zettabgp::bmp::prelude::BmpMessagePeerHeader;
use zettabgp::bmp::prelude::BmpMessageTermination;
use crate::table::{Table, TableSelector, SessionId, Session, Client};
use serde::Deserialize;
use log::*;

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
            trace!("unknown peer type {} flags {:x}", rm.peer.peertype, rm.peer.flags);
            return;
        }
    };

    table.insert_bgp_update(session, rm.update).await;
}

pub async fn run_client(io: TcpStream, client_addr: SocketAddr, table: &impl Table) -> anyhow::Result<BmpMessageTermination> {
    let read = LengthDelimitedCodec::builder()
        .length_field_offset(1)
        .length_field_type::<u32>()
        .num_skip(0)
        .new_read(io)
        .filter_map(|msg| async move {
            let orig_msg = match msg {
                Ok(v) => v,
                Err(e) => {
                    warn!("BMP Codec Error: {:?}", e);
                    return None;
                }
            };
            match BmpMessage::decode_from(&orig_msg[5..]) {
                Ok(v) => Some(v),
                Err(e) => {
                    warn!("BMP Parse Error: {:?}", e);
                    warn!("{:x?}", &orig_msg);
                    None
                }
            }
        });
    pin_mut!(read);
    let init_msg = match read.next().await {
        Some(BmpMessage::Initiation(i)) => i,
        other => {
            anyhow::bail!("expected initiation message, got: {:?}", other);
        }
    };
    let client_name = init_msg.sys_name.unwrap_or(client_addr.ip().to_string());
    table.client_up(client_addr, Client { client_name }).await;

    loop {
        let msg = read.next().await.ok_or(anyhow::anyhow!("unexpected end of stream"))?;

        match msg {
            BmpMessage::RouteMonitoring(rm) => {
                process_route_monitoring(table, client_addr, rm).await;
            }
            BmpMessage::PeerUpNotification(n) => {
                trace!("{} {:?}", client_addr, n);
                if let Some(session_id) = table_selector_for_peer(client_addr, &n.peer).and_then(|table| table.session_id().cloned()) {
                    table.session_up(session_id, Session {}).await;
                }
            }
            BmpMessage::PeerDownNotification(n) => {
                trace!("{} {:?}", client_addr, n);
                if let Some(session_id) = table_selector_for_peer(client_addr, &n.peer).and_then(|table| table.session_id().cloned()) {
                    table.session_down(session_id, None).await;
                }
            }
            BmpMessage::Termination(n) => break Ok(n),
            msg => trace!("unknown message from {} {:#?}", client_addr, msg),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct BmpCollectorConfig {
    pub bind: SocketAddr,
}

pub async fn run(cfg: BmpCollectorConfig, table: impl Table) -> anyhow::Result<()> {
    let listener = TcpListener::bind(cfg.bind).await?;
    loop {
        let (io, client_addr) = listener.accept().await?;
        info!("connected {:?}", client_addr);

        let table = table.clone();
        tokio::spawn(async move {
            match run_client(io, client_addr, &table).await {
                Err(e) => warn!("disconnected {} {}", client_addr, e),
                Ok(notification) => info!("disconnected {} {:?}", client_addr, notification),
            };
            table.client_down(client_addr).await;
        });

    }
}
