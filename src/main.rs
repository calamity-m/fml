use clap::Parser;

#[derive(Parser)]
#[command(name = "fml", about = "Feed Me Logs — terminal log triage")]
struct Cli {
    /// Write debug logs to /tmp/fml-debug.log (tail -f to inspect).
    #[arg(long)]
    debug: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if cli.debug {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/fml-debug.log")?;
        tracing_subscriber::fmt()
            .with_writer(std::sync::Mutex::new(file))
            .with_ansi(false)
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_env("RUST_LOG")
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("debug")),
            )
            .init();
        tracing::info!("fml debug log started — tail -f /tmp/fml-debug.log");
    }

    fml_tui::run()
}
