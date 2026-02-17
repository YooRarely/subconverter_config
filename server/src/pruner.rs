use serde_yaml_ng::Value;
use tracing::info;

/// 模拟 Python 的 is_valid 逻辑
fn is_valid(group: &Value) -> bool {
    let valid_groups = ["🚀 节点选择"];
    let special_proxies = ["DIRECT", "REJECT", "PASS"];

    let name = group.get("name").and_then(|v| v.as_str()).unwrap_or("");

    // 1. 如果是核心选择组，保留
    if valid_groups.contains(&name) {
        return true;
    }

    // 2. 检查 proxies 列表
    if let Some(proxies) = group.get("proxies").and_then(|v| v.as_sequence()) {
        if proxies.is_empty() {
            return false;
        }
        // 3. 如果成员里至少有一个不是特殊指令，则合法
        return proxies.iter().any(|p| {
            let p_str = p.as_str().unwrap_or("");
            !special_proxies.contains(&p_str)
        });
    }

    false
}

/// 模拟 Python 的 prune_groups 逻辑
fn prune_groups(config: &mut Value) -> bool {
    let mut changed = false;

    // 获取并过滤出不合法的组名
    let invalid_names: Vec<String> = if let Some(groups) = config.get("proxy-groups").and_then(|v| v.as_sequence()) {
        groups
            .iter()
            .filter(|g| !is_valid(g))
            .filter_map(|g| g.get("name").and_then(|v| v.as_str()).map(|s| s.to_string()))
            .collect()
    } else {
        return false;
    };

    if invalid_names.is_empty() {
        return false;
    }

    // 1. 物理过滤：从 proxy-groups 序列中移除不合法的组
    if let Some(groups) = config.get_mut("proxy-groups").and_then(|v| v.as_sequence_mut()) {
        let original_len = groups.len();
        groups.retain(|g| {
            let name = g.get("name").and_then(|v| v.as_str()).unwrap_or("");
            !invalid_names.contains(&name.to_string())
        });
        if groups.len() != original_len {
            changed = true;
        }
    }

    // 2. 引用剪枝：移除其他组中对已删除组的引用
    if let Some(groups) = config.get_mut("proxy-groups").and_then(|v| v.as_sequence_mut()) {
        for g in groups {
            if let Some(proxies) = g.get_mut("proxies").and_then(|v| v.as_sequence_mut()) {
                let original_proxies_len = proxies.len();
                proxies.retain(|p| {
                    let p_str = p.as_str().unwrap_or("");
                    !invalid_names.contains(&p_str.to_string())
                });
                if proxies.len() != original_proxies_len {
                    changed = true;
                }
            }
        }
    }

    changed
}

/// 导出主剪枝函数
pub fn main_prune(mut config: Value) -> Value {
    if config.get("proxy-groups").is_none() || config.get("proxies").is_none() {
        return config;
    }

    info!("开始执行 YAML 剪枝迭代...");
    let mut iter_count = 0;
    
    // 循环迭代，直到没有更多的组可以被修剪
    while prune_groups(&mut config) {
        iter_count += 1;
        info!("剪枝第 {} 轮完成", iter_count);
    }

    if let Some(groups) = config.get("proxy-groups").and_then(|v| v.as_sequence()) {
        info!("剪枝最终完成，剩余策略组数: {}", groups.len());
    }
    
    config
}
