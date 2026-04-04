use std::io::stdout;

use crossterm::{
    ExecutableCommand as _,
    event::EnableMouseCapture,
    terminal::{EnterAlternateScreen, enable_raw_mode},
};
use tracing::info;

use crate::{error::FmlError, state::AppState, tui};

pub struct App {
    pub state: AppState,
}

impl App {
    pub fn new(config: crate::config::Config) -> Result<Self, FmlError> {
        Ok(Self {
            state: AppState::new(config)?,
        })
    }

    pub async fn run(&mut self) -> Result<(), FmlError> {
        // Setup the ratatui terminal tui
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        stdout().execute(EnableMouseCapture)?;

        // Spawn our TUI before we get to our
        // blocking event loop
        tui::spawn(
            &self.state.config.tui,
            self.state.events.tui_event_tx.clone(),
        );

        // Sit on our event loop until we want to quit
        self.event_loop().await;

        // Cleanup the tui - we will expect and panic here
        // so if our exit doesn't happen cleanly, we can rely
        // on our panic hooks to cleanup for us. A bit yucky,
        // but better than a failed/weird exit.
        tui::kill().expect("tui cleanup exited safely");

        Ok(())
    }

    /// Handle events as part of the main program
    async fn event_loop(&mut self) {
        // Handle events as our main loop

        loop {
            tokio::select! {
                _ = self.state.events.quit_rx.recv() =>  {
                    info!("quitting tui");
                    break;

                },
                Some(event) = self.state.events.tui_event_rx.recv() => {
                    tui::handle_tui_event(event, &mut self.state);
                },

            }
        }
    }
}
