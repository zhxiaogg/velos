//! End-to-end: real server (in-process, over TCP) + a `veloslet` driving a
//! `FakeRuntime`, exercising the container happy path Pending → Scheduled →
//! Running → Succeeded through the public REST API.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use velos_runtime::FakeRuntime;
use velos_server::{app, controllers};
use velos_store::{SqliteStore, Store};
use veloslet::{ApiClient, run_once};

/// Bind an ephemeral port, serve the server in the background, and return the
/// base URL plus the shared store (so the test can drive controllers directly).
async fn start() -> (String, Arc<dyn Store>) {
    let store: Arc<dyn Store> = Arc::new(SqliteStore::in_memory().unwrap());
    let router = app(Arc::clone(&store));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (format!("http://{addr}"), store)
}

async fn post(http: &reqwest::Client, base: &str, plural: &str, body: serde_json::Value) {
    let resp = http
        .post(format!("{base}/api/v1/{plural}"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::CREATED);
}

async fn get_container(http: &reqwest::Client, base: &str, name: &str) -> serde_json::Value {
    http.get(format!("{base}/api/v1/containers/{name}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn container_runs_through_full_lifecycle() {
    let (base, store) = start().await;
    let http = reqwest::Client::new();

    // A ready worker with capacity.
    post(
        &http,
        &base,
        "workers",
        serde_json::json!({
            "metadata": { "name": "w1" },
            "spec": { "unschedulable": false },
            "status": {
                "allocatable": { "cpu": 4, "memoryBytes": 8589934592u64 },
                "conditions": [{ "conditionType": "Ready", "status": true }]
            }
        }),
    )
    .await;

    // A pending container.
    post(
        &http,
        &base,
        "containers",
        serde_json::json!({
            "metadata": { "name": "c1" },
            "spec": { "image": "alpine", "resources": { "cpu": 1, "memoryBytes": 536870912u64 } },
            "status": { "phase": "Pending" }
        }),
    )
    .await;

    // Scheduler binds the container to the worker.
    let bound = controllers::reconcile_scheduling(store.as_ref()).unwrap();
    assert_eq!(bound, 1);
    let c = get_container(&http, &base, "c1").await;
    assert_eq!(c["spec"]["nodeName"], "w1");
    assert_eq!(c["status"]["phase"], "Scheduled");
    let uid = c["metadata"]["uid"].as_str().unwrap().to_string();

    // veloslet observes the assignment and launches the instance.
    let client = ApiClient::new(&base, None);
    let runtime = FakeRuntime::new();
    let acted = run_once(&client, &runtime, "w1").await.unwrap();
    assert_eq!(acted, 1);
    let c = get_container(&http, &base, "c1").await;
    assert_eq!(c["status"]["phase"], "Running");
    assert_eq!(c["status"]["workerName"], "w1");

    // A second pass is a no-op (already Running and reported).
    assert_eq!(run_once(&client, &runtime, "w1").await.unwrap(), 0);

    // The instance exits cleanly; veloslet reports the terminal phase.
    runtime.set_exited(&uid, 0).unwrap();
    let acted = run_once(&client, &runtime, "w1").await.unwrap();
    assert_eq!(acted, 1);
    let c = get_container(&http, &base, "c1").await;
    assert_eq!(c["status"]["phase"], "Succeeded");
    assert_eq!(c["status"]["exitCode"], 0);
}

/// Serve an auth-enabled server on an ephemeral port; return the base URL.
async fn start_auth() -> String {
    let store: Arc<dyn Store> = Arc::new(SqliteStore::in_memory().unwrap());
    let auth = Arc::new(velos_auth::StoreAuthenticator::new(Arc::clone(&store)));
    let router = velos_server::app_with_auth(store, auth);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn admin_auth_end_to_end() {
    let base = start_auth().await;
    let http = reqwest::Client::new();

    // Unauthenticated /api/v1 is rejected while uninitialized.
    let r = http
        .get(format!("{base}/api/v1/containers"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::UNAUTHORIZED);

    // First-run setup, then login for a session token.
    http.post(format!("{base}/auth/v1/setup"))
        .json(&serde_json::json!({ "username": "admin", "password": "pw" }))
        .send()
        .await
        .unwrap();
    let session = http
        .post(format!("{base}/auth/v1/login"))
        .json(&serde_json::json!({ "username": "admin", "password": "pw" }))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();

    // Mint a CLI token and use it on /api/v1.
    let cli = http
        .post(format!("{base}/auth/v1/admin/tokens"))
        .bearer_auth(&session)
        .json(&serde_json::json!({ "label": "ci" }))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();
    let r = http
        .get(format!("{base}/api/v1/containers"))
        .bearer_auth(&cli)
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());

    // Worker bootstrap still works: admin mints, worker registers.
    let boot = http
        .post(format!("{base}/auth/v1/tokens"))
        .bearer_auth(&session)
        .json(&serde_json::json!({ "ttlSeconds": 3600 }))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    let boot_tok = format!(
        "{}.{}",
        boot["tokenId"].as_str().unwrap(),
        boot["secret"].as_str().unwrap()
    );
    let reg = http
        .post(format!("{base}/auth/v1/register"))
        .bearer_auth(&boot_tok)
        .json(&serde_json::json!({ "name": "w1" }))
        .send()
        .await
        .unwrap();
    assert!(reg.status().is_success());
}
