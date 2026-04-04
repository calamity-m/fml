use color_eyre::Result;
use std::fs;
use tracing::info;

use tracing_subscriber::EnvFilter;

use crate::{app::App, config::Config};

mod app;
mod config;
mod error;
mod event;
mod log;
mod message;
mod state;
mod tui;
pub fn init(
    config: &crate::config::Config,
) -> color_eyre::Result<Option<tracing_appender::non_blocking::WorkerGuard>> {
    if !config.enable_logging {
        return Ok(None);
    }

    fs::create_dir_all(&config.debug_log_dir)?;

    let file_appender =
        tracing_appender::rolling::hourly(&config.debug_log_dir, env!("CARGO_PKG_NAME"));
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    // RUST_LOG controls the level, e.g. RUST_LOG=debug or RUST_LOG=fml=debug
    // Defaults to the configured fallback when RUST_LOG is not set.
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(config.default_log_level.clone()));

    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_env_filter(filter)
        .init();

    info!("debugging enabled");

    Ok(Some(guard))
}

#[tokio::main]
async fn main() -> Result<()> {
    color_eyre::install()?;

    let config = Config::new()?;
    let _guard = init(&config)?;

    info!("config and logging intialized");

    let mut app = App::new(config)?;

    let _ = app.run().await?;

    info!("exiting");

    Ok(())
}
