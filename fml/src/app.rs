use std::io::{Stdout, stdout};

use crossterm::{
    ExecutableCommand as _,
    event::EnableMouseCapture,
    terminal::{EnterAlternateScreen, enable_raw_mode},
};
use ratatui::{Terminal, backend::Backend, prelude::CrosstermBackend};
use tracing::info;

use crate::{
    error::FmlError,
    event::{Query, TuiEvent},
    log::Source,
    producer::{
        self, LogProducer, ProducerSpec, docker::DockerProducer, fake::FakeProducer,
        file::FileProducer, kubernetes::KubernetesProducer,
    },
    search,
    state::AppState,
    tui,
};

pub struct App<B: Backend = CrosstermBackend<Stdout>> {
    pub state: AppState,
    pub terminal: Terminal<B>,
    pub producers: Vec<Box<dyn LogProducer>>,
}

impl App<CrosstermBackend<Stdout>> {
    pub fn new(config: crate::config::Config, specs: Vec<ProducerSpec>) -> Result<Self, FmlError> {
        let mut app = Self {
            state: AppState::new(config)?,
            terminal: Terminal::new(CrosstermBackend::new(stdout()))?,
            producers: Vec::new(),
        };

        let mut demo_count: u32 = 0;
        for spec in specs {
            match spec {
                ProducerSpec::Demo => {
                    demo_count += 1;
                    let source = Source {
                        producer: "fake".to_string(),
                        id: format!("demo-{demo_count}"),
                        display_name: format!("Demo {demo_count}"),
                        group: None,
                    };
                    app.register_producer(Box::new(FakeProducer::new(source)));
                }
                ProducerSpec::File(path) => {
                    app.register_producer(Box::new(FileProducer::new(path)));
                }
                ProducerSpec::Docker => match DockerProducer::new() {
                    Ok(producer) => app.register_producer(Box::new(producer)),
                    Err(err) => tracing::warn!("failed to construct docker producer: {err}"),
                },
                ProducerSpec::Kubernetes(ns) => {
                    match ns
                        .map(Ok)
                        .unwrap_or_else(KubernetesProducer::resolve_namespace)
                        .and_then(KubernetesProducer::new)
                    {
                        Ok(producer) => app.register_producer(Box::new(producer)),
                        Err(err) => {
                            tracing::warn!("failed to construct kubernetes producer: {err}")
                        }
                    }
                }
            }
        }

        Ok(app)
    }
}

#[cfg(any(test, feature = "integration"))]
impl App<ratatui::backend::TestBackend> {
    /// Build an `App` backed by a `TestBackend` for integration tests.
    /// No raw mode, no alt screen, no demo producers.
    pub fn with_test_backend(
        config: crate::config::Config,
        width: u16,
        height: u16,
    ) -> Result<Self, FmlError> {
        // `Terminal::new(TestBackend)` returns `Result<_, Infallible>`, so
        // `unwrap` here is genuinely unreachable rather than a panic risk.
        let terminal = Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        Ok(Self {
            state: AppState::new(config)?,
            terminal,
            producers: Vec::new(),
        })
    }

    /// Run the same `event_loop` used in production until a `QuitEvent`
    /// arrives, then return `self` so tests can inspect state and snapshot
    /// the terminal buffer. Skips raw-mode setup, `tui::spawn`, and producer
    /// auto-start — tests own those concerns.
    pub async fn run_until_quit(self) -> Self {
        self.event_loop().await
    }
}

impl<B: Backend> App<B> {
    /// Register a producer to be started after the TUI spawns and stopped
    /// during shutdown. Producers must be registered before [`Self::run`].
    pub fn register_producer(&mut self, producer: Box<dyn LogProducer>) {
        self.producers.push(producer);
    }

    /// Handle events as part of the main program.
    ///
    /// `biased` ordering means a `QuitEvent` does not preempt already-queued
    /// TUI/search/producer events: the loop drains those first (their `recv`
    /// branches return Pending once empty) and only then observes the quit.
    /// This keeps shutdown predictable for tests that queue events ahead of
    /// the loop and avoids dropping a queued render on quit in production.
    pub(crate) async fn event_loop(mut self) -> Self {
        // Kick off a continuous tail search so the log pane starts rendering
        // live entries as soon as producers begin emitting.
        if let Err(err) = self
            .state
            .event_bus
            .tui_event_tx
            .send(TuiEvent::DispatchLogPaneSearch(Query::Tail))
        {
            tracing::warn!("failed to dispatch initial tail search: {err}");
        }

        loop {
            tokio::select! {
                biased;
                Some(event) = self.state.event_bus.tui_event_rx.recv() => {
                    if matches!(event, TuiEvent::Render) {
                        tui::render(&mut self.state, &mut self.terminal);
                    }
                    let new_state = tui::handle_tui_event(event, self.state);
                    self.state = new_state;
                },
                Some(event) = self.state.event_bus.search_event_rx.recv() => {
                    let new_state = search::handle_search_event(event, self.state);
                    self.state = new_state;
                },
                Some(event) = self.state.event_bus.producer_event_rx.recv() => {
                    let new_state = producer::handle_producer_event(event, self.state);
                    self.state = new_state;
                },
                _ = self.state.event_bus.quit_rx.recv() => {
                    info!("quitting tui");
                    break;
                },
            }
        }

        self
    }
}

impl App<CrosstermBackend<Stdout>> {
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
}
