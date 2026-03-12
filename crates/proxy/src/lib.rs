use conduit_core::Config;

pub fn start(config: Config) -> anyhow::Result<()> {
    tracing::debug!(?config, "here it is!");
    Ok(())
}
