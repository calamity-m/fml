use thiserror::Error;

#[derive(Error, Debug)]
pub enum FmlError {
    #[error("cli error: {0}")]
    CliError(#[from] clap::Error),

    #[error("app error: {0}")]
    AppError(String),

    #[error("error with loading config: {0}")]
    ConfigError(#[from] config::ConfigError),

    #[error("io error occured: {0}")]
    IoError(#[from] std::io::Error),

    #[error("store error: {0}")]
    StoreError(String),

    #[error("search error: {0}")]
    SearchError(String),

    #[error("theme error: {0}")]
    ThemeError(String),

    #[error("ingest error: {0}")]
    IngestError(String),

    #[error("keybinding error: {0}")]
    KeybindingError(String),
}
