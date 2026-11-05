@echo off
chcp 65001 >nul
title 本地订阅转换服务
echo ========================================================
echo   正在启动本地独立订阅转换服务...
echo   (如果 8080 被占用将自动顺延到 25580，绝不闪退)
echo ========================================================
cd /d "%~dp0"
if exist "server.exe" (
    server.exe
) else (
    cargo run -p subconverter-core
)
pause
