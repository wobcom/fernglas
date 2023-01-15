mod table;
mod table_impl;
mod bmp_collector;
mod bgpdumper;
mod bgp_collector;
mod api;

use serde::Deserialize;
use futures_util::future::select_all;
use std::pin::Pin;
use std::future::Future;

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

    let mut futures: Vec<Pin<Box<dyn Future<Output = anyhow::Result<()>>>>> = vec![];

    futures.push(Box::pin(api::run_api_server(cfg.api, table.clone())));

    futures.extend(cfg.collectors.into_iter().map(|collector| -> Pin<Box<dyn Future<Output = anyhow::Result<()>>>> {
        match collector {
            CollectorConfig::Bmp(cfg) => Box::pin(bmp_collector::run(cfg, table.clone())),
            CollectorConfig::Bgp(cfg) => Box::pin(bgp_collector::run(cfg, table.clone())),
        }
    }));

    select_all(futures).await.0
}

