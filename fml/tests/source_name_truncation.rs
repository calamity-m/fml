#![cfg(feature = "integration")]

mod common;

use std::time::Duration;

use fml::{
    app::App,
    config::Config,
    event::{ProducerEvent, QuitEvent, TuiEvent},
};

use common::{buffer_to_string, make_entry_with_source_display};

/// Long Kubernetes `pod/container` display names are middle-truncated in the
/// log pane so the message column keeps its space. Without truncation each
/// `payments-api-7d4b8c9f5d-<suffix>/<container>` name alone is wider than the
/// split-layout log pane, pushing the message entirely off-screen. The
/// snapshot shows the shortened names (workload prefix + container suffix
/// survive, the replicaset hash is dropped) and the still-visible messages.
#[tokio::test]
async fn long_kubernetes_names_are_truncated_in_log_pane() {
    let config = Config::default();
    let app = App::with_test_backend(config, 80, 24).expect("app construction");

    let producer_tx = app.state.event_bus.producer_event_tx.clone();
    let tui_tx = app.state.event_bus.tui_event_tx.clone();
    let quit_tx = app.state.event_bus.quit_tx.clone();

    tokio::spawn(async move {
        let sources = [
            ("payments-api-7d4b8c9f5d-x2k4p/payments", "request accepted"),
            (
                "payments-api-7d4b8c9f5d-x2k4p/istio-proxy",
                "upstream ready",
            ),
            ("checkout-worker-6c5f9b8d7c-qz9wp/worker", "job dequeued"),
        ];
        for (display, msg) in sources {
            producer_tx
                .send(ProducerEvent::StoreEvent(make_entry_with_source_display(
                    msg,
                    &format!("ns/{display}"),
                    display,
                )))
                .await
                .expect("send producer event");
        }

        // Let the tail worker tick and flow its result back into TUI state.
        tokio::time::sleep(Duration::from_millis(400)).await;

        tui_tx.send(TuiEvent::Render).expect("send render event");
        tokio::time::sleep(Duration::from_millis(50)).await;
        quit_tx.send(QuitEvent {}).await.expect("send quit");
    });

    let app = app.run_until_quit().await;
    insta::assert_snapshot!(buffer_to_string(app.terminal.backend().buffer()));
}
