from __future__ import annotations

from typing import Any

from defusedxml import ElementTree as ET


class FIXTreeParser:
    def __init__(self, xml_path: str):
        self.tree = ET.parse(xml_path)
        self.root = self.tree.getroot()
        self.tag_to_name = {f.get("number"): f.get("name") for f in self.root.findall(".//fields/field")}
        self.name_to_tag = {v: k for k, v in self.tag_to_name.items()}
        self.components = {c.get("name"): c for c in self.root.findall(".//components/component")}

    def _get_fields_and_groups(self, element: Any) -> tuple[set[str], dict[str, Any], str | None]:
        inner_tags: set[str] = set()
        sub_groups: dict[str, Any] = {}
        first_tag: str | None = None

        for child in element:
            first_tag = self._process_child(child, inner_tags, sub_groups, first_tag)
        return inner_tags, sub_groups, first_tag

    def _process_child(
        self,
        child: Any,
        inner_tags: set[str],
        sub_groups: dict[str, Any],
        first_tag: str | None,
    ) -> str | None:
        if child.tag == "field":
            return self._process_field(child, inner_tags, first_tag)
        if child.tag == "component":
            return self._process_component(child, inner_tags, sub_groups, first_tag)
        if child.tag == "group":
            return self._process_group(child, inner_tags, sub_groups, first_tag)
        return first_tag

    def _process_field(self, child: Any, inner_tags: set[str], first_tag: str | None) -> str | None:
        tag = self.name_to_tag.get(child.get("name"))
        if not tag:
            return first_tag
        inner_tags.add(tag)
        return tag if first_tag is None else first_tag

    def _process_component(
        self,
        child: Any,
        inner_tags: set[str],
        sub_groups: dict[str, Any],
        first_tag: str | None,
    ) -> str | None:
        comp_name = child.get("name")
        if comp_name not in self.components:
            return first_tag
        c_tags, c_groups, c_first = self._get_fields_and_groups(self.components[comp_name])
        inner_tags.update(c_tags)
        sub_groups.update(c_groups)
        return c_first if first_tag is None else first_tag

    def _process_group(
        self,
        child: Any,
        inner_tags: set[str],
        sub_groups: dict[str, Any],
        first_tag: str | None,
    ) -> str | None:
        g_name = child.get("name")
        g_tag = self.name_to_tag.get(g_name)
        if not g_tag:
            return first_tag
        inner_tags.add(g_tag)
        g_inner_tags, g_sub_groups, g_first = self._get_fields_and_groups(child)
        sub_groups[g_tag] = {
            "name": g_name,
            "first_tag": g_first,
            "inner_tags": g_inner_tags,
            "sub_groups": g_sub_groups,
        }
        return g_tag if first_tag is None else first_tag

    def parse(self, message: Any, separator: str = "|") -> dict[str, Any]:
        if hasattr(message, "toString"):
            raw_fix = message.toString().replace("\x01", separator)
        else:
            raw_fix = str(message)

        pairs = [p.split("=") for p in raw_fix.strip(separator).split(separator) if "=" in p]
        if not pairs:
            return {}

        msg_type_val = next((v for t, v in pairs if t == "35"), None)
        msg_def = self.root.find(f".//messages/message[@msgtype='{msg_type_val}']")

        _, group_specs, _ = self._get_fields_and_groups(msg_def) if msg_def is not None else (set(), {}, None)

        it = iter(pairs)
        return self._parse_recursive(it, group_specs)

    def _parse_recursive(
        self,
        it: Any,
        group_specs: dict[str, Any],
    ) -> dict[str, Any]:
        node: dict[str, Any] = {}
        current_pair: tuple[str, str] | None = None

        while True:
            try:
                tag, val = current_pair if current_pair else next(it)
                current_pair = None

                field_name = self.tag_to_name.get(tag, f"Tag_{tag}")

                if tag in group_specs:
                    count = int(val)
                    spec = group_specs[tag]
                    items = []
                    pending_pair = None

                    for _ in range(count):
                        first_p = pending_pair if pending_pair else next(it)

                        item_data, pending_pair = self._parse_recursive_item(first_p, it, spec)
                        items.append(item_data)

                    node[field_name] = items
                    current_pair = pending_pair
                else:
                    node[field_name] = val
            except StopIteration:
                break
        return node

    def _parse_recursive_item(
        self,
        first_pair: tuple[str, str],
        it: Any,
        spec: dict[str, Any],
    ) -> tuple[dict[str, Any], tuple[str, str] | None]:
        item_data: dict[str, Any] = {}
        tag, val = first_pair
        item_data[self.tag_to_name.get(tag, tag)] = val

        current_pair = None
        while True:
            try:
                tag, val = current_pair if current_pair else next(it)
                current_pair = None

                if tag == spec["first_tag"]:
                    return item_data, (tag, val)

                if tag in spec["sub_groups"]:
                    count = int(val)
                    sub_spec = spec["sub_groups"][tag]
                    sub_items = []
                    p_pair = None
                    for _ in range(count):
                        f_p = p_pair if p_pair else next(it)
                        sub_item, p_pair = self._parse_recursive_item(f_p, it, sub_spec)
                        sub_items.append(sub_item)
                    item_data[self.tag_to_name.get(tag, tag)] = sub_items
                    current_pair = p_pair
                elif tag in spec["inner_tags"]:
                    item_data[self.tag_to_name.get(tag, tag)] = val
                else:
                    return item_data, (tag, val)
            except StopIteration:
                return item_data, None
