use std::sync::Arc;

use velos_store::SqliteStore;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let store = Arc::new(SqliteStore::open("velos.db")?);
    let app = velos_apiserver::app(store);

    let addr = "127.0.0.1:8080";
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("velos-apiserver listening on {addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
