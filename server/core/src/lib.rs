use axum::{
    Router,
    body::Body,
    extract::{OriginalUri, Path},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use humansize::{BINARY, format_size};
use hyper::{HeaderMap, StatusCode, header};
use std::sync::Arc;
use tokio::sync::OnceCell;
use tracing::{error, info, warn};

pub mod admin;
pub mod classifier;
pub mod converter;
pub mod env;
pub mod registry;
pub mod rules;
pub mod store;

static CONVERTER: OnceCell<Arc<converter::Converter>> = OnceCell::const_new();

pub(crate) async fn get_converter() -> &'static Arc<converter::Converter> {
    CONVERTER
        .get_or_init(|| async { Arc::new(converter::Converter::new()) })
        .await
}

pub fn router() -> Router {
    Router::new()
        .route("/", get(admin::page_handler))
        .route("/refresh", get(refresh_handler))
        .route("/url", get(url_handler))
        .route("/favicon.ico", get(ico))
        .route("/TwemojiCountryFlags.woff2", get(admin::font_handler))
        .route("/api/admin/info", get(admin::info_handler))
        .route("/api/admin/list", get(admin::list_handler))
        .route("/api/admin/save", post(admin::save_handler))
        .route("/api/admin/delete", post(admin::delete_handler))
        .route("/api/admin/clean_yaml", post(admin::clean_yaml_handler))
        .route("/api/admin/clear_cache", post(admin::clear_cache_handler))
        .route("/api/admin/fetch_local", post(admin::fetch_local_handler))
        .route("/api/admin/probe_live", post(admin::probe_live_handler))
        .route("/api/admin/subscription_content", get(admin::subscription_content_handler))
        .route("/api/admin/login", post(admin::login_handler))
        .route("/{key}", get(subscribe_handler))
}

async fn ico() {}

/// 主动刷新私人规则接口：访问后立即从 GitHub 重新拉取 China.list 与 Global.list
async fn refresh_handler() -> impl IntoResponse {
    info!("--- 收到主动刷新私人规则请求 ---");
    let (china_ok, global_ok) = rules::RulesBuilder::refresh_private_rules().await;
    let msg = format!(
        "🔄 私人规则已完成主动刷新！\n- China.list: {}\n- Global.list: {}\n\n现在刷新客户端订阅即可立刻应用最新规则，无需重新构建服务！",
        if china_ok { "成功更新 (已从 GitHub 拉取最新)" } else { "未更新 (已使用本地文件或内置数据)" },
        if global_ok { "成功更新 (已从 GitHub 拉取最新)" } else { "未更新 (已使用本地文件或内置数据)" }
    );
    (StatusCode::OK, msg)
}

async fn subscribe_handler(
    Path(raw_key): Path<String>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> impl IntoResponse {
    let key = urlencoding::decode(&raw_key)
        .unwrap_or(std::borrow::Cow::Borrowed(&raw_key))
        .to_string();
    info!("尝试匹配路径密钥: {} (原始: {})", key, raw_key);
    let query = uri.query().unwrap_or("");
    let force_refresh = query.contains("refresh");

    match query {
        "" | "refresh" => match registry::lookup_item(&key).await {
            Some(item) => {
                if item.is_snapshot_mode() {
                    // 已开启真节点快照直发模式：直接用保存的真节点底包重构输出（全绿、毫秒级极速响应、无风控，并带上保存的流量响应头）
                    info!("⚡ [真节点快照直发] 命中真节点底包直接重构输出: {} (包含 {} 个节点)", key, item.node_count);
                    render_yaml_response(&item.raw_yaml, force_refresh, None, Some(&item.user_info)).await
                } else {
                    // 实时在线拉取模式：向原 URL 实时拉取并重构输出，顺便记录/更新状态看板（绝不冻结/缓存 YAML）
                    info!("🌐 [实时在线拉取] 未缓存真节点快照，向原 URL 实时请求: {} -> {}", key, item.url);
                    process_subscription(headers, &item.url, force_refresh, Some(&key)).await
                }
            }
            None => {
                warn!("未找到订阅: {}", key);
                (
                    StatusCode::NOT_FOUND,
                    format!("未找到登记的订阅: {}\n请先在管理后台 /admin 中添加该订阅。", key),
                )
                    .into_response()
            }
        },
        "clear" => {
            registry::unregister(&key).await;
            (StatusCode::OK, format!("密钥 [{}] 已注销", key)).into_response()
        }
        _ => {
            // 兼容快捷带参注册
            let decoded_url =
                urlencoding::decode(query).unwrap_or_else(|_| std::borrow::Cow::Borrowed(query));
            registry::register(&key, &decoded_url).await;
            process_subscription(headers, &decoded_url, force_refresh, Some(&key)).await
        }
    }
}

async fn url_handler(headers: HeaderMap, OriginalUri(uri): OriginalUri) -> impl IntoResponse {
    info!("--- 收到新 /url 请求 ---");
    let raw_query = uri.query().unwrap_or("");
    if raw_query.is_empty() {
        warn!("请求失败: 未提供机场订阅 URL");
        return (StatusCode::BAD_REQUEST, "Missing airport URL.").into_response();
    }

    let force_refresh = raw_query.contains("refresh");
    let decoded_url =
        urlencoding::decode(raw_query).unwrap_or_else(|_| std::borrow::Cow::Borrowed(raw_query));
    process_subscription(headers, &decoded_url, force_refresh, None).await
}

async fn process_subscription(
    headers: HeaderMap,
    airport_url: &str,
    force_refresh: bool,
    bind_key: Option<&str>,
) -> Response {
    let airport_url = airport_url.trim();
    info!("开始下载机场订阅: {} (强制刷新规则: {})", airport_url, force_refresh);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let mut req_builder = client.get(airport_url);

    let client_ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // 若客户端是浏览器或普通工具，统一使用 clash.meta 确保机场返回标准 YAML
    let airport_ua = if client_ua.to_lowercase().contains("clash")
        || client_ua.to_lowercase().contains("meta")
        || client_ua.to_lowercase().contains("mihomo")
    {
        client_ua
    } else {
        "clash.meta"
    };
    req_builder = req_builder.header(header::USER_AGENT, airport_ua);

    // 透传客户端的真实 IP 头部，解除机场对海外云端机房 (Render 等) 的风控拦截
    for ip_header in &["x-forwarded-for", "cf-connecting-ip", "x-real-ip"] {
        if let Some(val) = headers.get(*ip_header) {
            req_builder = req_builder.header(*ip_header, val);
        }
    }

    let backend_res = match req_builder.send().await {
        Ok(r) => r,
        Err(e) => {
            error!("下载机场订阅失败: {}", e);
            if let Some(k) = bind_key {
                registry::record_fetch_error(k, &format!("连接失败: {}", e)).await;
            }
            return (
                StatusCode::BAD_GATEWAY,
                format!("Failed to fetch airport subscription: {}", e),
            )
                .into_response();
        }
    };

    let backend_status = StatusCode::from_u16(backend_res.status().as_u16())
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let backend_headers = backend_res.headers().clone();

    if !backend_status.is_success() {
        let err_body = backend_res.text().await.unwrap_or_default();
        error!("机场返回错误状态码 ({}): {}", backend_status, err_body);
        if let Some(k) = bind_key {
            registry::record_fetch_error(k, &format!("HTTP {}", backend_status)).await;
        }
        return (backend_status, err_body).into_response();
    }

    // 打印流量信息（如有）
    let user_info = UserInfo::from_header(&backend_headers);
    if user_info.is_present() {
        info!("{}", user_info);
    }

    let body_text = match backend_res.text().await {
        Ok(t) => t,
        Err(e) => {
            error!("读取订阅响应体失败: {}", e);
            if let Some(k) = bind_key {
                registry::record_fetch_error(k, "读取内容失败").await;
            }
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Read body error: {}", e),
            )
                .into_response();
        }
    };

    // 模式 A 顺手更新状态看板：记录节点数、流量信息与最新文件缓存（供后台纯读查看，不影响动态拉取）
    if let Some(k) = bind_key {
        let (cleaned_yaml, node_count) = match registry::clean_raw_yaml(&body_text) {
            Ok((cleaned, count)) => (cleaned, count),
            Err(_) => (String::new(), 0),
        };
        let user_info_str = backend_headers
            .get("subscription-userinfo")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        registry::record_fetch_success(k, node_count, user_info_str, &cleaned_yaml).await;
    }

    info!("订阅下载成功 (大小: {} 字节)，开始执行本地转换...", body_text.len());

    render_yaml_response(&body_text, force_refresh, Some(&backend_headers), None).await
}

/// 核心重构与响应渲染：动态注入国家 select 策略分组与最新规则，并透传流量信息
async fn render_yaml_response(
    raw_yaml: &str,
    force_refresh: bool,
    backend_headers: Option<&HeaderMap>,
    cached_user_info: Option<&str>,
) -> Response {
    let converter = get_converter().await;
    let final_yaml = match converter.convert(raw_yaml, force_refresh).await {
        Ok(yaml) => yaml,
        Err(e) => {
            warn!("转换 YAML 出现异常 (将透传原配置): {}", e);
            raw_yaml.to_string()
        }
    };

    let mut response = Response::builder().status(StatusCode::OK);

    // 设置响应头（透传流量信息等，排除 hop-by-hop 头部）
    let excluded = [
        "content-encoding",
        "content-length",
        "transfer-encoding",
        "connection",
    ];

    if let Some(res_headers) = response.headers_mut() {
        if let Some(orig_headers) = backend_headers {
            for (name, value) in orig_headers.iter() {
                if !excluded.contains(&name.as_str()) {
                    res_headers.insert(name, value.clone());
                }
            }
        } else if let Some(user_info) = cached_user_info {
            if !user_info.trim().is_empty() {
                if let Ok(v) = user_info.parse() {
                    res_headers.insert("subscription-userinfo", v);
                }
            }
        }
        res_headers.insert(
            header::CONTENT_TYPE,
            "text/yaml; charset=utf-8".parse().unwrap(),
        );
    }

    response.body(Body::from(final_yaml)).unwrap().into_response()
}

#[derive(Debug, Default)]
struct UserInfo {
    upload: u64,
    download: u64,
    total: u64,
    found: bool,
}

impl UserInfo {
    fn is_present(&self) -> bool {
        self.found
    }

    fn from_header(header: &HeaderMap) -> Self {
        if let Some(v) = header.get("subscription-userinfo") {
            if let Ok(s) = v.to_str() {
                let mut info = Self::from_header_str(s);
                info.found = true;
                return info;
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
        writeln!(f, "  💾 流量使用 : {:>12} / {}", used_str, total_str)?;
        write!(f, "└────────────────────────────────────────────┘")
    }
}
