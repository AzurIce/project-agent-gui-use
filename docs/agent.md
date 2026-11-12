# Android PTC agent

`gui-use-agent` 是独立的命令行原型，连接支持 function tools 的 OpenAI-compatible Chat Completions 接口。不依赖 OpenCode、Node 或 Python 代码执行环境，也不改变手机协议。

## 执行方式

模型只看到一个工具 `android_code({code})`。工具参数是 JavaScript 异步函数体，可以直接使用 `await` 和 `return`：

```javascript
const screen = await android.ui.snapshot({filter: "all"});
const inputs = screen.nodes.filter(node => node.flags.editable);
return inputs.map(({id, text, view_id}) => ({id, text, view_id}));
```

完整快照在本机 QuickJS 子进程内处理，只有 `print(...)`、返回值、错误和执行统计进入模型上下文。循环、条件、列表筛选以及多次 Android 调用可在一次工具执行中完成，无需让模型接收每个中间响应。

调用链为：自然语言任务 → 模型生成 JavaScript → 独立 QuickJS worker → Rust 调用代理 → host 私有 Unix socket → 手机。worker 没有模型 API key、配对 token、host socket 路径或设备选择权。

JavaScript 包装与参数名直接读取 Rust `METHODS`；`android.help("ui.act")` 的文档和输入/输出 schema 来自 Rust 注释与类型。新增 RPC 必须在 Rust 代理中明确归类为读取或修改，不能自动获得操作权限。

## 构建与前置条件

```bash
nix develop
just build-host
just build-agent
```

USB 联调只需手机安装最新 debug APK、允许 USB 调试。agent 自动创建本地身份、启动 host、映射 USB 端口、导入手机配置并启动手机服务；不需要手动 `init`、`serve` 或填写 token/指纹。USB 导入会自动允许可启动应用，不要求手工维护包名列表。首次 UI 操作仍需在手机启用系统无障碍权限，读取通知需通知访问权限。

已有一台手机连接时直接复用；否则只检查已授权 USB 设备，自动选择唯一安装了 GUI Use 的手机。多台符合条件时显示名称与 serial，使用 `--serial SERIAL` 选择。`--device ID` 仅用于复用已有连接，不是必填参数。

自动启动的 host 在本次 CLI 退出时停止，原本运行的 host 不会被停止。手机服务保持运行、等待下次电脑上线；手机端可主动停止。`--state-dir` 默认仍为 `$XDG_STATE_HOME/gui-use` 或 `~/.local/state/gui-use`，手动启动 host 时需使用相同目录。可以用 `gui-use-agent connect` 保持电脑连接直到 Ctrl-C，不调用模型。

只有 agent 构建需要编译嵌入的 QuickJS C 库；普通 Android 构建不依赖它。当前使用 Unix socket，面向 Linux/macOS host。

## 不用模型先试执行器

```bash
target/debug/gui-use-agent exec --file examples/ptc/inspect.js
target/debug/gui-use-agent --allow-actions exec --file examples/ptc/diagnostics.js
```

`inspect.js` 返回当前窗口的少量可操作节点；`diagnostics.js` 只打开本应用的 DiagnosticsActivity，点击 Increment 三次，并读取计数验证。后者包含 13 个 RPC，但只返回一个精简结果。自有测试页也必须在白名单中。

## 自然语言 agent

默认使用 DeepSeek 官方 `deepseek-flash`；直接读取已有 `DEEPSEEK_API_KEY` 即可。其他模型可配置 `GUI_USE_BASE_URL`（包含 `/v1` 等所需路径）和 `GUI_USE_MODEL`。客户端在 base URL 后追加 `/chat/completions`，不接受 URL 中的凭据或查询参数。远程服务必须使用 HTTPS；本机 loopback 可使用 HTTP。

认证优先级为 `--api-key-file` / `GUI_USE_API_KEY_FILE` → `GUI_USE_API_KEY` → `DEEPSEEK_API_KEY`。本地免认证服务可以不配置 token。不要把 key 放进命令参数、任务、仓库或普通聊天消息。

```bash
target/debug/gui-use-agent --allow-actions run \
  '打开 GUI Use 的 DiagnosticsActivity，找到 Increment，点击三次并核实计数变化。只操作这个本地测试页。'
```

也可以用 `run --base-url URL --model MODEL TASK` 显式配置。模型必须支持 Chat Completions 的 `tool_calls` 消息格式；本原型不直接实现 Anthropic Messages、OpenAI Responses 或 OAuth 登录。未完成的工具响应、多个并行代码调用和未知工具均拒绝执行；非法 JSON 参数返回可修正的工具错误。

任务完成后 stdout 输出 JSON，包括模型答案、`turns`、`steps`、代码执行次数、RPC 次数、原始响应字节、实际工具结果字节，以及累计的服务端 usage。一个用户任务是一个 Turn，其中每次模型请求及其工具执行是一个 Step；最终答案也计为一个 Step。默认不保存会话记录。

### 日志与消耗统计

诊断使用 `tracing` / `tracing-subscriber`，写入 stderr，不混入 stdout JSON 或 worker 通信。默认 INFO 日志包含：

- 每次 LLM 请求完成后的模型名、耗时、输入/输出/总 token、缓存命中/未命中、推理 token 和服务端原始 usage，包括最终答案请求。服务端未提供的字段不填零；不自行估算金额，只有服务端返回费用时才显示。
- 每条 RPC 的方法、脱敏参数、结果摘要、耗时，以及失败或中断。快照、应用、通知和事件列表按数量/元数据摘要；其他业务结果直接显示。
- 每个 Step 的完成状态和累计 RPC/字节统计，以及每次 PTC 执行统计。日志携带 Turn → Step → PTC → RPC 的作用域。

通过 `RUST_LOG` 调整级别，例如 `RUST_LOG=warn,gui_use_agent=debug,gui_use_host=info` 可查看脱敏的完整 RPC 响应。密钥和截图二进制不记录；RPC 参数、普通业务结果及 DEBUG 响应可能包含界面或通知正文，请谨慎分享日志。`RUST_LOG=error` 可关闭常规进度输出。

### DeepSeek Flash

DeepSeek 官方 Chat Completions 配置为 `base_url=https://api.deepseek.com`、`model=deepseek-flash`，不是 `/anthropic` 或 `/responses`。在 [DeepSeek 平台](https://platform.deepseek.com/api_keys)创建 API key，使用私有文件或已有环境变量提供认证。

```bash
export GUI_USE_BASE_URL=https://api.deepseek.com
export GUI_USE_MODEL=deepseek-flash
export GUI_USE_THINKING=disabled
export GUI_USE_API_KEY_FILE="$HOME/.config/gui-use/deepseek.key"
```

上例的 key 文件必须已存在、只包含 token，放在仓库外并使用 0600 权限。若已经设置 `DEEPSEEK_API_KEY`，上述 export 都不需要，直接运行 agent。不要把 token 写入会保留历史的命令或提交到仓库。

`disabled` 适合先验证低延迟的操作链路，也是默认官方 DeepSeek 地址下的行为。需要思考时设为 `enabled`，也可在 `run` 后传 `--thinking enabled`；其他地址未指定时沿用服务端默认。agent 会按 [DeepSeek 工具调用约定](https://api-docs.deepseek.com/guides/thinking_mode)原样保留并回传 `reasoning_content`，但不打印到终端或放进 PTC 工具结果。模型服务自身的上下文长度限制仍适用。

```bash
target/debug/gui-use-agent --allow-actions run \
  '只操作 dev.azurice.guiuse 的 DiagnosticsActivity：点击 Increment 三次，读取操作前后计数并核实差值为 3。不要发送任何通知回复。'
```

命令返回后本次任务结束，目前不是持续聊天界面。若希望点名某台 USB 手机，用 `--serial SERIAL`；不需要复制设备 UUID。

`rpc_response_bytes` 是留在本机的业务响应量，`tool_result_bytes` 是实际送入对话的工具结果量；两者用于观察裁剪效果，**不是总 token 节省率**。总费用还包含提示词、生成的代码、模型推理和重复发送的对话历史。

## JavaScript API

- `android.app.*`、`android.ui.*`、`android.notification.*` 对应现有 RPC，均使用一个命名参数对象，结果为业务对象而非 JSON-RPC 信封。
- `android.help()` 返回简要目录；`android.help("ui.snapshot")` 返回详细 schema，可先在代码中读取后只输出需要的字段。
- `android.host.devices()` 返回连接设备；`android.host.events()` 返回所选设备的有限事件历史，需自行按 `session`、`seq` 筛选并处理对账。
- `state` 是跨代码执行保留的 JSON 对象，最多 64 KiB，不自动发送给模型；脚本成功且返回值未超预算时提交。局部变量不跨执行保存，失败也不回滚手机上的副作用。
- 节点 ID 不能跨新快照使用。需要动作前重新定位；有多个候选时应报错而不是猜测。
- `print(...)` 和 `console.log(...)` 输出可见证据；未显式输出的原始响应不进入对话。
- 截图保存为私有 PNG artifact，只把元数据和 artifact 名称返回 JavaScript。模型**看不到像素**，此原型以 UI 树操作为主，不包含视觉模型图片附件接入。

`gui-use-agent api` 或 `gui-use-agent api ui.act` 可以不连接手机查询接口。

## 权限与执行预算

默认只读。启动应用、节点动作、手势需要 `--allow-actions`；通知回复还需要 `--allow-notification-reply`。这两个开关不是对发送、支付、删除等具体行为的用户授权，任务仍须明确授权目标操作。应用包名白名单及敏感屏幕规则沿用 Android 端，不由模型代码修改。

每次代码执行默认 30 秒、32 KiB 代码、64 MiB JavaScript 堆、512 KiB JavaScript 栈，不限制 RPC 次数。`--code-timeout-ms` 可设为 100–120000 毫秒。打印和返回值共享约 8 KiB 的 JSON payload 预算，另有少量结果信封/统计开销；打印最多 64 条。过大返回值会被拒绝，而不是把完整树截断成难以使用的片段。

任务默认不限制 Step 数、累计 RPC 次数或本地对话字节数，持续到模型完成、调用出错或用户按 Ctrl-C。只有显式传入 `--max-steps N` 时才限制 Step 数，N 可以是任意正整数；`--max-turns` 是同一参数的兼容别名。不设置就没有这一限制。达到显式限制时不会把未完成任务报告成成功。调用按顺序执行，`Promise.all` 不提供真实并发。

worker 每次重新启动、清空环境，没有 shell、文件、模块加载器、任意网络或计时器接口。父进程在总期限到达时杀死 worker，包括它阻塞等待 host 的情况。已经送达设备的动作不能撤销，断线/超时后应先检查现状，不能盲目重试。

这些是原型的能力隔离和资源约束，**不是内核级沙箱**，不防御 QuickJS 或原生依赖漏洞。运行真正敌对代码还需要容器/OS 沙箱。PTC 也不是数据脱敏机制：模型主动打印的界面和通知内容仍会发往所配置的服务；提示注入防御不能只靠 system prompt。

## 测试

```bash
cargo test -p gui-use-agent --locked -j8 -- --test-threads=4
just test-ptc-device SERIAL
```

离线测试使用脚本化模型 HTTP 服务和模拟 host，验证完整模型→代码→RPC→精简结果→下一轮循环、状态保持、裁剪、错误修正、权限、输出/时间/内存预算和私有图片文件。它验证实现链路，不等同于真实模型自主完成任务的成功率测试。

真机测试只操作明确选定的 OPPO 和本应用测试页，复用现有测试脚本的配置恢复/进程清理。无模型执行器测试不会访问远程模型。
