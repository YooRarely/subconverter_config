use std::env;

use axum::{
    Router,
    body::Body,
    extract::{OriginalUri, Path},
    response::{IntoResponse, Response},
    routing::get,
};
use hyper::{HeaderMap, StatusCode, header};
use tracing::{error, info, warn};

mod pruner;
mod rules_processor;
pub fn router() -> Router {
    Router::new()
        .route("/", get(index))
        .route("/url", get(proxy_handler))
        .route("/{key}", get(subscript))
}
async fn execute_proxy_logic<URL: AsRef<str>>(headers: HeaderMap,url: URL) ->Response
{
    let sub_backend = env::var("SUB_BACKEND")
        .unwrap_or_else(|_| "http://subconverter.zeabur.internal:25500/sub".into());
    let github_config = env::var("GITHUB_CONFIG_URL")
	.unwrap_or_else(|_| "https://raw.githubusercontent.com/YooRarely/subconverter_config/refs/heads/main/config/remote_config.toml".into());

    let final_url = format!(
        "{}?target=clash&url={}&config={}&emoji=true&list=false&udp=true",
        sub_backend, url.as_ref(), github_config
    );

    let mut forward_headers = headers.clone();
    forward_headers.remove(header::HOST);

    let client = reqwest::Client::new();
    let res = match client.get(&final_url).headers(forward_headers).send().await {
        Ok(r) => r,
        Err(e) => {
            error!("无法连接后端: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Network error: {}", e),
            )
                .into_response();
        }
    };

    let res_status =
        StatusCode::from_u16(res.status().as_u16()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let res_headers = res.headers().clone();

    if !res_status.is_success() {
        let err_body = res.text().await.unwrap_or_default();
        error!("后端返回错误 ({}): {}", res_status, err_body);
        return (res_status, err_body).into_response();
    }

    // 请求成功，打印日志
    info!("后端请求成功，开始处理 YAML 数据...");

    let body_text = res.text().await.unwrap_or_default();

    // 解析并剪枝
    let config_data: serde_yaml_ng::Value = match serde_yaml_ng::from_str(&body_text) {
        Ok(v) => v,
        Err(e) => {
            error!("YAML 解析失败: {}", e);
            return (StatusCode::OK, body_text).into_response(); // 解析失败则透传原文
        }
    };

    // 调用 pruner.rs 里的主函数
    let clean_config = pruner::main_prune(config_data);
    let final_config = rules_processor::apply_custom_rules(clean_config).await;
    let result_yaml = serde_yaml_ng::to_string(&final_config).unwrap_or_default();

    info!("处理完成，正在透传结果...");

    let mut response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/yaml; charset=utf-8");

    let excluded = [
        "content-encoding",
        "content-length",
        "transfer-encoding",
        "connection",
    ];
    if let Some(headers_mut) = response.headers_mut() {
        for (name, value) in res_headers.iter() {
            if !excluded.contains(&name.as_str()) {
                headers_mut.insert(name, value.clone());
            }
        }
    }
    response
        .body(Body::from(result_yaml))
        .unwrap()
        .into_response()
}
async fn index() -> &'static str {
    "Private Subconverter Service is Running (Rust Axum).V1.2"
}
async fn proxy_handler(headers: HeaderMap, OriginalUri(uri): OriginalUri) -> impl IntoResponse {
    info!("--- 收到新请求 ---");

    let raw_query = uri.query().unwrap_or("");
    if raw_query.is_empty() {
        warn!("请求失败: 未提供机场 URL");
        return (StatusCode::BAD_REQUEST, "Missing airport URL.").into_response();
    }

    let decoded_url =
        urlencoding::decode(raw_query).unwrap_or_else(|_| std::borrow::Cow::Borrowed(raw_query));
    let encoded_url = urlencoding::encode(&decoded_url);
	execute_proxy_logic(headers,encoded_url).await
	
   
}
async fn subscript(
    Path(key): Path<String>, // 提取捕获到的路径字符串
    headers: HeaderMap,
    OriginalUri(_): OriginalUri,
) -> impl IntoResponse {
    info!("尝试匹配路径密钥: {}", key);

    // 1. 直接把路径 key 当作环境变量名去查找
    match std::env::var(&key) {
        Ok(airport_url) => {
            info!("校验通过，密钥 {} 对应机场: {}", key, airport_url);
			execute_proxy_logic(headers,airport_url).await
        }
        Err(_) => {
            warn!("非法访问: 环境变量中找不到密钥 {}", key);
            (StatusCode::NOT_FOUND, "Resource not found").into_response()
        }
    }
}
