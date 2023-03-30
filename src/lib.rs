pub mod store;
pub mod store_impl;
pub mod table_impl;
pub mod bmp_collector;
mod bgpdumper;
pub mod bgp_collector;
pub mod api;
mod compressed_attrs;

use serde::Deserialize;

pub fn config_path_from_args() -> String {
    let mut args = std::env::args();
    let program = args.next().unwrap();
    let config_path = match args.next() {
        Some(v) => v,
        _ => usage(&program),
    };
    if args.next().is_some() {
        usage(&program);
    }

    config_path
}

fn usage(program: &str) -> ! {
    eprintln!("usage: {} <CONFIG>", program);
    std::process::exit(1)
}

#[derive(Deserialize, Debug)]
#[serde(tag = "collector_type")]
pub enum CollectorConfig {
    Bmp(bmp_collector::BmpCollectorConfig),
    Bgp(bgp_collector::BgpCollectorConfig),
}

#[derive(Deserialize, Debug)]
pub struct Config {
    pub collectors: Vec<CollectorConfig>,
    pub api: api::ApiServerConfig,
}

