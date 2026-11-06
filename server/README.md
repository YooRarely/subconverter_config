# 独立订阅转换服务 (Rust / Axum)

## 📌 项目概述
纯自研、自包含的 VPN 订阅转换服务。彻底剥离了第三方转换后端，能够无损保留新兴协议节点（如 `anytls`、`hysteria2`、`vless` 等），自动按国家重新编排节点组（`proxy-groups`）并注入自定义分流规则（`rules`）。

支持本地独立部署、Render (Docker) 云端部署以及 Vercel Serverless 部署。

---

## 📡 接口使用与控制台

### 1. Web 可视化控制台（推荐）
- 直接在浏览器打开主页：`http://127.0.0.1:25580` 或 `http://127.0.0.1:25580/admin`（云端直接输入域名回车即可，无需后缀）；
- 开启管理密码保护（通过环境变量 `ADMIN_PASSWORD` 保护，未登录前阻断任何数据展示）；
- 支持一键录入短链 Key、一键缓存真节点快照、在线实时探测、清空缓存、复制短链等。

### 2. 客户端订阅短链
- **日常刷新（客户端填此极简短链）**：
  `GET http://127.0.0.1:25580/<key>`
- **带参强制刷新规则**：
  `GET http://127.0.0.1:25580/<key>?refresh`
- **注销解绑**：
  `GET http://127.0.0.1:25580/<key>?clear`

### 3. 私人规则热刷新（修改 China.list 后使用）
- **触发实时拉取**：
  `GET http://127.0.0.1:25580/refresh`（或云端对应地址）
  服务器会立即重新从 GitHub 拉取最新的 `China.list` 与 `Global.list` 覆盖内存缓存。
  改动后 1 秒生效，无需等待，更无需重新构建服务！

---

## 🚀 启动与部署

- **本地运行**：双击运行 `启动server.bat`，服务监听在 `http://127.0.0.1:25580`（自动避开 Steam 占用的 8080 端口）；
- **Render 部署**：
  - 在 Render 后台创建 Web Service，根目录指向当前仓库；
  - 环境推荐选择 Docker（自动根据 `server/Dockerfile` 构建）；
  - 在 Render **Environment** 中配置以下环境变量：
    - `ADMIN_PASSWORD`: 管理员控制台登录密码 (必填，防止陌生人偷看订阅)；
    - `REDIS_CONNECTION_STRING`: Render 提供的免费 Redis 连接串 (如 `rediss://...`，用于持久化保存订阅和真节点快照)；
- **Vercel 部署**：进入 `vercel/` 目录通过 `vercel deploy` 发布，通过 `core::router()` 运行。
