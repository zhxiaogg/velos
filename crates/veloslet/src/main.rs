use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use velos_runtime::{AppleContainer, ContainerRuntime};
use veloslet::{ApiClient, run_loop};

/// The Velos worker daemon.
#[derive(Parser, Debug)]
#[command(name = "veloslet", version)]
struct Args {
    /// server base URL, e.g. http://127.0.0.1:8080
    #[arg(long, default_value = "http://127.0.0.1:8080")]
    server: String,

    /// This worker's name.
    #[arg(long)]
    node: String,

    /// Bootstrap token (`id.secret`) used to register on first start.
    #[arg(long)]
    token: Option<String>,

    /// Reconcile interval in seconds.
    #[arg(long, default_value_t = 5)]
    reconcile_secs: u64,

    /// Heartbeat (lease renew) interval in seconds.
    #[arg(long, default_value_t = 10)]
    heartbeat_secs: u64,

    /// Lease duration in seconds.
    #[arg(long, default_value_t = 40)]
    lease_secs: u32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    let runtime = AppleContainer::new();
    let runtime_version = runtime
        .version()
        .await
        .unwrap_or_else(|_| "unknown".to_string());

    // Register with a bootstrap token to obtain a worker credential.
    let mut credential: Option<String> = None;
    if let Some(token) = &args.token {
        let boot = ApiClient::new(&args.server, Some(token.clone()));
        let request = serde_json::json!({
            "name": args.node,
            "capacity": { "cpu": 4, "memoryBytes": 8u64 * 1024 * 1024 * 1024, "maxContainers": 16 },
            "addresses": [],
            "containerRuntimeVersion": runtime_version,
        });
        let resp = boot.register(&request).await?;
        credential = resp
            .get("token")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        tracing::info!("registered worker {}", args.node);
    }

    let client = ApiClient::new(&args.server, credential.or(args.token.clone()));
    let runtime: Arc<dyn ContainerRuntime> = Arc::new(runtime);

    tracing::info!("veloslet {} reconciling against {}", args.node, args.server);
    run_loop(
        client,
        runtime,
        args.node,
        Duration::from_secs(args.reconcile_secs),
        Duration::from_secs(args.heartbeat_secs),
        args.lease_secs,
    )
    .await;
    Ok(())
}
