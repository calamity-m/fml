//! Static log corpora used across harnesses.
//!
//! Each corpus is a `&'static [&'static str]` of representative log lines.
//! The HIGH_VOLUME corpus is intentionally large to stress-test throughput
//! paths without allocating at test time.

use serde_json;

/// A small sample of valid JSON log lines in various shapes.
pub const CORPUS_JSON: &[&str] = &[
    r#"{"ts":"2024-01-15T10:00:00Z","level":"INFO","message":"Server started","port":8080}"#,
    r#"{"timestamp":"2024-01-15T10:00:01Z","severity":"ERROR","msg":"Connection refused","host":"db.internal","port":5432,"err":"dial tcp: connect: connection refused"}"#,
    r#"{"time":"2024-01-15T10:00:02.123Z","level":"WARN","message":"Slow query","duration_ms":4200,"query":"SELECT * FROM users WHERE id=$1"}"#,
    r#"{"@timestamp":"2024-01-15T10:00:03Z","log.level":"debug","message":"Cache miss","key":"user:42","ttl":300}"#,
    r#"{"t":"2024-01-15T10:00:04Z","lvl":"fatal","msg":"Out of memory","rss_mb":16384,"limit_mb":16384}"#,
    r#"{"timestamp":"2024-01-15T10:00:05Z","level":"INFO","request_id":"req-abc123","method":"POST","path":"/api/v1/payments","status":200,"latency_ms":47}"#,
    r#"{"ts":"2024-01-15T10:00:06Z","level":"ERROR","request_id":"req-abc123","error":"payment gateway timeout","gateway":"stripe","attempt":3}"#,
    r#"{"ts":"2024-01-15T10:00:07Z","level":"INFO","message":"Token validated","user_id":"usr-999","token_type":"bearer","expires_in":3600}"#,
];

/// A small sample of logfmt-style log lines.
pub const CORPUS_LOGFMT: &[&str] = &[
    "ts=2024-01-15T10:00:00Z level=info msg=\"Server started\" port=8080",
    "ts=2024-01-15T10:00:01Z level=error msg=\"Connection refused\" host=db.internal err=\"dial tcp: connect: connection refused\"",
    "ts=2024-01-15T10:00:02Z level=warn msg=\"Slow query\" duration_ms=4200",
    "ts=2024-01-15T10:00:03Z level=debug msg=\"Cache miss\" key=user:42 ttl=300",
    "ts=2024-01-15T10:00:04Z level=info method=GET path=/healthz status=200 latency_ms=1",
    "ts=2024-01-15T10:00:05Z level=error msg=\"Auth failed\" user=alice reason=invalid_token",
];

/// Unstructured log lines that require heuristic parsing.
pub const CORPUS_UNSTRUCTURED: &[&str] = &[
    "2024-01-15 10:00:00 INFO  Starting application version 2.4.1",
    "2024-01-15 10:00:01 ERROR Failed to connect to database after 3 retries",
    "Jan 15 10:00:02 myhost sshd[12345]: Failed password for invalid user admin from 10.0.0.1 port 54321 ssh2",
    "[2024-01-15T10:00:03Z] WARN: Disk usage at 92% on /dev/sda1",
    "ERROR: NullPointerException at com.example.App.handle(App.java:42)",
    "10:00:05.123 [main] DEBUG o.s.w.s.DispatcherServlet - Initializing Servlet 'dispatcherServlet'",
    "time=2024-01-15T10:00:06Z severity=CRITICAL message=\"Panic: index out of bounds\"",
    "GET /api/v1/users 200 47ms",
];

/// A mixed corpus combining JSON, logfmt, and unstructured lines.
pub const CORPUS_MIXED: &[&str] = &[
    r#"{"ts":"2024-01-15T10:00:00Z","level":"INFO","message":"api-gateway started"}"#,
    "ts=2024-01-15T10:00:01Z level=info msg=\"auth-service ready\" port=9090",
    "2024-01-15 10:00:02 ERROR worker-1: task queue overflow",
    r#"{"ts":"2024-01-15T10:00:03Z","level":"ERROR","request_id":"req-xyz","message":"upstream timeout"}"#,
    "ts=2024-01-15T10:00:04Z level=warn msg=\"retry\" attempt=2 max=3",
    "2024-01-15 10:00:05 INFO  Graceful shutdown complete",
];

/// Generate 1 000 synthetic log lines for throughput testing. Allocates once
/// and leaks into 'static so callers get a `&'static [String]`-like handle.
/// Call this from `#[test]` setup; subsequent calls return the same slice.
pub fn corpus_high_volume() -> Vec<String> {
    (0..1_000usize)
        .map(|i| {
            let level = match i % 10 {
                0 => "ERROR",
                1 | 2 => "WARN",
                _ => "INFO",
            };
            format!(
                r#"{{"ts":"2024-01-15T{:02}:{:02}:{:02}Z","level":"{}","message":"log line {}","seq":{}}}"#,
                i / 3600 % 24,
                i / 60 % 60,
                i % 60,
                level,
                i,
                i,
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Fixture file generation helpers
// ---------------------------------------------------------------------------

/// Write the high-volume fixture and Kubernetes/Docker JSON fixtures to
/// `tests/fixtures/` if they don't already exist. Call this from test setup
/// before throughput tests.
pub fn ensure_fixtures(fixture_dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(fixture_dir)?;
    ensure_high_volume(fixture_dir)?;
    ensure_kubernetes_pods(fixture_dir)?;
    ensure_docker_containers(fixture_dir)?;
    Ok(())
}

fn ensure_high_volume(dir: &std::path::Path) -> std::io::Result<()> {
    let path = dir.join("high_volume.log");
    if path.exists() {
        return Ok(());
    }
    let lines: Vec<String> = corpus_high_volume();
    std::fs::write(path, lines.join("\n"))
}

fn ensure_kubernetes_pods(dir: &std::path::Path) -> std::io::Result<()> {
    let path = dir.join("kubernetes_pods.json");
    if path.exists() {
        return Ok(());
    }
    let json = serde_json::json!({
        "apiVersion": "v1",
        "kind": "PodList",
        "items": [
            {
                "metadata": { "name": "api-7f9b4d", "namespace": "default" },
                "status": { "phase": "Running" },
                "spec": { "containers": [{ "name": "api" }] }
            },
            {
                "metadata": { "name": "worker-4c2a", "namespace": "default" },
                "status": { "phase": "Running" },
                "spec": { "containers": [{ "name": "worker" }] }
            },
            {
                "metadata": { "name": "worker-9e1b", "namespace": "default" },
                "status": { "phase": "Running" },
                "spec": { "containers": [{ "name": "worker" }] }
            }
        ]
    });
    std::fs::write(path, serde_json::to_string_pretty(&json).unwrap())
}

fn ensure_docker_containers(dir: &std::path::Path) -> std::io::Result<()> {
    let path = dir.join("docker_containers.json");
    if path.exists() {
        return Ok(());
    }
    let json = serde_json::json!([
        {
            "Id": "abc123def456",
            "Names": ["/myapp_api_1"],
            "Image": "myapp/api:latest",
            "State": "running",
            "Labels": {
                "com.docker.compose.project": "myapp",
                "com.docker.compose.service": "api"
            }
        },
        {
            "Id": "bcd234efa567",
            "Names": ["/myapp_worker_1"],
            "Image": "myapp/worker:latest",
            "State": "running",
            "Labels": {
                "com.docker.compose.project": "myapp",
                "com.docker.compose.service": "worker"
            }
        },
        {
            "Id": "cde345gab678",
            "Names": ["/standalone"],
            "Image": "nginx:latest",
            "State": "running",
            "Labels": {}
        }
    ]);
    std::fs::write(path, serde_json::to_string_pretty(&json).unwrap())
}
