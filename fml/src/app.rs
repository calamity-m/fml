use std::io::stdout;

use crossterm::{
    ExecutableCommand as _,
    event::EnableMouseCapture,
    terminal::{EnterAlternateScreen, enable_raw_mode},
};
use tracing::info;

use crate::{
    error::FmlError,
    producer::{self, LogProducer},
    search,
    state::AppState,
    tui,
};

pub struct App {
    pub state: AppState,
    pub producers: Vec<Box<dyn LogProducer>>,
}

impl App {
    pub fn new(config: crate::config::Config) -> Result<Self, FmlError> {
        Ok(Self {
            state: AppState::new(config)?,
            producers: Vec::new(),
        })
    }

    /// Register a producer to be started after the TUI spawns and stopped
    /// during shutdown. Producers must be registered before [`Self::run`].
    pub fn register_producer(&mut self, producer: Box<dyn LogProducer>) {
        self.producers.push(producer);
    }

    pub async fn run(mut self) -> Result<(), FmlError> {
        // Setup the ratatui terminal tui
        enable_raw_mode()?;
        stdout().execute(EnterAlternateScreen)?;
        stdout().execute(EnableMouseCapture)?;

        // Spawn our TUI before we get to our
        // blocking event loop
        tui::spawn(
            &self.state.config.tui,
            self.state.event_bus.tui_event_tx.clone(),
        );

        // Kick off any registered producers now that the event loop below
        // is the consumer of the producer event channel.
        for producer in &self.producers {
            producer.start(self.state.event_bus.producer_event_tx.clone());
        }

        // Sit on our event loop until we want to quit
        self = self.event_loop().await;

        // Signal producers to halt before tearing down the TUI. Each
        // producer is responsible for observing the signal and exiting
        // its background task (see `LogProducer` cancellation contract).
        for producer in &self.producers {
            producer.stop();
        }

        // Cleanup the tui - we will expect and panic here
        // so if our exit doesn't happen cleanly, we can rely
        // on our panic hooks to cleanup for us. A bit yucky,
        // but better than a failed/weird exit.
        tui::kill().expect("tui cleanup exited safely");

        Ok(())
    }

    /// Handle events as part of the main program
    async fn event_loop(mut self) -> Self {
        // Handle events as our main loop

        loop {
            tokio::select! {
                _ = self.state.event_bus.quit_rx.recv() =>  {
                    info!("quitting tui");
                    break;

                },
                Some(event) = self.state.event_bus.tui_event_rx.recv() => {
                    let new_state = tui::handle_tui_event(event, self.state);
                    self.state = new_state;
                },
                Some(event) = self.state.event_bus.search_event_rx.recv() => {
                    let new_state = search::handle_search_event(event, self.state);
                    self.state = new_state
                },
                Some(event) = self.state.event_bus.producer_event_rx.recv() => {
                    let new_state = producer::handle_producer_event(event, self.state);
                    self.state = new_state
                },

            }
        }

        self
    }
}
