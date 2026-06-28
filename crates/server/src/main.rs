use std::sync::Arc;

use clap::Parser;
use velos_auth::{AuthService, StoreAuthenticator};
use velos_server::controllers::{self, ControllerConfig};
use velos_store::{SqliteStore, Store};

/// The Velos control-plane server.
#[derive(Parser, Debug)]
#[command(name = "velos-server", version)]
struct Args {
    /// Address to bind, e.g. 127.0.0.1:8080 or 0.0.0.0:8080.
    #[arg(long, env = "VELOS_LISTEN", default_value = "127.0.0.1:8080")]
    listen: String,

    /// Path to the SQLite database file.
    #[arg(long, env = "VELOS_DB", default_value = "velos.db")]
    db: String,

    /// Probe a running server's `/healthz` over loopback and exit 0 if healthy,
    /// non-zero otherwise. Used as the container HEALTHCHECK so the image needs
    /// no curl/wget. Uses the port from `--listen`/`VELOS_LISTEN`.
    #[arg(long)]
    health_check: bool,
}

/// Probe `http://127.0.0.1:<port>/healthz` and return Ok on HTTP 200. Connects
/// to loopback (the server may bind 0.0.0.0) on the port from `listen`. Uses
/// only std so the runtime image stays dependency-free.
fn health_check(listen: &str) -> anyhow::Result<()> {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    let port = listen
        .rsplit(':')
        .next()
        .and_then(|p| p.parse::<u16>().ok())
        .ok_or_else(|| anyhow::anyhow!("invalid listen address: {listen}"))?;
    let timeout = Duration::from_secs(2);

    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    stream.write_all(b"GET /healthz HTTP/1.0\r\nHost: localhost\r\nConnection: close\r\n\r\n")?;

    let mut response = String::new();
    stream.read_to_string(&mut response)?;

    let status = response.lines().next().unwrap_or_default();
    if status.contains(" 200") {
        Ok(())
    } else {
        anyhow::bail!("unhealthy: {status}")
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

    if args.health_check {
        return health_check(&args.listen);
    }

    let store: Arc<dyn Store> = Arc::new(SqliteStore::open(&args.db)?);
    controllers::spawn(Arc::clone(&store), ControllerConfig::default());
    let auth: Arc<dyn AuthService> = Arc::new(StoreAuthenticator::new(Arc::clone(&store)));
    let app = velos_server::app_with_auth(store, auth);

    let listener = tokio::net::TcpListener::bind(&args.listen).await?;
    tracing::info!(
        "velos-server listening on {} (db: {})",
        args.listen,
        args.db
    );
    axum::serve(listener, app).await?;
    Ok(())
}
