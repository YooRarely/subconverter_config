use axum::{
    Json,
    extract::Query,
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::env;
use crate::registry::{self, SubscriptionItem};
use crate::store;

/// 认证检查：若设置了 ADMIN_PASSWORD 环境变量，必须提供匹配的 Token
pub fn is_authorized(headers: &HeaderMap, query_token: Option<&str>) -> bool {
    let required_pwd = env::ADMIN_PASSWORD;
    let required_pwd = required_pwd.trim();
    if required_pwd.is_empty() {
        return true;
    }

    if let Some(token) = query_token {
        if token.trim() == required_pwd {
            return true;
        }
    }

    if let Some(auth_val) = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        if let Some(token) = auth_val.strip_prefix("Bearer ") {
            if token.trim() == required_pwd {
                return true;
            }
        }
    }

    if let Some(token_val) = headers.get("x-admin-token").and_then(|v| v.to_str().ok()) {
        if token_val.trim() == required_pwd {
            return true;
        }
    }

    false
}

#[derive(Serialize)]
pub struct SystemInfo {
    pub redis_connected: bool,
    pub auth_required: bool,
}

pub async fn info_handler() -> impl IntoResponse {
    let required_pwd = env::ADMIN_PASSWORD;
    let auth_required = !required_pwd.trim().is_empty();
    let redis_connected = store::cache().await.is_redis_connected();

    Json(SystemInfo {
        redis_connected,
        auth_required,
    })
}

/// 获取分流规则链各规则文件的结构化详情（按文件分类，供控制台可视化规则面板展示）
pub async fn rules_overview_handler(
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
) -> Result<Json<crate::rules::RulesOverviewResponse>, (StatusCode, &'static str)> {
    if !is_authorized(&headers, query.token.as_deref()) {
        return Err((StatusCode::UNAUTHORIZED, "管理员密码错误或未提供"));
    }

    let overview = crate::rules::RulesBuilder::get_rules_overview().await;
    Ok(Json(overview))
}

#[derive(Deserialize)]
pub struct LoginPayload {
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub success: bool,
    pub message: String,
    pub token: String,
}

pub async fn login_handler(Json(payload): Json<LoginPayload>) -> impl IntoResponse {
    let required_pwd = env::ADMIN_PASSWORD;
    let required_pwd = required_pwd.trim();

    if required_pwd.is_empty() {
        return Json(LoginResponse {
            success: true,
            message: "未设置密码，直接免密通过".to_string(),
            token: String::new(),
        });
    }

    if payload.password.trim() == required_pwd {
        Json(LoginResponse {
            success: true,
            message: "验证通过".to_string(),
            token: required_pwd.to_string(),
        })
    } else {
        Json(LoginResponse {
            success: false,
            message: "密码错误，请重新输入".to_string(),
            token: String::new(),
        })
    }
}

#[derive(Deserialize)]
pub struct AuthQuery {
    pub token: Option<String>,
}

pub async fn list_handler(
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
) -> Result<Json<Vec<SubscriptionItem>>, (StatusCode, &'static str)> {
    if !is_authorized(&headers, query.token.as_deref()) {
        return Err((StatusCode::UNAUTHORIZED, "管理员密码错误或未提供"));
    }
    let list = registry::list_all().await;
    Ok(Json(list))
}

#[derive(Deserialize)]
pub struct SavePayload {
    pub key: String,
    pub url: String,
    pub raw_yaml: Option<String>,
    pub user_info: Option<String>,
    pub is_manual_cached: Option<bool>,
}

#[derive(Serialize)]
pub struct CommonResponse {
    pub success: bool,
    pub message: String,
}

pub async fn save_handler(
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    Json(payload): Json<SavePayload>,
) -> Result<Json<CommonResponse>, (StatusCode, &'static str)> {
    if !is_authorized(&headers, query.token.as_deref()) {
        return Err((StatusCode::UNAUTHORIZED, "管理员密码错误或未提供"));
    }

    let key = payload.key.trim().to_string();
    let new_url = payload.url.trim().to_string();

    if key.is_empty() {
        return Ok(Json(CommonResponse {
            success: false,
            message: "订阅短链 Key 不能为空".to_string(),
        }));
    }

    let existing_opt = registry::lookup_item(&key).await;

    // 场景 A：纯表单录入/更新 URL（未显式传入 raw_yaml 快照数据）
    if payload.raw_yaml.is_none() {
        if let Some(ref existing) = existing_opt {
            if existing.url.trim() == new_url {
                // 1. URL 没有任何变化，服务器什么也不做
                return Ok(Json(CommonResponse {
                    success: true,
                    message: format!("订阅 [{}] 的 URL 未发生变化，配置保持原样不变", key),
                }));
            } else {
                // 2. URL 发生了变化，等同于全新添加，彻底清空重置旧快照与旧流量状态！
                let item = SubscriptionItem {
                    key: key.clone(),
                    url: new_url,
                    raw_yaml: String::new(),
                    user_info: String::new(),
                    updated_at: 0,
                    node_count: 0,
                    is_manual_cached: false,
                    last_status: "pending".to_string(),
                };
                registry::save_subscription(item).await;
                return Ok(Json(CommonResponse {
                    success: true,
                    message: format!("订阅 [{}] 的 URL 已更新！关联的旧节点快照与流量数据已彻底清空重置。", key),
                }));
            }
        } else {
            // 3. 全新添加条目
            let item = SubscriptionItem {
                key: key.clone(),
                url: new_url,
                raw_yaml: String::new(),
                user_info: String::new(),
                updated_at: 0,
                node_count: 0,
                is_manual_cached: false,
                last_status: "pending".to_string(),
            };
            registry::save_subscription(item).await;
            return Ok(Json(CommonResponse {
                success: true,
                message: format!("订阅 [{}] 添加成功！(当前为实时在线拉取)", key),
            }));
        }
    }

    // 场景 B：显式传入了 raw_yaml（如列表点击「更新快照」或保存节点快照）
    let mut raw_yaml = String::new();
    let mut node_count = 0;

    if let Some(input_yaml) = payload.raw_yaml {
        let input_yaml = input_yaml.trim();
        if !input_yaml.is_empty() {
            match registry::clean_raw_yaml(input_yaml) {
                Ok((cleaned, count)) => {
                    raw_yaml = cleaned;
                    node_count = count;
                }
                Err(e) => {
                    return Ok(Json(CommonResponse {
                        success: false,
                        message: format!("清洗 YAML 失败: {}", e),
                    }));
                }
            }
        }
    }

    // 流量信息智能补全：优先使用传入值 -> 继承已有流量 -> 向机场URL快速探活捕获最新流量响应头
    let mut user_info = if let Some(info) = payload.user_info {
        info
    } else if let Some(ref existing) = existing_opt {
        existing.user_info.clone()
    } else {
        String::new()
    };

    // 若流量为空且具备机场原链，向机场请求一次获取最新的 Subscription-Userinfo 响应头 (机场不风控流量响应头)
    if user_info.trim().is_empty() && !new_url.is_empty() {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(6))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        if let Ok(resp) = client.get(&new_url).header(header::USER_AGENT, "clash.meta").send().await {
            if let Some(info_val) = resp.headers().get("subscription-userinfo").and_then(|v| v.to_str().ok()) {
                user_info = info_val.to_string();
                info!("已自动为快照订阅 [{}] 捕获最新流量响应头: {}", key, user_info);
            }
        }
    }

    let is_manual_cached = payload.is_manual_cached.unwrap_or(!raw_yaml.is_empty());

    let item = SubscriptionItem {
        key: key.clone(),
        url: new_url,
        raw_yaml,
        user_info,
        updated_at: 0,
        node_count,
        is_manual_cached,
        last_status: "ok".to_string(),
    };

    registry::save_subscription(item).await;

    let desc = if is_manual_cached && node_count > 0 {
        format!("已开启真节点快照直发模式 (已缓存 {} 个节点)", node_count)
    } else {
        "已配置为实时在线拉取模式 (客户端请求时实时向机场获取)".to_string()
    };

    Ok(Json(CommonResponse {
        success: true,
        message: format!("订阅 [{}] 快照更新成功！{}", key, desc),
    }))
}

#[derive(Deserialize)]
pub struct ContentQuery {
    pub key: String,
    pub token: Option<String>,
}

#[derive(Serialize)]
pub struct GroupView {
    pub name: String,
    pub group_type: String,
    pub proxies: Vec<String>,
}

#[derive(Serialize)]
pub struct ProxyNodeView {
    pub name: String,
    pub proxy_type: String,
    pub server: String,
    pub port: u16,
}

#[derive(Serialize)]
pub struct ContentResponse {
    pub success: bool,
    pub key: String,
    pub has_content: bool,
    pub content: String,
    pub node_count: usize,
    pub is_manual_cached: bool,
    pub groups: Vec<GroupView>,
    pub proxies: Vec<ProxyNodeView>,
    pub message: String,
}

/// 纯读本地/Redis缓存的订阅文件内容，并按策略组与归属节点结构化返回（绝不发起外部抓取，毫秒级内存重构）
pub async fn subscription_content_handler(
    headers: HeaderMap,
    Query(query): Query<ContentQuery>,
) -> Result<Json<ContentResponse>, (StatusCode, &'static str)> {
    if !is_authorized(&headers, query.token.as_deref()) {
        return Err((StatusCode::UNAUTHORIZED, "管理员密码错误或未提供"));
    }

    let key = query.key.trim();
    if let Some(item) = registry::lookup_item(key).await {
        let has_content = !item.raw_yaml.trim().is_empty();

        let (groups, proxies_list, final_yaml) = if has_content {
            let converter = crate::get_converter().await;
            match converter.convert(&item.raw_yaml, false).await {
                Ok(full_yaml) => {
                    let mut groups_vec = Vec::new();
                    let mut proxies_vec = Vec::new();

                    if let Ok(val) = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&full_yaml) {
                        // 1. 提取所有策略组及组内包含的节点名
                        if let Some(groups_seq) = val.get("proxy-groups").and_then(|v| v.as_sequence()) {
                            for g in groups_seq {
                                let name = g.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                                let group_type = g.get("type").and_then(|t| t.as_str()).unwrap_or("select").to_string();
                                let mut proxies = Vec::new();
                                if let Some(px_seq) = g.get("proxies").and_then(|p| p.as_sequence()) {
                                    for p in px_seq {
                                        if let Some(p_str) = p.as_str() {
                                            proxies.push(p_str.to_string());
                                        }
                                    }
                                }
                                groups_vec.push(GroupView { name, group_type, proxies });
                            }
                        }

                        // 2. 提取所有物理节点详情（名字、协议、服务器域名、端口）
                        if let Some(proxies_seq) = val.get("proxies").and_then(|v| v.as_sequence()) {
                            for p in proxies_seq {
                                let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                                let proxy_type = p.get("type").and_then(|t| t.as_str()).unwrap_or("").to_string();
                                let server = p.get("server").and_then(|s| s.as_str()).unwrap_or("").to_string();
                                let port = p.get("port").and_then(|pt| pt.as_u64()).unwrap_or(0) as u16;
                                proxies_vec.push(ProxyNodeView { name, proxy_type, server, port });
                            }
                        }
                    }

                    (groups_vec, proxies_vec, full_yaml)
                }
                Err(_) => (Vec::new(), Vec::new(), item.raw_yaml.clone()),
            }
        } else {
            (Vec::new(), Vec::new(), String::new())
        };

        let node_count = if proxies_list.is_empty() { item.node_count } else { proxies_list.len() };

        Ok(Json(ContentResponse {
            success: true,
            key: key.to_string(),
            has_content,
            content: final_yaml,
            node_count,
            is_manual_cached: item.is_manual_cached,
            groups,
            proxies: proxies_list,
            message: if has_content {
                "成功读取已缓存的订阅内容".to_string()
            } else {
                "当前订阅尚未生成或缓存订阅文件内容".to_string()
            },
        }))
    } else {
        Ok(Json(ContentResponse {
            success: false,
            key: key.to_string(),
            has_content: false,
            content: String::new(),
            node_count: 0,
            is_manual_cached: false,
            groups: Vec::new(),
            proxies: Vec::new(),
            message: format!("未找到订阅 [{}]", key),
        }))
    }
}

#[derive(Deserialize)]
pub struct ProbeLivePayload {
    pub key: String,
    pub url: String,
}

/// 专为在线实时拉取模式提供的探测接口：仅向机场拉取节点数与流量并更新看板，绝不冻结/缓存 YAML！
pub async fn probe_live_handler(
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    Json(payload): Json<ProbeLivePayload>,
) -> Result<Json<CommonResponse>, (StatusCode, &'static str)> {
    if !is_authorized(&headers, query.token.as_deref()) {
        return Err((StatusCode::UNAUTHORIZED, "管理员密码错误或未提供"));
    }

    let key = payload.key.trim();
    let url = payload.url.trim();
    if key.is_empty() || url.is_empty() {
        return Ok(Json(CommonResponse {
            success: false,
            message: "Key 和 URL 不能为空".to_string(),
        }));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let resp = match client
        .get(url)
        .header(header::USER_AGENT, "clash.meta")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            let err_msg = format!("探测连接失败: {}", e);
            registry::record_fetch_error(key, &err_msg).await;
            return Ok(Json(CommonResponse {
                success: false,
                message: err_msg,
            }));
        }
    };

    if !resp.status().is_success() {
        let err_msg = format!("机场返回 HTTP {}", resp.status());
        registry::record_fetch_error(key, &err_msg).await;
        return Ok(Json(CommonResponse {
            success: false,
            message: err_msg,
        }));
    }

    let user_info_str = resp
        .headers()
        .get("subscription-userinfo")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let body_text = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            let err_msg = format!("读取内容失败: {}", e);
            registry::record_fetch_error(key, &err_msg).await;
            return Ok(Json(CommonResponse {
                success: false,
                message: err_msg,
            }));
        }
    };

    let (cleaned_yaml, node_count) = match registry::clean_raw_yaml(&body_text) {
        Ok((cleaned, count)) => (cleaned, count),
        Err(_) => (String::new(), 0),
    };

    // 关键：更新看板元数据，同时把清洗后的节点内容存为查看缓存，供详情页面纯读展示！
    registry::record_fetch_success(key, node_count, &user_info_str, &cleaned_yaml).await;

    Ok(Json(CommonResponse {
        success: true,
        message: format!("在线探测成功！当前包含 {} 个节点，订阅文件已同步缓存", node_count),
    }))
}

#[derive(Deserialize)]
pub struct ClearCachePayload {
    pub key: String,
}

pub async fn clear_cache_handler(
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    Json(payload): Json<ClearCachePayload>,
) -> Result<Json<CommonResponse>, (StatusCode, &'static str)> {
    if !is_authorized(&headers, query.token.as_deref()) {
        return Err((StatusCode::UNAUTHORIZED, "管理员密码错误或未提供"));
    }

    let ok = registry::clear_subscription_cache(&payload.key).await;
    Ok(Json(CommonResponse {
        success: ok,
        message: if ok {
            format!("订阅 [{}] 的节点缓存已清空，变更为实时在线拉取状态", payload.key)
        } else {
            format!("未找到订阅 [{}]", payload.key)
        },
    }))
}

#[derive(Deserialize)]
pub struct DeletePayload {
    pub key: String,
}

pub async fn delete_handler(
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    Json(payload): Json<DeletePayload>,
) -> Result<Json<CommonResponse>, (StatusCode, &'static str)> {
    if !is_authorized(&headers, query.token.as_deref()) {
        return Err((StatusCode::UNAUTHORIZED, "管理员密码错误或未提供"));
    }

    registry::unregister(&payload.key).await;

    Ok(Json(CommonResponse {
        success: true,
        message: format!("订阅 [{}] 已删除", payload.key),
    }))
}

#[derive(Deserialize)]
pub struct FetchLocalPayload {
    pub url: String,
}

#[derive(Serialize)]
pub struct FetchLocalResponse {
    pub success: bool,
    pub message: String,
    pub cleaned_yaml: String,
    pub node_count: usize,
    pub user_info: String,
}

/// 专为模式 B 提供的后端抓取接口：使用当前环境网络并伪装 clash.meta UA，避免浏览器 Forbidden header 与跨域限制
pub async fn fetch_local_handler(
    headers: HeaderMap,
    Query(query): Query<AuthQuery>,
    Json(payload): Json<FetchLocalPayload>,
) -> Result<Json<FetchLocalResponse>, (StatusCode, &'static str)> {
    if !is_authorized(&headers, query.token.as_deref()) {
        return Err((StatusCode::UNAUTHORIZED, "管理员密码错误或未提供"));
    }

    let raw_url = payload.url.trim();
    // 自动解转义：若用户不小心传入了被转义的 URL (如含有 %3A%2F%2F)，自动解码为原始 URL
    let airport_url = if raw_url.contains("%3A%2F%2F") || raw_url.contains("%3a%2f%2f") {
        urlencoding::decode(raw_url)
            .unwrap_or(std::borrow::Cow::Borrowed(raw_url))
            .to_string()
    } else {
        raw_url.to_string()
    };

    if !airport_url.starts_with("http://") && !airport_url.starts_with("https://") {
        return Ok(Json(FetchLocalResponse {
            success: false,
            message: "订阅 URL 必须以 http:// 或 https:// 开头".to_string(),
            cleaned_yaml: String::new(),
            node_count: 0,
            user_info: String::new(),
        }));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    // 使用 clash.meta UA 发起请求，确保机场面板识别为 Clash 客户端下发标准 YAML，绝不下发 Base64 纯文本
    let resp = match client
        .get(&airport_url)
        .header(header::USER_AGENT, "clash.meta")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return Ok(Json(FetchLocalResponse {
                success: false,
                message: format!("请求机场订阅失败: {}", e),
                cleaned_yaml: String::new(),
                node_count: 0,
                user_info: String::new(),
            }));
        }
    };

    if !resp.status().is_success() {
        return Ok(Json(FetchLocalResponse {
            success: false,
            message: format!("机场返回错误状态码: {}", resp.status()),
            cleaned_yaml: String::new(),
            node_count: 0,
            user_info: String::new(),
        }));
    }

    // 提取机场下发的流量信息响应头
    let user_info = resp
        .headers()
        .get("subscription-userinfo")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let body_text = match resp.text().await {
        Ok(t) => t,
        Err(e) => {
            return Ok(Json(FetchLocalResponse {
                success: false,
                message: format!("读取机场响应内容失败: {}", e),
                cleaned_yaml: String::new(),
                node_count: 0,
                user_info: String::new(),
            }));
        }
    };

    // 执行清洗，剔除旧的 proxy-groups 和 rules
    match registry::clean_raw_yaml(&body_text) {
        Ok((cleaned, count)) => Ok(Json(FetchLocalResponse {
            success: true,
            message: format!("成功提取并清洗出 {} 个真实节点！", count),
            cleaned_yaml: cleaned,
            node_count: count,
            user_info,
        })),
        Err(e) => {
            let preview: String = body_text.chars().take(120).collect();
            Ok(Json(FetchLocalResponse {
                success: false,
                message: format!("清洗失败: {} (返回内容预览: {})", e, preview),
                cleaned_yaml: String::new(),
                node_count: 0,
                user_info: String::new(),
            }))
        }
    }
}

/// 优先从 server/web/TwemojiCountryFlags.woff2 读取静态字体文件
pub async fn font_handler() -> impl IntoResponse {
    let candidates = [
        "server/web/TwemojiCountryFlags.woff2",
        "web/TwemojiCountryFlags.woff2",
        "../web/TwemojiCountryFlags.woff2",
        "../../web/TwemojiCountryFlags.woff2",
    ];

    for path in &candidates {
        if let Ok(bytes) = tokio::fs::read(path).await {
            return (
                [
                    (header::CONTENT_TYPE, "font/woff2"),
                    (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
                ],
                bytes,
            ).into_response();
        }
    }

    (StatusCode::NOT_FOUND, "404 Not Found: 静态字体文件缺失").into_response()
}

/// 优先从 server/web/index.html 读取静态 HTML，热更新立即可见
pub async fn page_handler() -> impl IntoResponse {
    let candidates = [
        "server/web/index.html",
        "web/index.html",
        "../web/index.html",
        "../../web/index.html",
    ];

    for path in &candidates {
        if let Ok(html) = tokio::fs::read_to_string(path).await {
            return (
                [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                html,
            ).into_response();
        }
    }

    (
        StatusCode::NOT_FOUND,
        "404 Not Found: server/web/index.html 缺失，请检查 web 目录静态资源。",
    ).into_response()
}