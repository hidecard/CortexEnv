//! Standalone CortexEnv HTTP server.
use anyhow::Result;
use sandbox_api::router;
use sandbox_runtime::Runtime;
use std::{net::SocketAddr, sync::Arc};
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let address: SocketAddr = std::env::var("CORTEXENV_BIND")
        .unwrap_or_else(|_| "127.0.0.1:8080".into())
        .parse()?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "CortexEnv API server listening");
    axum::serve(listener, router(Arc::new(Runtime::default())))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    tracing::info!("shutdown signal received");
}
