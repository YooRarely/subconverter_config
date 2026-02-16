@echo off
:: 设置环境变量
set SUB_BACKEND=https://yoorarely-subconverter.zeabur.app/sub
set GITHUB_CONFIG_URL=https://raw.githubusercontent.com/YooRarely/subconverter_config/refs/heads/main/config/remote_config.toml

:: 打印一下，方便确认
echo ========================================
echo Starting Flask Server...
echo Backend: %SUB_BACKEND%
echo ========================================

:: 运行 Python 脚本
python app.py

pause