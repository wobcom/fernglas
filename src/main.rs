mod table;
mod table_impl;
mod bmp_collector;
mod bgpdumper;
mod bgp_collector;
mod api;
mod compressed_attrs;

use serde::Deserialize;
use futures_util::future::{join_all, select_all};
use tokio::signal::unix::{signal, SignalKind};
use log::*;

#[derive(Deserialize)]
#[serde(tag = "collector_type")]
enum CollectorConfig {
    Bmp(bmp_collector::BmpCollectorConfig),
    Bgp(bgp_collector::BgpCollectorConfig),
}

#[derive(Deserialize)]
struct Config {
    collectors: Vec<CollectorConfig>,
    api: api::ApiServerConfig,
}

fn usage(program: &str) -> ! {
    eprintln!("usage: {} <CONFIG>", program);
    std::process::exit(1)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let table: table_impl::InMemoryTable = Default::default();

    let mut args = std::env::args();
    let program = args.next().unwrap();
    let config_path = match args.next() {
        Some(v) => v,
        _ => usage(&program),
    };
    if args.next().is_some() {
        usage(&program);
    }

    let cfg: Config = serde_yaml::from_slice(&tokio::fs::read(&config_path).await?)?;

    let mut futures = vec![];

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    futures.push(tokio::task::spawn(api::run_api_server(cfg.api, table.clone(), shutdown_rx.clone())));

    futures.extend(cfg.collectors.into_iter().map(|collector| {
        match collector {
            CollectorConfig::Bmp(cfg) => tokio::task::spawn(bmp_collector::run(cfg, table.clone(), shutdown_rx.clone())),
            CollectorConfig::Bgp(cfg) => tokio::task::spawn(bgp_collector::run(cfg, table.clone(), shutdown_rx.clone())),
        }
    }));

    let mut sigint = signal(SignalKind::interrupt())?;
    let mut sigterm = signal(SignalKind::terminate())?;
    let res = tokio::select! {
        _ = sigint.recv() => {
            info!("shutting down on signal SIGINT");
            Ok(())
        }
        _ = sigterm.recv() => {
            info!("shutting down on signal SIGTERM");
            Ok(())
        }
        task_ended = select_all(&mut futures) => {
            warn!("shutting down because task unexpectedly ended");
            task_ended.0?
        }
    };
    shutdown_tx.send(true)?;
    join_all(futures).await;
    res
}

