use clap::Parser;
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
mod producer;
mod search;
mod state;
mod store;
mod tui;

#[derive(Parser, Debug)]
#[command(author, version = version(), about)]
pub struct Cli {
    /// Tick rate, i.e. number of ticks per second
    #[arg(short, long)]
    pub tick_rate: Option<f64>,

    /// Frame rate, i.e. number of frames per second
    #[arg(short, long)]
    pub frame_rate: Option<f64>,

    /// Enable debug mode, which will enable logging output
    #[arg(short, long)]
    pub debug: bool,

    /// Start the app with a synthetic demo log source
    #[arg(long)]
    pub demo: bool,

    /// Override the configured TUI theme name
    #[arg(long)]
    pub theme: Option<String>,
}

pub fn version() -> String {
    format!(
        "\n
Version: {}
Build Date: {}

Config Dir: {}
",
        env!("CARGO_PKG_VERSION"),
        env!("VERGEN_BUILD_DATE"),
        config::default_config_dir()
    )
}

impl Cli {
    pub fn init(&self) -> Result<(Config, Option<tracing_appender::non_blocking::WorkerGuard>)> {
        let mut config = Config::new()?;

        if self.debug {
            config.enable_logging = true;
        }

        if let Some(frame_rate) = self.frame_rate {
            config.tui.frame_rate = frame_rate;
        }

        if let Some(theme) = &self.theme {
            config.tui.theme = theme.clone();
        }

        let guard = init(&config)?;

        Ok((config, guard))
    }
}
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
    // Setup eyre panic hook
    color_eyre::install()?;

    // Parse our CLI
    let cli = Cli::try_parse()?;

    // Init the CLI to retrieve our config and tracing guard
    let (config, _guard) = cli.init()?;
    info!("config and logging intialized");

    // Create the app and run it
    let app = App::new(config)?;
    app.run().await?;
    info!("exiting");

    Ok(())
}
