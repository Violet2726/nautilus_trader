import xml.etree.ElementTree as ET


class FIXTreeParser:
    def __init__(self, xml_path):
        self.tree = ET.parse(xml_path)
        self.root = self.tree.getroot()
        self.tag_to_name = {f.get('number'): f.get('name') for f in self.root.findall(".//fields/field")}
        self.name_to_tag = {v: k for k, v in self.tag_to_name.items()}
        self.components = {c.get('name'): c for c in self.root.findall(".//components/component")}

    def _get_fields_and_groups(self, element):
        """递归展开 component 并提取字段和子组"""
        inner_tags = set()
        sub_groups = {}
        first_tag = None

        for child in element:
            if child.tag == 'field':
                tag = self.name_to_tag.get(child.get('name'))
                if tag:
                    inner_tags.add(tag)
                    if first_tag is None: first_tag = tag

            elif child.tag == 'component':
                comp_name = child.get('name')
                if comp_name in self.components:
                    c_tags, c_groups, c_first = self._get_fields_and_groups(self.components[comp_name])
                    inner_tags.update(c_tags)
                    sub_groups.update(c_groups)
                    if first_tag is None: first_tag = c_first

            elif child.tag == 'group':
                g_name = child.get('name')
                g_tag = self.name_to_tag.get(g_name)
                if g_tag:
                    inner_tags.add(g_tag)
                    if first_tag is None: first_tag = g_tag
                    # 递归解析子组定义
                    g_inner_tags, g_sub_groups, g_first = self._get_fields_and_groups(child)
                    sub_groups[g_tag] = {
                        "name": g_name,
                        "first_tag": g_first,
                        "inner_tags": g_inner_tags,
                        "sub_groups": g_sub_groups
                    }
        return inner_tags, sub_groups, first_tag

    def parse(self, message, separator='|'):
        # --- 核心修复：处理 QuickFIX Message 对象 ---
        if hasattr(message, 'toString'):
            # 将 SOH (\x01) 替换为统一分隔符
            raw_fix = message.toString().replace('\x01', separator)
        else:
            raw_fix = str(message)
        # 预处理字符串
        pairs = [p.split('=') for p in raw_fix.strip(separator).split(separator) if '=' in p]
        if not pairs: return {}

        # 获取消息定义
        msg_type_val = next((v for t, v in pairs if t == '35'), None)
        msg_def = self.root.find(f".//messages/message[@msgtype='{msg_type_val}']")

        # 预加载该消息的结构
        _, group_specs, _ = self._get_fields_and_groups(msg_def) if msg_def is not None else (set(), {}, None)

        it = iter(pairs)
        return self._parse_recursive(it, group_specs)

    def _parse_recursive(self, it, group_specs, stop_condition=None):
        node = {}
        current_pair = None

        while True:
            try:
                tag, val = current_pair if current_pair else next(it)
                current_pair = None

                # 检查是否触发出栈条件（属于上一层级的数据）
                if stop_condition and stop_condition(tag):
                    return node, (tag, val)

                field_name = self.tag_to_name.get(tag, f"Tag_{tag}")

                if tag in group_specs:
                    count = int(val)
                    spec = group_specs[tag]
                    items = []
                    pending_pair = None

                    for _ in range(count):
                        # 解析单个组条目
                        first_p = pending_pair if pending_pair else next(it)

                        # 定义条目结束条件：遇到该组的首字段且不是第一次，或者遇到不属于该组的字段
                        def is_entry_end(t):
                            return t == spec['first_tag'] or (
                                    t not in spec['inner_tags'] and t not in spec['sub_groups'])

                        item_data, pending_pair = self._parse_recursive_item(first_p, it, spec)
                        items.append(item_data)

                    node[field_name] = items
                    current_pair = pending_pair  # 将解析组剩下的那个 pair 交给下一轮
                else:
                    node[field_name] = val
            except StopIteration:
                break
        return node

    def _parse_recursive_item(self, first_pair, it, spec):
        """解析重复组中的一个 Item"""
        item_data = {}
        tag, val = first_pair
        item_data[self.tag_to_name.get(tag, tag)] = val

        current_pair = None
        while True:
            try:
                tag, val = current_pair if current_pair else next(it)
                current_pair = None

                # 如果遇到本组的首字段，说明新条目开始了，当前条目结束
                if tag == spec['first_tag']:
                    return item_data, (tag, val)

                # 如果遇到子组
                if tag in spec['sub_groups']:
                    count = int(val)
                    sub_spec = spec['sub_groups'][tag]
                    sub_items = []
                    p_pair = None
                    for _ in range(count):
                        f_p = p_pair if p_pair else next(it)
                        sub_item, p_pair = self._parse_recursive_item(f_p, it, sub_spec)
                        sub_items.append(sub_item)
                    item_data[self.tag_to_name.get(tag, tag)] = sub_items
                    current_pair = p_pair
                # 如果是组内普通字段
                elif tag in spec['inner_tags']:
                    item_data[self.tag_to_name.get(tag, tag)] = val
                else:
                    # 不属于本组的字段，交给父级处理
                    return item_data, (tag, val)
            except StopIteration:
                return item_data, None
