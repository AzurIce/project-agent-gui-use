# 请求方法

方法只有在 `hello` 的 `capabilities` 中声明后才可调用，未声明的方法返回 `-32601`。

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

按需抓取一帧，返回二进制帧而不是 base64，帧格式见[设备协议](../protocol.md#二进制帧)。

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
