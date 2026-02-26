use std::collections::HashMap;

use tracing::error;

pub async fn fetch() -> HashMap<String, String> {
    let url = "https://raw.githubusercontent.com/YooRarely/subconverter_config/refs/heads/rust-server/config/man_what_can_i_say";
	let xor_key = std::env::var("XOR_KEY").unwrap_or_else(|_| "default".into());

    let client = reqwest::Client::new();
    if let Ok(res) = client.get(url).send().await {
        if let Ok(hex_content) = res.text().await {
            // 调用函数解密
            let json_str = xor_process(&hex_content, &xor_key, true);
            return serde_json::from_str(&json_str).unwrap_or_default();
        }
    }
    HashMap::new()
}

/// 异或加解密核心函数
/// data: 输入的原始字符串或十六进制密文
/// key: 你在环境变量中设置的密钥字符串
/// is_hex_input: 如果是从 GitHub 下载的密文，设为 true
pub fn xor_process(data: &str, key: &str, is_hex_input: bool) -> String {
    let key_bytes = key.as_bytes();
    
    if is_hex_input {
        // --- 解密流程：Hex String -> Bytes -> XOR -> String ---
        // 每两个字符解析为一个字节
        let bytes: Vec<u8> = (0..data.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&data[i..i + 2], 16).unwrap_or(0))
            .collect();

        let decrypted: Vec<u8> = bytes.iter()
            .zip(key_bytes.iter().cycle())
            .map(|(&d, &k)| d ^ k)
            .collect();
        
        String::from_utf8(decrypted).unwrap_or_else(|_| "解密失败".into())
    } else {
        // --- 加密流程：String -> Bytes -> XOR -> Hex String ---
        data.as_bytes().iter()
            .zip(key_bytes.iter().cycle())
            .map(|(&d, &k)| format!("{:02x}", d ^ k))
            .collect()
    }
}
#[test]
fn encrypt() {
    let json = r#"{"my777": "https://airport-a.com/link"}"#;
    let key = "your_secret_key"; // 你的密钥
    let encrypted = xor_process(json, key, false);
    println!("加密后的文本: {}", encrypted); 
    // 输出类似: 1a2b3c4d...
}
#[test]
fn decrypt(){
	let text = "024d180b68445241484556371f11090a555a5d3e1a17131d1700720a4b1a16025a1e361d0e410f";
	let key = "your_secret_key"; // 你的密钥
    let decrypted = xor_process(text, key, true);
    println!("解密后的文本: {}", decrypted);
}
