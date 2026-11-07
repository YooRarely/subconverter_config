use axum::body::to_bytes;
use hyper::{Request, StatusCode};
use subconverter_core::converter::Converter;
use subconverter_core::router;
use serde_yaml_ng::Value;
use tower::ServiceExt;

#[tokio::test]
async fn test_user_sample_convert() {
    let input = r#"
mixed-port: 7890
allow-lan: true
bind-address: '*'
mode: rule
log-level: info
external-controller: '127.0.0.1:9090'
proxies:
    - { name: 香港01, type: anytls, server: hk01.dw.sh-cloudflare.sbs, port: 11101, password: e9885a09-a998-46f9-885b-1470f21a845c, client-fingerprint: chrome, udp: true, alpn: [h2, http/1.1], sni: hk01.shanhai.me, skip-cert-verify: false }
    - { name: 台湾01 - 彰化, type: vless, server: tw01.test.com, port: 443 }
    - { name: 东京 01 - BGP, type: vless, server: jp01.test.com, port: 443 }
    - { name: US-SiliconValley-01, type: vmess, server: us01.test.com, port: 443 }
    - { name: 某未知神秘节点, type: ss, server: unk01.test.com, port: 8388 }
    - { name: 剩余流量：128GB, type: ss, server: 127.0.0.1, port: 1080 }
"#;

    let converter = Converter::new();
    let result = converter.convert(input, false).await.expect("转换应该成功");

    println!("--- 转换结果 YAML --- \n{}", result);

    let doc: Value = serde_yaml_ng::from_str(&result).expect("解析输出结果应该合法");
    let map = doc.as_mapping().expect("顶级应为 Mapping");

    // 1. 验证原配置字段完整保留
    assert_eq!(map.get(&Value::String("mixed-port".to_string())).and_then(|v| v.as_i64()), Some(7890));
    assert_eq!(map.get(&Value::String("allow-lan".to_string())).and_then(|v| v.as_bool()), Some(true));
    assert_eq!(map.get(&Value::String("bind-address".to_string())).and_then(|v| v.as_str()), Some("*"));
    assert_eq!(map.get(&Value::String("mode".to_string())).and_then(|v| v.as_str()), Some("rule"));
    assert_eq!(map.get(&Value::String("log-level".to_string())).and_then(|v| v.as_str()), Some("info"));

    // 2. 验证 proxies 中的 anytls 等专属字段未丢失
    let proxies = map.get(&Value::String("proxies".to_string())).and_then(|v| v.as_sequence()).unwrap();
    assert_eq!(proxies.len(), 6);
    let hk_node = &proxies[0];
    assert_eq!(hk_node.get(&Value::String("name".to_string())).and_then(|v| v.as_str()), Some("香港01"));
    assert_eq!(hk_node.get(&Value::String("type".to_string())).and_then(|v| v.as_str()), Some("anytls"));
    assert_eq!(hk_node.get(&Value::String("client-fingerprint".to_string())).and_then(|v| v.as_str()), Some("chrome"));
    assert_eq!(hk_node.get(&Value::String("skip-cert-verify".to_string())).and_then(|v| v.as_bool()), Some(false));

    // 3. 验证 proxy-groups 结构
    let groups = map.get(&Value::String("proxy-groups".to_string())).and_then(|v| v.as_sequence()).unwrap();
    let group_names: Vec<String> = groups.iter()
        .filter_map(|g| g.get(&Value::String("name".to_string())).and_then(|v| v.as_str()).map(|s| s.to_string()))
        .collect();

    // 主选择组在最前
    assert_eq!(group_names[0], "🚀 节点选择");

    // 必须包含香港、台湾、日本、美国、剩余流量、以及未知
    assert!(group_names.iter().any(|g| g.contains("香港")));
    assert!(group_names.iter().any(|g| g.contains("台湾")));
    assert!(group_names.iter().any(|g| g.contains("日本")));
    assert!(group_names.iter().any(|g| g.contains("美国")));
    assert!(group_names.iter().any(|g| g.contains("剩余流量")));
    assert!(group_names.contains(&"❓ 未知".to_string()));

    // 没有节点的组（如 🇨🇳 中国, 🇬🇧 英国 等）绝不应该生成
    assert!(!group_names.iter().any(|g| g.contains("中国")));
    assert!(!group_names.iter().any(|g| g.contains("英国")));

    // 验证主组包含了这些策略组
    let main_group = &groups[0];
    let main_proxies = main_group.get(&Value::String("proxies".to_string())).and_then(|v| v.as_sequence()).unwrap();
    let main_proxy_names: Vec<&str> = main_proxies.iter().filter_map(|v| v.as_str()).collect();
    assert!(main_proxy_names.iter().any(|p| p.contains("香港")));
    assert!(main_proxy_names.iter().any(|p| p.contains("台湾")));
    assert!(main_proxy_names.contains(&"❓ 未知"));

    // 4. 验证 rules 存在并包含核心条目
    let rules = map.get(&Value::String("rules".to_string())).and_then(|v| v.as_sequence()).unwrap();
    let rule_strs: Vec<&str> = rules.iter().filter_map(|v| v.as_str()).collect();

    println!("\n=== Generated Clash Rules Preview (first 10) ===");
    for (idx, r) in rule_strs.iter().take(10).enumerate() {
        println!("  [{}] {}", idx + 1, r);
    }

    assert!(rule_strs.iter().any(|r| r.ends_with(",DIRECT")));
    assert!(rule_strs.iter().any(|r| r.ends_with(",🚀 节点选择")));
    assert!(rule_strs.iter().any(|r| r.ends_with(",REJECT")));
    assert!(rule_strs.contains(&"GEOIP,CN,DIRECT,no-resolve"));
    assert_eq!(rule_strs.last(), Some(&"MATCH,🚀 节点选择"));
}

#[tokio::test]
async fn test_router_endpoints() {
    let _ = subconverter_core::env::init();
    let app = router();

    // 1. GET /
    let res = app.clone()
        .oneshot(Request::builder().uri("/").body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = to_bytes(res.into_body(), 64 * 1024).await.unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(body.contains("Subconverter"));

    // 2. GET /favicon.ico
    let res = app.clone()
        .oneshot(Request::builder().uri("/favicon.ico").body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 2.1 GET /TwemojiCountryFlags.woff2
    let res = app.clone()
        .oneshot(Request::builder().uri("/TwemojiCountryFlags.woff2").body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 3. GET /url 不再是特殊路由，而是作为正常的 key 供订阅 (未注册报 404)
    let res = app.clone()
        .oneshot(Request::builder().uri("/url").body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // 4. GET /unregistered_key 未注册报 404
    let res = app.clone()
        .oneshot(Request::builder().uri("/unregistered_dummy_key_123").body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // 5. GET /test_key?clear 返回 200
    let res = app.clone()
        .oneshot(Request::builder().uri("/test_key?clear").body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 6. GET /refresh 主动刷新规则测试 返回 200
    let res = app.clone()
        .oneshot(Request::builder().uri("/refresh").body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 7. GET /admin 不再是控制台路由，而是作为正常的 key 供客户端订阅 (未注册报 404)
    let res = app.clone()
        .oneshot(Request::builder().uri("/admin").body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // 8. GET /api/admin/info 返回 200 JSON
    let res = app.clone()
        .oneshot(Request::builder().uri("/api/admin/info").body(axum::body::Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}
