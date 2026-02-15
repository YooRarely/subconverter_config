import os

# ================= 配置区 =================
CONFIG = {
    "name_format": "{emoji}{zws} {name}", 
    "MERGE_NAME": "🚀 节点选择",
    "UNKNOWN_NAME": "❓ 未知", # 兜底组的名称
    
    "use_zws": True,
    "interval": 300,
    "tolerance": 50,
    "lazy": True,
    "input_file": "emoji.txt",
    "output_file": "country.toml"
}
# ==========================================

ZWS = '\u200b' if CONFIG["use_zws"] else ''

def generate_toml():
    input_path = CONFIG["input_file"]
    output_path = CONFIG["output_file"]

    if not os.path.exists(input_path):
        print(f"找不到输入文件: {input_path}")
        return

    with open(input_path, 'r', encoding='utf-8') as f:
        lines = [line.strip() for line in f if line.strip() and '/' in line]

    toml_blocks = []
    all_country_proxies = []
    all_emojis = []

    for line in lines:
        display_name, emoji = line.split('/')
        all_emojis.append(emoji) # 收集所有已知的 Emoji
        
        full_group_name = CONFIG["name_format"].format(
            emoji=emoji, zws=ZWS, name=display_name
        ).strip()
        
        block = (
            f'[[custom_groups]]\n'
            f'name = "{full_group_name}"\n'
            f'type = "url-test"\n'
            f'rule = [".*{emoji}"]\n'
            f'url = "http://www.gstatic.com/generate_204"\n'
            f'interval = {CONFIG["interval"]}\n'
            f'tolerance = {CONFIG["tolerance"]}\n'
            f'lazy = {str(CONFIG["lazy"]).lower()}\n'
        )
        toml_blocks.append(block)
        all_country_proxies.append(f"[]{full_group_name}")

    # --- 构造“未知”组的排除正则 ---
    # 原理：匹配不包含列表中任何一个 Emoji 的所有节点
    # 正则示例: ^((?!(🇭🇰|🇹🇼|🇸🇬)).)*$
    emoji_pattern = "|".join(all_emojis)
    exclude_rule = f"^((?!({emoji_pattern})).)*$"

    unknown_group_name = f"{CONFIG['UNKNOWN_NAME']}"
    unknown_block = (
        f'[[custom_groups]]\n'
        f'name = "{unknown_group_name}"\n'
        f'type = "select"\n' # 未知组建议用 select，方便手动看有哪些杂鱼节点
        f'rule = ["{exclude_rule}"]\n'
    )

    # --- 构造主选择组 (MERGE_NAME) ---
    # 顺序：DIRECT -> 所有国家组 -> 未知组
    # final_proxies = ["[]DIRECT"] + [f"[]{unknown_group_name}"] + all_country_proxies

    final_proxies = all_country_proxies
    final_proxies += [f"[]{unknown_group_name}"]

    rule_str = "[" + ", ".join([f'"{p}"' for p in final_proxies]) + "]"

    with open(output_path, 'w', encoding='utf-8') as f:
        f.write(f"# --- {CONFIG['MERGE_NAME']} 主组 ---\n")
        f.write("[[custom_groups]]\n")
        f.write(f'name = "{CONFIG["MERGE_NAME"]}"\n')
        f.write('type = "select"\n')
        f.write(f'rule = {rule_str}\n\n')
        
        f.write("# --- 兜底组 (不包含已知 Emoji 的节点) ---\n")
        f.write(unknown_block + "\n")
        
        f.write("# --- 自动生成的国家测速组 ---\n")
        f.write("\n".join(toml_blocks))

    print(f"成功！已添加兜底组: {CONFIG['UNKNOWN_NAME']}")
    print(f"该组正则已自动排除 {len(all_emojis)} 个已知 Emoji")

if __name__ == "__main__":
    generate_toml()