@echo off
git add .
git commit -m "Update config and rules"
git push
echo "部署完成！请刷新 Clash 订阅。"
pause

