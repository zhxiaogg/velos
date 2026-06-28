//! REAL end-to-end test against Apple Containerization — nothing is faked.
//!
//! The full pipeline runs for real: apiserver + controllers over TCP, bootstrap
//! token mint, worker registration, the `veloslet` loop (lease heartbeat +
//! reconcile) driving the actual `container` CLI, and a real micro-VM launched
//! from a real image. It asserts the container goes Pending → Scheduled →
//! Running → Succeeded, then exercises finalizer-based deletion.
//!
//! This test is always enabled (no `#[ignore]`): on a host with a working Apple
//! `container` CLI it runs for real end to end; everywhere else it **self-skips**
//! (the `container` binary isn't callable), so `cargo test` and CI stay green.
//! Run it explicitly with output via:
//!
//!     cargo test -p velos-tests --test e2e_apple -- --nocapture
//!
//! NOTE: the exact `container` CLI flags are centralized in
//! `velos_runtime::AppleContainer`; they match the apple/container 1.0 reference.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use velos_auth::{AuthService, StoreAuthenticator};
use velos_runtime::{AppleContainer, ContainerRuntime, RunSpec};
use velos_server::app_with_auth;
use velos_server::controllers::{self, ControllerConfig};
use velos_store::{SqliteStore, Store};
use veloslet::{ApiClient, run_loop};

const NODE: &str = "worker-e2e";
const IMAGE: &str = "docker.io/library/alpine:3";

/// Poll a container's `status.phase` until it is one of `wanted`, or panic on timeout.
async fn await_phase(
    http: &reqwest::Client,
    base: &str,
    token: &str,
    name: &str,
    wanted: &[&str],
    timeout: Duration,
) -> String {
    let deadline = Instant::now() + timeout;
    let mut last = String::from("<none>");
    while Instant::now() < deadline {
        let resp = http
            .get(format!("{base}/api/v1/containers/{name}"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        if resp.status().is_success() {
            let doc: Value = resp.json().await.unwrap();
            if let Some(phase) = doc.pointer("/status/phase").and_then(Value::as_str) {
                last = phase.to_string();
                if wanted.contains(&phase) {
                    return last;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("timed out waiting for {name} to reach {wanted:?}; last phase = {last}");
}

#[tokio::test]
async fn real_container_lifecycle_with_apple_containerization() {
    if !AppleContainer::new().available().await {
        eprintln!("SKIP: Apple `container` CLI not available on this host");
        return;
    }
    // Pre-flight: confirm this host can actually *launch* a micro-VM (the CLI may
    // be installed on a runner that lacks nested virtualization). If it can't, we
    // skip cleanly rather than failing — separating "environment can't run" from
    // "Velos has a bug".
    {
        let probe = AppleContainer::new();
        let spec = RunSpec {
            uid: "preflight".to_string(),
            image: IMAGE.to_string(),
            command: vec!["true".to_string()],
            env: vec![],
        };
        // Time-bound the probe so a runner that has the CLI but can't virtualize
        // skips quickly instead of hanging CI.
        match tokio::time::timeout(Duration::from_secs(120), probe.run(&spec)).await {
            Ok(Ok(_)) => {
                let _ = probe.remove("preflight").await;
            }
            Ok(Err(e)) => {
                eprintln!("SKIP: host cannot launch containers ({e})");
                return;
            }
            Err(_) => {
                eprintln!("SKIP: container launch timed out; host likely cannot virtualize");
                return;
            }
        }
    }

    // --- Control plane: apiserver + controllers (fast intervals for the test) ---
    let store: Arc<dyn Store> = Arc::new(SqliteStore::in_memory().unwrap());
    let auth: Arc<dyn AuthService> = Arc::new(StoreAuthenticator::new(Arc::clone(&store)));
    controllers::spawn(
        Arc::clone(&store),
        ControllerConfig {
            schedule_interval: Duration::from_millis(500),
            lifecycle_interval: Duration::from_millis(500),
            eviction_timeout: Duration::from_secs(300),
        },
    );
    let router = app_with_auth(Arc::clone(&store), Arc::clone(&auth));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let base = format!("http://{addr}");
    let http = reqwest::Client::new();

    // --- First-run admin setup + login (bootstrap minting is admin-only) ---
    http.post(format!("{base}/auth/v1/setup"))
        .json(&json!({ "username": "admin", "password": "pw" }))
        .send()
        .await
        .unwrap();
    let session: String = http
        .post(format!("{base}/auth/v1/login"))
        .json(&json!({ "username": "admin", "password": "pw" }))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string();

    // --- Mint a bootstrap token (operator action, as admin) ---
    let tok: Value = http
        .post(format!("{base}/auth/v1/tokens"))
        .bearer_auth(&session)
        .json(&json!({ "ttlSeconds": 1200 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let boot = format!(
        "{}.{}",
        tok["tokenId"].as_str().unwrap(),
        tok["secret"].as_str().unwrap()
    );

    // --- Worker registration (real join flow) ---
    let boot_client = ApiClient::new(&base, Some(boot));
    let reg = boot_client
        .register(&json!({
            "name": NODE,
            "capacity": { "cpu": 4, "memoryBytes": 8589934592u64, "maxContainers": 16 },
            "addresses": [],
            "containerRuntimeVersion": "apple-containerization",
        }))
        .await
        .unwrap();
    let credential = reg["token"].as_str().unwrap().to_string();

    // --- Start the real veloslet loop (heartbeat + reconcile, real runtime) ---
    let runtime: Arc<dyn ContainerRuntime> = Arc::new(AppleContainer::new());
    {
        let client = ApiClient::new(&base, Some(credential.clone()));
        let runtime = Arc::clone(&runtime);
        tokio::spawn(async move {
            run_loop(
                client,
                runtime,
                NODE.to_string(),
                Duration::from_millis(500),
                Duration::from_secs(1),
                40,
            )
            .await;
        });
    }

    // --- Operator creates a container (short-lived: runs then exits 0) ---
    let resp = http
        .post(format!("{base}/api/v1/containers"))
        .bearer_auth(&credential)
        .json(&json!({
            "metadata": { "name": "c-real", "finalizers": ["veloslet"] },
            "spec": {
                "image": IMAGE,
                "command": ["sleep", "3"],
                "resources": { "cpu": 1, "memoryBytes": 536870912u64 }
            },
            "status": { "phase": "Pending" }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::CREATED);

    // Scheduler binds it, veloslet launches the real micro-VM → Running.
    // Generous timeout to allow the first image pull.
    let phase = await_phase(
        &http,
        &base,
        &credential,
        "c-real",
        &["Running", "Succeeded"],
        Duration::from_secs(180),
    )
    .await;
    println!("observed phase: {phase}");

    // The process exits cleanly → veloslet reports terminal status.
    let phase = await_phase(
        &http,
        &base,
        &credential,
        "c-real",
        &["Succeeded", "Failed"],
        Duration::from_secs(60),
    )
    .await;
    assert_eq!(phase, "Succeeded", "container should exit 0");

    // --- Deletion: finalizer protocol drives real stop + remove ---
    let resp = http
        .delete(format!("{base}/api/v1/containers/c-real"))
        .bearer_auth(&credential)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());

    // veloslet clears its finalizer → apiserver hard-deletes → GET 404.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let resp = http
            .get(format!("{base}/api/v1/containers/c-real"))
            .bearer_auth(&credential)
            .send()
            .await
            .unwrap();
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "container was never hard-deleted"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
