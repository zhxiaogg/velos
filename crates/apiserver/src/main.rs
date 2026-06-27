use std::sync::Arc;

use velos_apiserver::controllers::{self, ControllerConfig};
use velos_store::{SqliteStore, Store};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let store: Arc<dyn Store> = Arc::new(SqliteStore::open("velos.db")?);
    controllers::spawn(Arc::clone(&store), ControllerConfig::default());
    let app = velos_apiserver::app(store);

    let addr = "127.0.0.1:8080";
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("velos-apiserver listening on {addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
