use axum::{
    body::Body,
    response::{IntoResponse, Response},
};
use humansize::{BINARY, format_size};
use hyper::{HeaderMap, StatusCode, header};
use tracing::{error, info};

use crate::{env, groups, rules};

pub async fn from_url(headers: HeaderMap, url: &str) -> Response {
    let final_url = format!(
        "{}?target=clash&url={}&config={}&emoji=true&list=false&udp=true",
        env::SUB_BACKEND,
        url,
        env::GITHUB_CONFIG_URL
    );

    let mut forward_headers = headers.clone();
    forward_headers.remove(header::HOST);

    let client = reqwest::Client::new();
    let backend_res = match client.get(&final_url).headers(forward_headers).send().await {
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

    let backend_status = StatusCode::from_u16(backend_res.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let backend_headers = backend_res.headers().clone();

    if !backend_status.is_success() {
        let err_body = backend_res.text().await.unwrap_or_default();
        error!("后端返回错误 ({}): {}", backend_status, err_body);
        return (backend_status, err_body).into_response();
    }

    // 请求成功，打印日志
    info!("后端请求成功，开始处理 YAML 数据...");

    let body_text = backend_res.text().await.unwrap_or_default();

    // 解析并剪枝
    let config_data: serde_yaml_ng::Value = match serde_yaml_ng::from_str(&body_text) {
        Ok(v) => v,
        Err(e) => {
            error!("YAML 解析失败: {}", e);
            return (StatusCode::OK, body_text).into_response(); // 解析失败则透传原文
        }
    };

    let clean_config = groups::prune(config_data);
    let final_config = rules::inject(clean_config).await;
    let result_yaml = serde_yaml_ng::to_string(&final_config).unwrap_or_default();

    info!("处理完成，正在透传结果...");

    let mut response = Response::builder().status(StatusCode::OK);

    let excluded = [
        "content-encoding",
        "content-length",
        "transfer-encoding",
        "connection",
    ];
    if let Some(headers) = response.headers_mut() {
        for (name, value) in backend_headers.iter() {
            if !excluded.contains(&name.as_str()) {
                headers.insert(name, value.clone());
            }
        }
    }
	info!("{}",UserInfo::from_header(backend_headers));
    info!("响应头已设置，正在发送结果...");

    response
        .body(Body::from(result_yaml))
        .unwrap()
        .into_response()
}
#[derive(Debug,Default)]
struct UserInfo {
    upload: u64,
    download: u64,
    total: u64,
}

impl UserInfo {
    fn from_header(header: HeaderMap) -> Self {
        if let Some(v) = header.get("subscription-userinfo") {
            if let Ok(s) = v.to_str() {
                return Self::from_header_str(s);
            }
        }
        Self::default()
    }
    fn from_header_str(s: &str) -> Self {
        let mut info = Self::default();
        for item in s.split(';') {
            let kv: Vec<&str> = item.split('=').map(|s| s.trim()).collect();
            if kv.len() == 2 {
                match kv[0] {
                    "upload" => info.upload = kv[1].parse().unwrap_or(0),
                    "download" => info.download = kv[1].parse().unwrap_or(0),
                    "total" => info.total = kv[1].parse().unwrap_or(0),
                    _ => {}
                }
            }
        }
        info
    }
}
impl std::fmt::Display for UserInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let used = self.upload + self.download;
        let up_str = format_size(self.upload, BINARY);
        let dl_str = format_size(self.download, BINARY);
        let used_str = format_size(used, BINARY);
        let total_str = format_size(self.total, BINARY);
		writeln!(f)?;
        writeln!(f, "┌────────────────────────────────────────────┐")?;
		writeln!(f, "")?; 
        writeln!(f, "  📤 上传数据 : {:>12}", up_str)?;
        writeln!(f, "  📥 下载数据 : {:>12}", dl_str)?;
        // 这里显示 使用量 / 总额
        writeln!(f, "  💾 流量使用 : {:>12} / {}", used_str, total_str)?;
        write!(f,"└────────────────────────────────────────────┘")
    }
}
