use futures_util::future::{join_all, select_all};
use tokio::signal::unix::{signal, SignalKind};
use fernglas::*;
use log::*;

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let config_path = config_path_from_args();
    let cfg: Config = serde_yaml::from_slice(&tokio::fs::read(&config_path).await?)?;

    let table = table_impl::PostgresTable::new(&cfg.db_uri).await?;

    let mut futures = vec![];

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    // Set up the exporter to collect metrics
    let _exporter = autometrics::global_metrics_exporter();

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

