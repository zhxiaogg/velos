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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let args = Args::parse();

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
