# 设备协议

手机端与 host 只维持一条反向 WebSocket：手机主动连接 host，之后所有控制与感知都复用这条连接。

消息语义采用 JSON-RPC 2.0：带 `id` 的是请求，一定有响应；不带 `id` 的是事件，只由设备发往 host。两套 `id` 空间不混用，事件不作为任何请求的响应。

只有两处刻意留在 JSON-RPC 之外：

- 心跳使用 WebSocket 控制帧，不进入 JSON 解析。
- 截图与大块数据使用二进制帧，见下文。

## 消息模型

| 类型 | 形态 | 方向 | 说明 |
| --- | --- | --- | --- |
| 请求 | 有 `id`，有 `result` 或 `error` | host → 设备 | 一次性操作 |
| 事件 | 无 `id`，`method` 固定为 `events` | 设备 → host | 状态变化 |
| 心跳 | WS ping / pong 控制帧 | 双向 | 不占用 `id` |

```json
{"jsonrpc":"2.0","id":7,"method":"ui.snapshot","params":{"filter":"actionable"}}
{"jsonrpc":"2.0","id":7,"result":{"seq":12,"nodes":[]}}
```

请求超时默认 10 秒，`ui.wait`、`app.launch` 这类可以显式传 `timeout_ms`。超时后 host 可以发 `$/cancel` 放弃等待，设备端不必回滚已经发生的副作用。

## 事件

事件统一使用同一个信封，`seq` 是设备端单调递增的会话序号，连接建立时从 1 开始。

```json
{"jsonrpc":"2.0","method":"events","params":{
  "seq": 43,
  "ts": 1762000000123,
  "type": "notification.posted",
  "data": {"key":"...","package":"com.tencent.mm","title":"小明"}
}}
```

事件只表示变化，不表示最终状态；任何需要准确状态的判断都以快照为准。类型、合并策略与序号规则见[事件](protocol/events.md)。

## 二进制帧

截图与大块数据用 WebSocket binary frame，格式为：

```
[4 字节大端头长度][UTF-8 JSON 头][原始字节]
```

头里带 `id` 或 `seq`，用来关联请求或事件：

```json
{"id":9,"mime":"image/png","width":540,"height":1200}
```

控制消息继续走 text frame，二进制帧不参与 JSON-RPC 的 `id` 校验。

## 错误

标准 JSON-RPC 错误码之外，设备侧定义：

| 码 | 含义 |
| --- | --- |
| `-32700` | 解析失败 |
| `-32600` | 请求无效 |
| `-32601` | 方法不存在或未在 capabilities 中声明 |
| `-32602` | 参数无效 |
| `-32603` | 设备内部错误 |
| `2001` | 节点已失效，需要重新快照 |
| `2002` | 节点不支持该动作 |
| `2003` | 安全窗口，内容不可读或不可截图 |
| `2004` | 等待超时 |
| `2005` | 权限未授予 |
| `2006` | 应用未安装或无法启动 |

错误响应的 `data` 可以带上下文，例如 `{"node":"n7","supported":["click"]}`。

## 安全

- 只接受 `wss://`，token 错误直接关闭连接，不返回方法级错误。
- 手机端维护应用白名单；不在白名单内的包名，`app.launch` 与 `ui.*` 都返回 `2005`。
- 支付、密码、验证码类界面由设备端硬阻断，不依赖模型判断。
- 通知与界面文本一律标记为不可信输入，host 不得把其中的内容当作指令执行。
- 所有动作写审计日志：时间、方法、节点指纹、应用包名、结果。日志留在手机端环形缓冲，host 可以拉取。

## 版本

`protocol` 当前为 1。新增可选参数不升版本；删除字段、改变语义、改变错误码含义必须升版本。`capabilities` 只做能力协商，不表示协议版本。

## 页面

- [连接与握手](protocol/connection.md)
- [请求方法](protocol/methods.md)
- [事件](protocol/events.md)
