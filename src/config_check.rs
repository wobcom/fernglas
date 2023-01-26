use fernglas::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let config_path = config_path_from_args();
    let _cfg: Config = serde_yaml::from_slice(&tokio::fs::read(&config_path).await?)?;

    Ok(())
}
