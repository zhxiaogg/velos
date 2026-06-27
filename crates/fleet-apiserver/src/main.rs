use std::sync::Arc;

use fleet_store::SqliteStore;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let store = Arc::new(SqliteStore::open("fleet.db")?);
    let app = fleet_apiserver::app(store);

    let addr = "127.0.0.1:8080";
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("fleet-apiserver listening on {addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
