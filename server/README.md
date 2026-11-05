# 独立订阅转换服务 (Rust / Axum)

## 📌 项目概述
纯自研、自包含的 VPN 订阅转换服务。彻底剥离了第三方转换后端，能够无损保留新兴协议节点（如 `anytls`、`hysteria2`、`vless` 等），自动按国家重新编排节点组（`proxy-groups`）并注入自定义分流规则（`rules`）。

支持本地独立部署、Render (Docker) 云端部署以及 Vercel Serverless 部署。

---

## 📡 接口使用

### 1. 订阅转换与短链绑定
- **首次绑定 / 转换**：
  `GET http://127.0.0.1:8080/<key>?<机场订阅链接>`
- **日常刷新（客户端填此极简短链）**：
  `GET http://127.0.0.1:8080/<key>`
- **注销解绑**：
  `GET http://127.0.0.1:8080/<key>?clear`

### 2. 私人规则热刷新（修改 China.list 后使用）
- **触发实时拉取**：
  `GET http://127.0.0.1:8080/refresh`（或云端对应地址）
  服务器会立即重新从 GitHub 拉取最新的 `China.list` 与 `Global.list` 覆盖内存缓存。
  改动后 1 秒生效，无需等待，更无需重新构建服务！
- **带参刷新**：
  访问 `http://127.0.0.1:8080/<key>?refresh` 也可一键热刷新并当场返回最新配置。

---

## 🚀 启动与部署

- **本地运行**：双击运行 `启动server.bat`，或在 `server/` 目录下执行 `cargo run -p core`；
- **Render 部署**：直接基于本目录的 `Dockerfile` 构建（执行 `cargo build --release -p core`）；
- **Vercel 部署**：进入 `vercel/` 目录通过 `vercel deploy` 发布，通过 `core::router()` 运行。
