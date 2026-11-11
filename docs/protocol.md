#import "./_components/pele/lib.notc" as pele;

# 设备协议

手机端与 host 只维持一条反向 WebSocket：手机主动连接 host，之后所有控制与感知都复用这条连接。

消息语义采用 JSON-RPC 2.0：带 `id` 的是请求，一定有响应；不带 `id` 的是事件，只由设备发往 host。两套 `id` 空间不混用，事件不作为任何请求的响应。

只有两处刻意留在 JSON-RPC 之外：心跳使用 WebSocket 控制帧，截图与大块数据使用二进制帧。

## 连接

- 传输为 `wss://`，手机主动外连，host 是服务端。地址与 token 保存在手机端配置里。
- 首次连接后发送 `hello`，协商协议版本与能力；`hello` 成功之前不接受其他请求。
- token 放在 `hello` 的 params 里，不放 URL，避免被代理日志记录。
- 重连采用指数退避，上限 30 秒；网络切换、亮屏、充电状态变化时立即重试一次。

#pele.diagram(r#"""
flowchart LR
  boot[手机端启动] --> connect[连接 host]
  connect --> hello[hello 协商]
  hello --> live[在线]
  live --> request[请求 / 响应]
  live --> events[事件推送]
  request --> drop[断线]
  events --> drop
  drop --> connect
  greet[重连成功] --> sync[快照对账]
  sync --> live
  connect -.-> greet
"""#)

## 握手

```json
{"jsonrpc":"2.0","id":0,"method":"hello","params":{
  "protocol": 1,
  "token": "...",
  "device": {"id":"...","model":"...","sdk":36},
  "capabilities": ["apps.list","app.launch","ui.snapshot"]
}}
```

成功返回 host 的协议版本和它接受的事件类型；版本不一致时直接返回错误，不做降级猜测。

```json
{"jsonrpc":"2.0","id":0,"result":{
  "protocol": 1,
  "events": ["notification.posted","ui.window_changed","ui.idle"]
}}
```

## 心跳

- 使用 WebSocket 控制帧（RFC 6455 ping/pong），不占用 JSON-RPC，也不与业务消息抢队列。
- 设备每 15 秒发 ping，host 立即回 pong；任意一侧 30 秒内既没有 pong 也没有其他帧，就判定连接不可用并重连。host 侧同样按 pong 超时回收会话。
- 不能只依赖 close 事件：移动网络切换、NAT 静默超时、进程被杀、Doze 挂起都会留下半开连接，TCP 可能长时间保持 ESTABLISHED，只有应用层 ping 才能发现对端已经不可用。TCP keepalive 的默认超时以小时计，不作为检测手段。

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

## 重连与对账

1. 连接断开后按退避重连，重新走 `hello`。
2. 新会话的 `seq` 从 1 开始，host 丢弃上一会话的增量缓存。
3. 对账顺序固定：`ui.snapshot` → `notifications.list` → 继续消费事件。
4. 对账期间收到的事件先入队，不丢弃；对账完成后按 `seq` 与快照的 `seq` 去重。

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

- [请求方法](protocol/methods.md)
- [事件](protocol/events.md)
