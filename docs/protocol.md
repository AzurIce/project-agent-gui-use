#import "./_components/pele/lib.notc" as pele;

# 设备协议

手机端与 host 之间只维持一条反向 WebSocket：手机主动连接 host，之后所有控制与感知都复用这条连接。

消息复用 JSON-RPC 2.0 的形式：带 `id` 的是请求，一定有响应；不带 `id` 的是事件，只有设备发往 host 一个方向。两者不共用 `id` 空间，事件也不允许作为任何请求的响应。

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

## 连接

- 传输为 `wss://`，手机主动外连，host 是服务端。地址与 token 保存在手机端配置里。
- 首次连接后发送 `hello`，协商协议版本与能力；`hello` 成功之前不接受其他请求。
- token 放在 `hello` 的 params 里，不放 URL，避免被代理日志记录。
- 双向心跳：设备每 15 秒发 `ping`，30 秒没有 `pong` 视为断线并重连。
- 重连采用指数退避，上限 30 秒；网络切换、亮屏、充电状态变化时立即重试一次。

## 消息模型

| 类型 | 形态 | 方向 | 说明 |
| --- | --- | --- | --- |
| 请求 | 有 `id`，有 `result` 或 `error` | host → 设备 | 一次性操作 |
| 事件 | 无 `id`，`method` 固定为 `events` | 设备 → host | 状态变化 |
| 心跳 | `ping` / `pong` | 双向 | 不占用 `id` |

请求超时默认 10 秒，`ui.wait`、`app.launch` 这类可以显式传 `timeout_ms`。超时后 host 可以发 `$/cancel` 放弃等待，设备端不必回滚已经发生的副作用。

```json
{"jsonrpc":"2.0","id":7,"method":"ui.snapshot","params":{"filter":"actionable"}}
{"jsonrpc":"2.0","id":7,"result":{"seq":12,"nodes":[]}}
```

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

## 应用

### apps.list

列出可启动的应用。默认只返回带 LAUNCHER 入口的包。

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `include_system` | bool | 是否包含系统应用，默认 false |

```json
{"jsonrpc":"2.0","id":1,"result":{"apps":[
  {"package":"com.tencent.mm","label":"微信","launchable":true}
]}}
```

### app.launch

启动应用。返回只表示请求已被接受，真正成功以 `ui.window_changed` 为准。

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `package` | string | 目标包名 |
| `activity` | string? | 指定 Activity，省略则用 LAUNCHER 入口 |
| `force_stop` | bool | 启动前是否强制停止 |

## 界面

### ui.snapshot

返回当前窗口的节点树。`seq` 对应该次快照观察到的最后一个事件序号，host 用它判断后续事件是否已经包含在快照里。

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `filter` | `"actionable"` \| `"all"` | 默认 `actionable`，剪掉不可见与不可操作子树 |
| `max_nodes` | int | 上限，默认 800，超出时 `truncated` 为 true |

```json
{"jsonrpc":"2.0","id":2,"result":{
  "seq": 42,
  "package": "com.tencent.mm",
  "activity": ".ui.LauncherUI",
  "truncated": false,
  "nodes": [
    {"id":"n1","parent":null,"class":"android.widget.FrameLayout",
     "bounds":[0,0,1080,2400],"flags":{},
     "actions":[]},
    {"id":"n7","parent":"n1","class":"android.widget.Button",
     "text":"发送","view_id":"com.tencent.mm:id/send",
     "bounds":[880,2100,1040,2200],
     "flags":{"clickable":true,"enabled":true},
     "actions":["click"]}
  ]
}}
```

节点字段：

| 字段 | 说明 |
| --- | --- |
| `id` | 本次快照内唯一，跨快照失效 |
| `parent` | 父节点 id，根节点为 null |
| `text` / `desc` | 可见文本与 contentDescription |
| `view_id` | `viewIdResourceName`，可能为 null |
| `bounds` | `[left, top, right, bottom]`，屏幕像素 |
| `flags` | clickable、editable、scrollable、checkable、enabled、focused、selected |
| `actions` | 该节点实际支持的动作 |

节点 id 只在一次快照内有效。host 保存的是节点指纹，不是 id；需要再次操作时用新快照匹配，匹配失败就重新观察，不允许拿旧 id 直接点。

### ui.act

按节点执行语义动作。

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `node` | string | 快照里的节点 id |
| `action` | string | `click`、`long_click`、`set_text`、`clear_text`、`focus`、`scroll_forward`、`scroll_backward`、`select` |
| `text` | string? | `set_text` 使用 |

执行顺序固定为：节点动作 → 最近的可操作祖先 → 坐标手势。节点动作不被支持时返回 `2002`，由 host 决定是否降级。

### ui.gesture

执行原始手势，用于多指、拖动和节点动作覆盖不到的界面。

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `strokes` | array | 每个 stroke 是 `{points:[[x,y],...], start_ms?:int, duration_ms?:int}` |
| `timeout_ms` | int? | 默认 5000 |

同一手势最多 10 个 stroke，总时长不超过 60 秒，坐标超出屏幕视为无效参数。

### ui.wait

等待界面进入稳定状态，替代固定 sleep。

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `until` | string | `idle`、`window`、`text` |
| `value` | string? | `window` 是包名，`text` 是子串 |
| `timeout_ms` | int? | 默认 5000 |

`idle` 表示连续 300 毫秒没有新的界面事件。

### ui.screenshot

按需抓取一帧，返回二进制帧而不是 base64。

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `max_width` | int? | 缩放上限，默认不缩放 |

## 通知

### notifications.list

返回当前活跃通知。没有历史回填，服务启用之前发生的事看不到。

```json
{"jsonrpc":"2.0","id":8,"result":{"notifications":[
  {"key":"0|com.tencent.mm|1|null|10001",
   "package":"com.tencent.mm",
   "title":"小明","text":"明天几点出发？",
   "posted_at":1762000000000,
   "ongoing":false,
   "actions":[{"index":0,"title":"回复","remote_input":true}]}
]}}
```

### notifications.reply

通过通知自带的 RemoteInput 回复，不打开界面。

| 参数 | 类型 | 说明 |
| --- | --- | --- |
| `key` | string | 通知的 key |
| `text` | string | 回复内容 |
| `action_index` | int | 默认为第一个带 remote input 的 action |

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

| 事件 | data | 合并策略 |
| --- | --- | --- |
| `notification.posted` | 通知内容 | 不合并，按 key 更新 |
| `notification.removed` | `{key}` | 不合并 |
| `ui.window_changed` | `{package, activity}` | 保留最后一条 |
| `ui.content_changed` | `{package, activity, kinds}` | 300ms 内合并 |
| `ui.text_changed` | `{node_hint, text}` | 300ms 内合并 |
| `ui.idle` | `{since_seq}` | 保留最后一条 |
| `screen.state` | `{on, interactive, locked}` | 保留最后一条 |

规则：

- 可合并事件在设备端合并，host 只收到合并后的结果。
- 不可合并事件写入环形缓冲，溢出时设备发一条 `lost: true` 的 `ui.content_changed`，host 收到后丢弃增量、重新快照。
- host 发现 `seq` 跳号时不做补发请求，直接全量对账：重新 `ui.snapshot` 与 `notifications.list`。
- 事件只表示变化，不表示最终状态。任何需要准确状态的判断都以快照为准。

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
