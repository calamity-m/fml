use thiserror::Error;

#[derive(Error, Debug)]
pub enum FmlError {
    #[error("cli error: {0}")]
    Cli(#[from] clap::Error),

    #[error("app error: {0}")]
    App(String),

    #[error("error with loading config: {0}")]
    Config(#[from] config::ConfigError),

    #[error("io error occured: {0}")]
    Io(#[from] std::io::Error),

    #[error("store error: {0}")]
    Store(String),

    #[error("search error: {0}")]
    Search(String),

    #[error("theme error: {0}")]
    Theme(String),

    #[error("ingest error: {0}")]
    Ingest(String),

    #[error("keybinding error: {0}")]
    Keybinding(String),
}
