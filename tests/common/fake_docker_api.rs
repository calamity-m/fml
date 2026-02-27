//! Fake Docker Engine API server for integration tests.
//!
//! Spins up a minimal `axum` HTTP server on a random TCP port bound to
//! 127.0.0.1. Serves:
//! - `GET /containers/json` — list of configured containers
//! - `GET /containers/{id}/logs` — streaming log output from a channel
//!
//! In production, the Docker API is served over a Unix socket. For tests we
//! use TCP for simplicity (axum 0.7 does not support UnixListener with
//! `axum::serve`). The ingestor under test should accept a configurable base
//! URL so it can be pointed at the test server.
//!
//! # Example
//!
//! ```rust,no_run
//! # tokio_test::block_on(async {
//! use common::fake_docker_api::FakeDockerApi;
//!
//! let api = FakeDockerApi::start().await.unwrap();
//! api.add_container("abc123", "myapp_api_1").await;
//! api.stream_log("abc123", r#"{"level":"INFO","msg":"hello"}"#).await;
//!
//! // Point your ingestor at api.base_url()
//! let url = api.base_url();
//! # });
//! ```

use axum::{
    extract::{Path, State},
    response::IntoResponse,
    routing::get,
    Router,
};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex};

/// State shared between the router and test code.
#[derive(Default)]
struct ApiState {
    containers: Vec<serde_json::Value>,
    /// Per-container log lines buffered for /containers/{id}/logs.
    log_lines: HashMap<String, Vec<String>>,
    /// Senders for live streaming (future use).
    log_senders: HashMap<String, mpsc::UnboundedSender<String>>,
}

/// Handle to the running fake Docker API server.
pub struct FakeDockerApi {
    addr: SocketAddr,
    state: Arc<Mutex<ApiState>>,
}

impl FakeDockerApi {
    /// Start the fake Docker API server on a random port. Returns once the
    /// server is listening.
    pub async fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let state = Arc::new(Mutex::new(ApiState::default()));

        let app = Router::new()
            .route("/containers/json", get(list_containers))
            .route("/containers/:id/logs", get(stream_logs))
            .with_state(state.clone());

        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        // Give the task a moment to register.
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;

        Ok(Self { addr, state })
    }

    /// Base URL for the API (e.g. `http://127.0.0.1:PORT`).
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Register a container in the /containers/json response.
    pub async fn add_container(&self, id: &str, name: &str) {
        let (tx, _rx) = mpsc::unbounded_channel::<String>();
        let mut state = self.state.lock().await;
        state.containers.push(serde_json::json!({
            "Id": id,
            "Names": [format!("/{}", name)],
            "State": "running",
            "Labels": {}
        }));
        state.log_lines.insert(id.to_string(), vec![]);
        state.log_senders.insert(id.to_string(), tx);
    }

    /// Buffer a log line to be returned by /containers/{id}/logs.
    pub async fn stream_log(&self, container_id: &str, line: &str) {
        let mut state = self.state.lock().await;
        if let Some(lines) = state.log_lines.get_mut(container_id) {
            lines.push(line.to_string());
        }
    }
}

// ---------------------------------------------------------------------------
// Route handlers
// ---------------------------------------------------------------------------

async fn list_containers(State(state): State<Arc<Mutex<ApiState>>>) -> impl IntoResponse {
    let state = state.lock().await;
    axum::Json(state.containers.clone())
}

async fn stream_logs(
    Path(id): Path<String>,
    State(state): State<Arc<Mutex<ApiState>>>,
) -> impl IntoResponse {
    let lines = {
        let state = state.lock().await;
        state.log_lines.get(&id).cloned().unwrap_or_default()
    };

    if lines.is_empty() {
        return (axum::http::StatusCode::NOT_FOUND, String::new());
    }

    (axum::http::StatusCode::OK, lines.join("\n"))
}
