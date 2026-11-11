#import "../_components/pele/lib.notc" as pele;

# 连接与握手

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

## 重连与对账

1. 连接断开后按退避重连，重新走 `hello`。
2. 新会话的 `seq` 从 1 开始，host 丢弃上一会话的增量缓存。
3. 对账顺序固定：`ui.snapshot` → `notifications.list` → 继续消费事件。
4. 对账期间收到的事件先入队，不丢弃；对账完成后按 `seq` 与快照的 `seq` 去重。
