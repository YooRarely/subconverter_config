use axum::{
    Router,
    extract::{OriginalUri, Path},
    response::{IntoResponse},
    routing::get,
};
use hyper::{HeaderMap, StatusCode};
use tracing::{info, warn};

mod env;
mod groups;
mod registry;
mod rules;
mod store;
mod subconverter;
pub fn router() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/url", get(url_handler))
        .route("/favicon.ico", get(ico))
        .route("/{key}", get(subscribe_handler))
}
async fn ico() {}
async fn index() -> &'static str {
    "Private Subconverter Service is Running (Rust Axum).V1.4"
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
    subconverter::from_url(headers, &encoded_url).await
}
async fn subscribe_handler(
    Path(key): Path<String>, // 提取捕获到的路径字符串
    headers: HeaderMap,

    OriginalUri(uri): OriginalUri,
) -> impl IntoResponse {
    info!("尝试匹配路径密钥: {}", key);
    let query = uri.query().unwrap_or("");
    match query {
        "" => match registry::lookup(&key).await {
            Some(url) => {
				info!("订阅地址: {} = {}", key, url);
                subconverter::from_url(headers, &url).await
            }
            None => {
                warn!("未找到订阅 URL: {}", key);
                (StatusCode::OK, "未找到订阅 URL").into_response()
            }
        },
        "clear" => {
            registry::unregister(&key).await;
            (StatusCode::OK, "操作完成").into_response()
        }
        _ => {
            registry::register(&key, &query).await;
            subconverter::from_url(headers, &query).await
        }
    }
}
