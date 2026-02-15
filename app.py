from flask import Flask, Response, request
import requests
import yaml
from pruner import main_prune

app = Flask(__name__)

# --- 配置 ---
GITHUB_CONFIG_URL = "https://raw.githubusercontent.com/YooRarely/subconverter_config/refs/heads/main/remote_config.toml"
SUB_BACKEND_URL = "http://subconverter.zeabur.internal/sub"

@app.route('/<path:airport_url>')
def smart_proxy(airport_url):
    # 1. 还原机场链接
    full_url = airport_url
    if request.query_string:
        full_url += '?' + request.query_string.decode('utf-8')

    # 2. 【核心修改】直接复制 Clash 发给 Python 的所有 Header
    # 这样 Subconverter 就会看到完整的 User-Agent 和原始请求信息
    forward_headers = dict(request.headers)
    # 移除一些可能冲突的本地 Host 字段
    forward_headers.pop('Host', None)

    params = {
        "target": "clash",
        "url": full_url,
        "config": GITHUB_CONFIG_URL,
        "emoji": "true",
        "list": "false",
        "udp": "true"
    }
    
    try:
        # 3. 带着 Clash 的 Header 去请求 Subconverter
        resp = requests.get(SUB_BACKEND_URL, params=params, headers=forward_headers, timeout=20)
        resp.raise_for_status()
        
        # 4. 剪枝逻辑处理 YAML 文本
        config_data = yaml.safe_load(resp.text)
        clean_config = main_prune(config_data)
        result_yaml = yaml.dump(clean_config, allow_unicode=True, sort_keys=False)
        
        # 5. 【核心修改】构造返回对象，并把 Subconverter 回传的所有 Header 直接复刻
        final_resp = Response(result_yaml, mimetype='text/yaml')
        
        # 排除掉一些敏感或自动生成的 Header (如 Content-Length)
        excluded_headers = ['content-encoding', 'content-length', 'transfer-encoding', 'connection']
        for key, value in resp.headers.items():
            if key.lower() not in excluded_headers:
                final_resp.headers[key] = value
                
        return final_resp
        
    except Exception as e:
        return f"透明代理转换失败: {str(e)}", 500

# 根路由给个提示，防止点进去一片空白
@app.route('/')
def index():
    return "Private Subconverter Service is Running. Append your subscription URL to the address bar."

if __name__ == '__main__':
    app.run(host='0.0.0.0', port=8080)