use core::router;

use tracing::info;
mod env;
#[tokio::main]
async fn main() {
    env::init().expect("QEnv 校验失败：缺少必要的环境变量");
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(env::RUST_LOG))
        .init();
    let addr = format!("0.0.0.0:{}", env::PORT);

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    info!("🚀 Rust 中转服务已启动: http://{}", addr);
    axum::serve(listener, router()).await.unwrap();
}
