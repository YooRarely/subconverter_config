from flask import Flask, Response, request

from urllib.parse import quote, unquote  # <--- 必须包含 quote
import os
import requests
import yaml
from pruner import main_prune

app = Flask(__name__)

# --- 配置 ---

# 1. 订阅后端地址
SUB_BACKEND_URL = os.getenv(
    "SUB_BACKEND", 
    "http://subconverter.zeabur.internal:25500/sub"
)

# 2. 远程配置文件地址
GITHUB_CONFIG_URL = os.getenv(
    "GITHUB_CONFIG_URL", 
    "https://raw.githubusercontent.com/YooRarely/subconverter_config/refs/heads/main/config/remote_config.toml"
)

@app.route('/url')
def proxy_with_query():
# 1. 拿到问号后的原始字符串
    raw_payload = request.query_string.decode('utf-8')
    
    if not raw_payload:
        return "Missing airport URL. Usage: /url?https://...", 400

    # 2. 动态判断：是否需要编码
    # 如果字符串里包含明文的 '://'，说明它没被编码，Subconverter 会报错
    # 我们需要把它变成编码格式
    if "://" in raw_payload:
        # 使用 quote 编码所有特殊字符 (包括 : / ? & =)
        # safe='' 表示没有任何字符是安全的，全都要编码
        target_url = quote(raw_payload, safe='')
    else:
        # 如果已经没有 :// 了，可能已经编码过了
        # 为了防止“部分编码”的情况，保险做法是先 unquote 再统一 quote
        # 但既然你要求“有斜杠就转”，我们保持简单：
        target_url = raw_payload

    # 3. 构造最终发往后端的 URL
    # 注意：这里的 target_url 已经是百分号格式了
    final_url = (
        f"{SUB_BACKEND_URL}?"
        f"target=clash&"
        f"url={target_url}&"
        f"config={GITHUB_CONFIG_URL}&"
        f"emoji=true&list=false&udp=true"
    )

    forward_headers = dict(request.headers)
    forward_headers.pop('Host', None)

    try:
        # 直接发送拼好的字符串，不使用 params 字典
        resp = requests.get(final_url, headers=forward_headers, timeout=20)
        resp.raise_for_status()
        
        # 剪枝逻辑 (保持你原有的 main_prune)
        config_data = yaml.safe_load(resp.text)
        from pruner import main_prune
        clean_config = main_prune(config_data)
        
        return Response(
            yaml.dump(clean_config, allow_unicode=True, sort_keys=False), 
            mimetype='text/yaml'
        )
        
    except Exception as e:
        return f"转换失败: {str(e)}\n最终构造地址: {final_url}", 500


@app.route('/')
def index():
    return "Private Subconverter Service is Running."

if __name__ == '__main__':
    app.run(host='0.0.0.0', port=8080)