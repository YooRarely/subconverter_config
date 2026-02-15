from flask import Flask, Response, request

from urllib.parse import quote_plus  # 必须加上这一行
import os
import requests
import yaml
from pruner import main_prune

app = Flask(__name__)

# --- 配置 ---

# 1. 订阅后端地址
SUB_BACKEND_URL = os.getenv(
    "SUB_BACKEND", 
    "https://yoorarely-subconverter.zeabur.app/sub"
)

# 2. 远程配置文件地址
GITHUB_CONFIG_URL = os.getenv(
    "GITHUB_CONFIG_URL", 
    "https://raw.githubusercontent.com/YooRarely/subconverter_config/refs/heads/main/config/remote_config.toml"
)

@app.route('/<path:airport_url>')
def smart_proxy(airport_url):
    # 1. 拿到机场链接并拼接参数
    full_airport_url = airport_url
    if request.query_string:
        full_airport_url += '?' + request.query_string.decode('utf-8')
    
    # 2. 【核心】强行编码！把 https:// 变成 https%3A%2F%2F
    # 这是 Subconverter 要求的标准格式，不这么写它永远报 400
    safe_airport_url = quote_plus(full_airport_url)

    # 3. 拼接
    final_url = (
        f"{SUB_BACKEND_URL}?"
        f"target=clash&"
        f"url={safe_airport_url}&"
        f"config={GITHUB_CONFIG_URL}&"
        f"emoji=true&list=false&udp=true"
    )

    forward_headers = dict(request.headers)
    forward_headers.pop('Host', None)

    try:
        # 直接发这个拼好的、带百分号的字符串
        resp = requests.get(final_url, headers=forward_headers, timeout=20)
        resp.raise_for_status()
        
        config_data = yaml.safe_load(resp.text)
        clean_config = main_prune(config_data)
        return Response(yaml.dump(clean_config, allow_unicode=True, sort_keys=False), mimetype='text/yaml')
        
    except Exception as e:
        return f"转换失败: {str(e)}\n最终发送给后端的URL: {final_url}", 500
@app.route('/')
def index():
    return "Private Subconverter Service is Running."

if __name__ == '__main__':
    app.run(host='0.0.0.0', port=8080)