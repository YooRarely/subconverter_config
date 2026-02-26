# --- 编译阶段 ---
FROM rust:1.80-slim AS builder
WORKDIR /app
# 先拷贝总管理论和所有子项目
COPY . .
# 强制指定编译 core 这个 member
RUN cargo build --release -p core
	
# --- 运行阶段 ---
FROM debian:bookworm-slim
# 必须安装 ca-certificates，否则你的 Rust 程序无法请求 https 链接
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
# 从 builder 拷贝二进制文件
COPY --from=builder /app/target/release/core /app/sub-proxy
# 运行
CMD ["./sub-proxy"]
