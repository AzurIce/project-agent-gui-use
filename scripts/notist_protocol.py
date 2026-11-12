"""Notist SSG extension: render Rust doc comments and expose protocol site data.

Rust emits JSON only. This extension adds Markdown documentation to the source
collection; field/type tables are built by the reusable .notc component.
"""

import json
import sys
from pathlib import Path


def method_docs(registry):
    parts = ["#protocol.index()"]
    for index, method in enumerate(registry["methods"]):
        parts += [
            f"## `{method['name']}`",
            method["docs"],
            "### 参数",
            f'#protocol.method_fields({index}, "params")',
            "### 返回",
            f'#protocol.method_fields({index}, "result")',
        ]
    parts += ["## 类型", "#protocol.types()"]
    return "\n\n".join(parts)


def event_docs(registry):
    parts = ["#protocol.event_index()"]
    for index, event in enumerate(registry["events"]):
        parts += [
            f"## `{event['name']}`",
            event["docs"],
            f"合并策略：{event['merge']}",
            "### 数据",
            f"#protocol.event_fields({index})",
        ]
    return "\n\n".join(parts)


def respond(request, registry):
    response = {"version": 2, "event": request["event"]}
    if request["version"] != 2:
        raise ValueError("unsupported Notist site extension protocol")
    if request["event"] == "before_compile":
        documents = dict(request["documents"])
        for path, marker, content in [
            ("protocol/methods.md", "<!-- protocol:methods -->", method_docs(registry)),
            ("protocol/events.md", "<!-- protocol:events -->", event_docs(registry)),
        ]:
            source = documents[path]
            if source.count(marker) != 1:
                raise ValueError(f"{path} must contain exactly one {marker}")
            documents[path] = source.replace(marker, content)
        response["documents"] = documents
    elif request["event"] == "before_template":
        response["data"] = {**request["data"], "protocol": registry}
    else:
        raise ValueError(f"unsupported event: {request['event']}")
    return response


def main():
    registry = None
    for line in sys.stdin:
        request = json.loads(line)
        try:
            if registry is None:
                root = Path(request["root"])
                registry = json.loads((root / "protocol/registry.json").read_text())
            response = respond(request, registry)
        except Exception as error:
            response = {"version": 2, "event": request["event"], "errors": [str(error)]}
        print(json.dumps(response, ensure_ascii=False), flush=True)


if __name__ == "__main__":
    main()
