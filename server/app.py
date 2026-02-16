from flask import Flask, Response, request
from urllib.parse import unquote, quote
import logging
import os
import requests
import yaml
from pruner import main_prune

app = Flask(__name__)


# 配置日志：在 Vercel 控制台可以直接看到 stdout
logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

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
    raw_payload = request.query_string.decode('utf-8')
    logger.info(f"--- 新请求收到 ---")
    logger.info(f"原始 Query String: {raw_payload}")
    
    if not raw_payload:
        logger.warning("请求失败: 未提供机场 URL")
        return "Missing airport URL. Usage: /url?https://...", 400
    target_url = quote(unquote(raw_payload),safe='')
    final_url = (
        f"{SUB_BACKEND_URL}?"
        f"target=clash&"
        f"url={target_url}&"
        f"config={GITHUB_CONFIG_URL}&"
        f"emoji=true&list=false&udp=true"
    )
    logger.info(f"完整 URL: {final_url}")
    
    forward_headers = dict(request.headers)
    forward_headers.pop('Host', None)

    try:
        logger.info("正在请求订阅转换后端...")
        resp = requests.get(final_url, headers=forward_headers, timeout=20)
        resp.raise_for_status()
        
        logger.info("开始执行 YAML 剪枝 (main_prune)...")
        config_data = yaml.safe_load(resp.text)
        from pruner import main_prune
        clean_config = main_prune(config_data)
        post_nodes = len(clean_config.get('proxies', []))
        post_groups = len(clean_config.get('proxy-groups', []))
        logger.info(f"剪枝完成，节点数: {post_nodes}, 策略组数: {post_groups}")
        
        return Response(
            yaml.dump(clean_config, allow_unicode=True, sort_keys=False), 
            mimetype='text/yaml'
        )
        
    except Exception as e:
        logger.error(f"发生异常: {str(e)}", exc_info=True)
        return f"转换失败: {str(e)}\n最终构造地址: {final_url}", 500


@app.route('/')
def index():
    return "Private Subconverter Service is Running."

if __name__ == '__main__':
    app.run(host='0.0.0.0', port=8080)
