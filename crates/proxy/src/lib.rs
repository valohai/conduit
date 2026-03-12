pub fn greeting() -> &'static str {
    tracing::trace!("greeting at trace level");
    tracing::debug!("greeting at debug level");
    tracing::info!("greeting at info level");
    tracing::warn!("greeting at warn level");
    tracing::error!("greeting at error level");
    conduit_core::GREETING
}
