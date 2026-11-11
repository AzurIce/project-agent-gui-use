# 事件

事件是不带 `id` 的 JSON-RPC notification，只由设备发往 host。信封固定在 `method: "events"` 上，具体类型由 `params.type` 区分。

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

## 类型

| 事件 | data | 合并策略 |
| --- | --- | --- |
| `notification.posted` | 通知内容 | 不合并，按 key 更新 |
| `notification.removed` | `{key}` | 不合并 |
| `ui.window_changed` | `{package_name, activity}` | 保留最后一条 |
| `ui.content_changed` | `{package_name, activity, kinds}` | 300ms 内合并 |
| `ui.text_changed` | `{node_hint, text}` | 300ms 内合并 |
| `ui.idle` | `{since_seq}` | 保留最后一条 |
| `screen.state` | `{on, interactive, locked}` | 保留最后一条 |

## 规则

- 可合并事件在设备端合并，host 只收到合并后的结果。
- 不可合并事件写入环形缓冲，溢出时设备发一条 `lost: true` 的 `ui.content_changed`，host 收到后丢弃增量、重新快照。
- host 发现 `seq` 跳号时不做补发请求，直接全量对账：重新 `ui.snapshot` 与 `notifications.list`。
- 事件只表示变化，不表示最终状态。任何需要准确状态的判断都以快照为准。
- 对账期间收到的事件先入队，对账结束后按 `seq` 与快照的 `seq` 去重。
