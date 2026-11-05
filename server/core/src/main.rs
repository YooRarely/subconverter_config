use subconverter_core::{env, router};
use tracing::info;

#[tokio::main]
async fn main() {
    // 优先加载 .env 文件
    dotenvy::dotenv().ok();

    // 初始化环境变量校验
    env::init().expect("QEnv 校验失败：缺少必要的环境变量");

    // 初始化日志跟踪系统
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(env::RUST_LOG))
        .init();

    let addr = format!("0.0.0.0:{}", env::PORT);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("绑定监听端口 {} 失败: {}", addr, e));

    info!("🚀 Rust 本地独立订阅转换服务 (Server2) 已启动: http://{}", addr);
    info!("💡 支持接口: GET /url?<订阅链接>  或  GET /{{key}}?{{订阅链接}}");

    axum::serve(listener, router()).await.unwrap();
}
