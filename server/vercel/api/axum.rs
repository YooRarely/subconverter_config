use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use core::router;
use tower::ServiceBuilder;
use vercel_runtime::Error;
use vercel_runtime::axum::VercelLayer;

// 针对 Vercel Serverless (AWS Lambda) 运行时无静态目录的平台特殊限制，
// 由 vercel-adapter 自身在编译期直接注入静态 Web 资产，不污染核心框架。
const VERCEL_INDEX_HTML: &str = include_str!("../../web/index.html");
const VERCEL_FONT_BYTES: &[u8] = include_bytes!("../../web/TwemojiCountryFlags.woff2");

async fn vercel_page_handler() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], VERCEL_INDEX_HTML)
}

async fn vercel_font_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "font/woff2"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        VERCEL_FONT_BYTES,
    )
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    // 挂载 core 基础业务路由，并在 vercel 适配层覆盖注入静态 Web 资产
    let app_router = router()
        .route("/", get(vercel_page_handler))
        .route("/TwemojiCountryFlags.woff2", get(vercel_font_handler));

    let app = ServiceBuilder::new()
        .layer(VercelLayer::new())
        .service(app_router);

    vercel_runtime::run(app).await
}
