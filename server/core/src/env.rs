// SUB_BACKEND=yoorarely-subconverter.zeabur.app
// GITHUB_CONFIG_URL=https://raw.githubusercontent.com/YooRarely/subconverter_config/refs/heads/main/config/remote_config.toml
// PORT=8080
// RUST_LOG=info
// DIRECT_RULES=https://raw.githubusercontent.com/YooRarely/subconverter_config/refs/heads/main/rules/China.list
// GLOBAL_RULES=https://raw.githubusercontent.com/YooRarely/subconverter_config/refs/heads/main/rules/Global.list

use qenv;

qenv::define! {
    SUB_BACKEND:"http://subconverter.zeabur.internal:25500/sub",
    GITHUB_CONFIG_URL:"https://raw.githubusercontent.com/YooRarely/subconverter_config/refs/heads/main/config/remote_config.toml",
    PORT:"8080",
    RUST_LOG:"info",
    DIRECT_RULES:"https://raw.githubusercontent.com/YooRarely/subconverter_config/refs/heads/main/rules/China.list",
    GLOBAL_RULES:"https://raw.githubusercontent.com/YooRarely/subconverter_config/refs/heads/main/rules/Global.list",
	REDIS_CONNECTION_STRING
}
#[test]
fn main() {
    // 1. 初始化
    init().expect("Failed to init qenv");

    println!("--- 🛠️  QENV 配置检查 ---");

    // 2. 测试带默认值的变量

    println!("监听端口: {}", PORT);
    println!("日志级别: {}", RUST_LOG);
    println!("配置地址: {}", GITHUB_CONFIG_URL);

    // 3. 测试必填项 (假设你没在 .env 里设置 SUB_BACKEND)
    match SUB_BACKEND.try_get() {
        Ok(val) => println!("✅ 发现后端地址: {}", val),
        Err(_) => println!("⚠️ SUB_BACKEND 未设置，将使用程序内置逻辑"),
    }

    // 4. 类型转换测试 (尝试把 PORT 转为 String)
    let port_str: String = PORT.take();
    assert_eq!(port_str, "8080");

    println!("--- 🎉 所有配置读取正常 ---");
}
