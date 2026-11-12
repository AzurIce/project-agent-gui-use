#import "../_components/protocol/lib.notc" as protocol;

# 事件

事件是不带 `id` 的 JSON-RPC notification，只由设备发往 host。信封固定在 `method: "events"` 上，具体类型由 `params.type` 区分。

`ui.text_changed` 的 `node_hint` 优先使用资源 ID，系统未提供时退回节点类名；它只是定位提示，执行动作仍需使用最新快照中的节点 ID。

```json
{"jsonrpc":"2.0","method":"events","params":{
  "seq": 43,
  "ts": "2026-11-12T01:23:45.678Z",
  "type": "notification.posted",
  "data": {"key":"...","package_name":"com.tencent.mm","title":"小明"}
}}
```

| 字段 | 说明 |
| --- | --- |
| `seq` | 设备端单调递增的会话序号，连接建立时从 1 开始 |
| `ts` | RFC 3339 时间戳 |
| `type` | 事件类型 |
| `data` | 类型相关载荷 |

<!-- protocol:events -->

## 规则

- 可合并事件在设备端合并，host 只收到合并后的结果。
- 通知事件不做去抖，UI 内容事件按 300 毫秒去抖。发送队列超过限制时设备关闭连接并重连，以新的会话边界触发对账。
- host 发现 `seq` 跳号时不做补发请求，直接全量对账：重新 `ui.snapshot` 与 `notification.list`。
- 事件只表示变化，不表示最终状态。任何需要准确状态的判断都以快照为准。
- host 保存最近 512 条事件；调用方按 `session`、`seq` 与快照的 `seq` 去重，不能把历史缓冲当成永久消息队列。
