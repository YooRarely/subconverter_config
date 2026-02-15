import os

# --- 配置区 ---
# 你的 GitHub 用户名和仓库名
GITHUB_USER = "YooRarely"
REPO_NAME = "subconverter_config"
BRANCH = "main"

# GitHub Raw 的前缀
RAW_PREFIX = f"https://raw.githubusercontent.com/{GITHUB_USER}/{REPO_NAME}/refs/heads/{BRANCH}/"

# 原始模板文件和输出文件
INPUT_FILE = "config.toml" # 你刚才发给我的那段代码存为此文件
OUTPUT_FILE = "remote_config.toml"        # 最终上传到 GitHub 的文件

def build_config():
    if not os.path.exists(INPUT_FILE):
        print(f"错误：找不到模板文件 {INPUT_FILE}")
        return

    with open(INPUT_FILE, 'r', encoding='utf-8') as f:
        content = f.read()

    # 执行替换逻辑：将 target/ 替换为 GitHub 的全路径
    # 注意：为了防止误伤，我们匹配 "target/"
    updated_content = content.replace('target/', RAW_PREFIX)

    with open(OUTPUT_FILE, 'w', encoding='utf-8') as f:
        f.write(updated_content)

    print(f"成功！已生成战斗版配置文件: {OUTPUT_FILE}")
    print(f"现在你可以执行 git push 把它送上云端了。")

if __name__ == "__main__":
    build_config()