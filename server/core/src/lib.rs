use axum::{
    Router,
    extract::{OriginalUri, Path},
    response::IntoResponse,
    routing::get,
};
use hyper::{HeaderMap, StatusCode};
use tracing::{info, warn};

mod pruner;
mod rules_processor;
mod subscribe;
mod url;
pub fn router() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/url", get(url_handler))
        .route("/{key}", get(subscribe_handler))
}

async fn index() -> &'static str {
    "Private Subconverter Service is Running (Rust Axum).V1.3"
}
async fn url_handler(headers: HeaderMap, OriginalUri(uri): OriginalUri) -> impl IntoResponse {
    info!("--- 收到新请求 ---");

    let raw_query = uri.query().unwrap_or("");
    if raw_query.is_empty() {
        warn!("请求失败: 未提供机场 URL");
        return (StatusCode::BAD_REQUEST, "Missing airport URL.").into_response();
    }

    let decoded_url =
        urlencoding::decode(raw_query).unwrap_or_else(|_| std::borrow::Cow::Borrowed(raw_query));
    let encoded_url = urlencoding::encode(&decoded_url);
    url::request(headers, encoded_url).await
}
async fn subscribe_handler(
    Path(key): Path<String>, // 提取捕获到的路径字符串
    headers: HeaderMap,
    OriginalUri(_): OriginalUri,
) -> impl IntoResponse {
    info!("尝试匹配路径密钥: {}", key);
    let subscribe_map = subscribe::fetch().await;
    // 1. 直接把路径 key 当作环境变量名去查找

    match subscribe_map.get(&key) {
        Some(airport_url) => {
            info!("校验通过，密钥 {} 对应机场: {}", key, airport_url);
            url::request(headers, airport_url).await
        }
        None => {
            warn!("非法访问: 环境变量中找不到密钥 {}", key);
            (StatusCode::NOT_FOUND, "Resource not found").into_response()
        }
    }
}
