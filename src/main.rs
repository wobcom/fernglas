use fernglas::*;
use figment::providers::{Env, Format, Yaml};
use figment::Figment;
use futures_util::future::{join_all, select_all};
use log::*;
use tokio::signal::unix::{signal, SignalKind};

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let mut figment = Figment::new();
    if let Some(config_path) = config_path_from_args() {
        figment = figment.merge(Yaml::file(config_path));
    }
    figment = figment.merge(Env::prefixed("FERNGLAS_").split("__"));

    let cfg: Config = figment.extract()?;

    trace!("config: {:#?}", &cfg);

    if cfg.config_check {
        std::process::exit(0);
    }

    let store: store_impl::InMemoryStore = Default::default();

    let mut futures = vec![];

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Set up the exporter to collect metrics
    let _exporter = autometrics::global_metrics_exporter();

    futures.push(tokio::task::spawn(api::run_api_server(
        cfg.api,
        store.clone(),
        shutdown_rx.clone(),
    )));

    futures.extend(
        cfg.collectors
            .into_iter()
            .map(|(_, collector)| match collector {
                CollectorConfig::Bmp(cfg) => {
                    tokio::task::spawn(bmp_collector::run(cfg, store.clone(), shutdown_rx.clone()))
                }
                CollectorConfig::Bgp(cfg) => {
                    tokio::task::spawn(bgp_collector::run(cfg, store.clone(), shutdown_rx.clone()))
                }
            }),
    );

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
        (result, idx, _) = select_all(&mut futures) => {
            warn!("shutting down because task unexpectedly ended");
            futures.remove(idx);
            result?
        }
    };
    shutdown_tx.send(true)?;
    join_all(futures).await;
    res
}
