def is_valid(group):
    valid_groups = ['🚀 节点选择']
    special_proxies = ['DIRECT', 'REJECT', 'PASS']
    
    # 如果是核心选择组，保留
    if group.get('name') in valid_groups:
        return True
    
    proxies = group.get('proxies', [])
    # 如果没有成员，不合法
    if not proxies:
        return False
    
    # 如果成员里至少有一个不是特殊指令（即包含真实节点或其他组），则合法
    return any(p not in special_proxies for p in proxies)

def prune_groups(config):
    groups = config.get('proxy-groups', [])
    if not groups:
        return False
    
    changed = False
    # 1. 识别不合法组的名单
    invalid_names = [g['name'] for g in groups if not is_valid(g)]
    
    if invalid_names:
        # 2. 物理过滤：移除不合法组
        original_count = len(groups)
        config['proxy-groups'] = [g for g in groups if g['name'] not in invalid_names]
        if len(config['proxy-groups']) != original_count:
            changed = True
            
        # 3. 引用剪枝：在剩余组的 proxies 列表中移除这些被删掉的组名
        for g in config['proxy-groups']:
            proxies = g.get('proxies', [])
            new_proxies = [p for p in proxies if p not in invalid_names]
            if len(new_proxies) != len(proxies):
                g['proxies'] = new_proxies
                changed = True
                
    return changed

def main_prune(config):
    if 'proxy-groups' not in config or 'proxies' not in config:
        return config
    
    # 循环迭代，直到没有更多的组可以被修剪
    while prune_groups(config):
        pass
        
    return config