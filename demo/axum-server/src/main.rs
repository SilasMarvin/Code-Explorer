//! A deliberately layered Axum service: routes -> handlers -> services -> store.

mod error;
mod handlers;
mod models;
mod routes;
mod services;
mod store;

use std::net::SocketAddr;

use store::Store;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt::init();

    let app = routes::router(Store::seeded());
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "listening");
    axum::serve(listener, app).await
}
