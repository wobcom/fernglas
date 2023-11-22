pub mod api;
pub mod bgp_collector;
mod bgpdumper;
pub mod bmp_collector;
mod compressed_attrs;
pub mod store;
pub mod store_impl;
pub mod table_impl;

use serde::Deserialize;
use std::collections::HashMap;

pub fn config_path_from_args() -> Option<String> {
    let mut args = std::env::args().skip(1);
    let config_path = args.next();
    if args.next().is_some() {
        usage();
    }

    config_path
}

pub fn usage() -> ! {
    let program = std::env::args().next().unwrap();
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
    pub collectors: HashMap<String, CollectorConfig>,
    pub api: api::ApiServerConfig,
    /// Only check config and exit
    #[serde(default)]
    pub config_check: bool,
}
