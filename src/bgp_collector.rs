use futures_util::StreamExt;
use futures_util::pin_mut;
use tokio::net::TcpListener;
use crate::bgpdumper::BgpDumper;
use tokio::net::TcpStream;
use std::net::SocketAddr;
use zettabgp::BgpSessionParams;
use zettabgp::BgpCapability;
use zettabgp::BgpTransportMode;
use crate::table::{Table, TableSelector};

pub async fn run_peer(cfg: Config, table: impl Table, stream: TcpStream, client_addr: SocketAddr) -> anyhow::Result<()> {
    let mut dumper = BgpDumper::new(
        BgpSessionParams::new(
            cfg.source_asn,
            180,
            BgpTransportMode::IPv4,
            std::net::Ipv4Addr::new(1, 0, 0, 0),
            vec![
                BgpCapability::SafiIPv4u,
                BgpCapability::SafiIPv6u,
                BgpCapability::CapRR,
                BgpCapability::CapASN32(cfg.source_asn),
            ]
            .into_iter()
            .collect(),
        ),
        stream,
    );
    if let Err(e) = dumper.start_active().await {
        anyhow::bail!("{}", e);
    }
    let stream = dumper.lifecycle();
    pin_mut!(stream);
    loop {
        let update = match stream.next().await {
            Some(Ok(update)) => update,
            Some(Err(Ok(notification))) => {
                println!("notification {:?}", notification);
                break Ok(());
            },
            Some(Err(Err(e))) => anyhow::bail!("{}", e),
            None => panic!(),
        };
        table.insert_bgp_update(TableSelector::LocRib { from_client: client_addr }, update).await;
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub source_asn: u32,
}

pub async fn run(table: impl Table) -> anyhow::Result<()> {
    let cfg = Config {
        source_asn: 64519,
    };
    let listener = TcpListener::bind("[::]:179").await?;
    loop {
        let (io, client_addr) = listener.accept().await?;
        eprintln!("connected {:?}", client_addr);

        let table = table.clone();
        let cfg = cfg.clone();
        tokio::spawn(async move {
            if let Err(e) = run_peer(cfg.clone(), table.clone(), io, client_addr).await {
                println!("bgp session error: {}", e);
            }
            table.clear_router_table(client_addr).await;
        });
    }
}
