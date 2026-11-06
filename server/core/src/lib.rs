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

/// 纯业务与 API 路由（不绑定任何前端静态下发方式，供各平台适配器自由组合）
pub fn api_router() -> Router {
    Router::new()
        .route("/refresh", get(refresh_handler))
        .route("/favicon.ico", get(ico))
        .route("/api/admin/info", get(admin::info_handler))
        .route("/api/admin/list", get(admin::list_handler))
        .route("/api/admin/save", post(admin::save_handler))
        .route("/api/admin/delete", post(admin::delete_handler))
        .route("/api/admin/clear_cache", post(admin::clear_cache_handler))
        .route("/api/admin/fetch_local", post(admin::fetch_local_handler))
        .route("/api/admin/probe_live", post(admin::probe_live_handler))
        .route("/api/admin/subscription_content", get(admin::subscription_content_handler))
        .route("/api/admin/login", post(admin::login_handler))
        .route("/{key}", get(subscribe_handler))
}

/// 标准完整路由（本地 / Docker / Render 容器环境使用，静态资源优先动态读磁盘）
pub fn router() -> Router {
    api_router()
        .route("/", get(admin::page_handler))
        .route("/TwemojiCountryFlags.woff2", get(admin::font_handler))
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
    info!("收到客户端订阅请求: key={} (原始路径={})", key, raw_key);

    let query = uri.query().unwrap_or("");
    let force_refresh = query.contains("refresh");

    // 客户端显式携带了 ?clear 参数时直接执行注销
    if query == "clear" || query.starts_with("clear&") {
        registry::unregister(&key).await;
        return (StatusCode::OK, format!("密钥 [{}] 已注销", key)).into_response();
    }

    // 1. 优先按 key 查询数据库（只要存在，绝对是正式订阅，绝不被任何 query 参数破坏）
    if let Some(item) = registry::lookup_item(&key).await {
        // 核心分支 1：真节点快照直发模式 (已缓存真节点)
        // 铁律：100% 纯内存只读转换与直发，0 写库操作，绝无可能被篡改或覆盖！
        if item.is_snapshot_mode() {
            info!("⚡ [真节点快照直发] 命中锁定真节点: {} (包含 {} 个节点, 纯只读极速直发)", key, item.node_count);
            return render_yaml_response(
                &item.raw_yaml,
                force_refresh,
                None,
                Some(&item.user_info),
            ).await;
        }

        // 核心分支 2：实时在线拉取模式 (未缓存真节点)
        info!("🌐 [实时在线拉取] 未缓存真节点快照，向原 URL 实时请求: {} -> {}", key, item.url);
        return process_subscription(headers, &item.url, force_refresh, Some(&key)).await;
    }

    // 2. 数据库中未找到该 key 的情况：
    // 仅当 query 是以 http:// 或 https:// 开头的合法 URL 时，才允许快捷带参注册
    let decoded_query = urlencoding::decode(query).unwrap_or(std::borrow::Cow::Borrowed(query));
    if decoded_query.starts_with("http://") || decoded_query.starts_with("https://") {
        info!("未登记的 key，执行快捷带参注册: {} -> {}", key, decoded_query);
        registry::register(&key, &decoded_query).await;
        return process_subscription(headers, &decoded_query, force_refresh, Some(&key)).await;
    }

    // 既没有该 key，query 也不是合法 URL，规范返回 404
    warn!("未找到登记的订阅: {}", key);
    (
        StatusCode::NOT_FOUND,
        format!("404 Not Found: 未找到登记的订阅短链 [{}]，请先在控制台中添加。", key),
    ).into_response()
}

async fn process_subscription(
    _headers: HeaderMap,
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

    // 统一使用验证最稳定、兼容性最好的 clash.meta UA，不添加任何可能引发 Cloudflare WAF 403 拦截的私有头部
    let req_builder = client
        .get(airport_url)
        .header(header::USER_AGENT, "clash.meta");

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
