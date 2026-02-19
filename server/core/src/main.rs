mod pruner; // 引入新文件

use axum::{
    routing::get,
    Router,
};
use std::env;
use tracing::{error, info, warn};
use core::router;
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(
            env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .init();

    let port = env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let addr = format!("0.0.0.0:{}", port);
    
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    info!("🚀 Rust 中转服务已启动: http://{}", addr);
    axum::serve(listener, router()).await.unwrap();
}
