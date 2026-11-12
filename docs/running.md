# 运行与真机测试

## 构建

进入 `nix develop`，在仓库根目录运行：

```bash
cargo build --locked -p gui-use-host -j8
./android/gradlew -p android :app:assembleDebug --max-workers=4
```

普通 Android 构建不调用 Rust 或协议生成器。修改协议后先运行 `just protocol`，再提交生成产物。

## 配对

host 首次使用时创建专用身份目录：

```bash
cargo run --locked -p gui-use-host -- init
```

默认位置为 `$XDG_STATE_HOME/gui-use`，未设置时使用 `~/.local/state/gui-use`。也可在子命令前加 `--state-dir PATH`。目录权限为 0700，私钥、证书与 `pairing.json` 权限为 0600；已有身份不会被覆盖。不要把此目录、token 或配对文件提交到仓库。

在手机上安装 APK，打开 GUI Use：

1. 启用本应用的无障碍与通知访问权限。
2. 从私有 `pairing.json` 填入 endpoint、token、certificate_sha256。
3. 配置允许操作的包名，默认仅有 `dev.azurice.guiuse`。不要加入银行、钱包或其他敏感应用。
4. 点击 Save and connect，保持前台服务通知开启。

客户端仅接受 `wss://`，使用明确配对的证书 SHA-256 指纹验证 TLS；token 在加密后的 `hello` 中发送，不放 URL。错误 token 或证书会被拒绝。更换 host 身份需要重新配对。

## 启动 host 与调用

```bash
cargo run --locked -p gui-use-host -- serve
```

默认仅监听 `127.0.0.1:8765`。USB 测试可对明确选择的手机执行 `adb -s SERIAL reverse tcp:8765 tcp:8765`，手机 endpoint 使用 `wss://localhost:8765`。LAN 使用时显式指定 `serve --listen HOST_IP:8765`，并在手机填写相应地址；不要在未配置防火墙的公网暴露服务。

host 运行后，在另一个终端使用：

```bash
target/debug/gui-use-host devices
target/debug/gui-use-host apps --include-system
target/debug/gui-use-host call app.list --params '{"include_system":true}'
target/debug/gui-use-host call app.launch --params '{"package_name":"dev.azurice.guiuse","activity":".DiagnosticsActivity"}'
target/debug/gui-use-host call ui.snapshot --params '{"filter":"all","max_nodes":800}'
target/debug/gui-use-host events
target/debug/gui-use-host call ui.screenshot --params '{"max_width":720}' --output screenshot.png
```

多个设备连接时用 `--device ID` 选择。CLI 通过身份目录内权限为 0600 的 Unix socket 控制 host，没有第二个未认证的 HTTP 控制端口。`--timeout-ms` 设置 host 等待时间。截图输出只创建新文件，不覆盖已有文件。

Rust 调用方可以用 `DeviceClient::new(gui_use_host::client::LocalTransport::new(directory, device_id))` 连接运行中的本地 host，再调用 `client.app_list(Some(false)).await?` 等生成方法。host 内部使用 `WsTransport`，同一连接上统一分配线缆请求 ID。

## 支持范围与限制

- `app.list`：枚举可启动应用，可选择是否包含系统应用。
- `app.launch`：启动白名单中的应用或明确指定的 Activity；`force_stop=true` 返回 2002，普通 Android 应用没有强制停止其他应用的权限。
- `ui.snapshot`：读取活动窗口，可选择完整或可操作节点，节点上限 10000。节点 ID 只属于最近一次快照。
- `ui.act`：点击、长按、输入、清空、聚焦、滚动、选中；不支持的节点动作返回 2002，过期节点返回 2001。
- `ui.gesture`：最多 10 个 stroke，每个最多 1000 个点，整体不超过 60 秒。
- `ui.wait`：等待空闲、窗口包名或文本；超时返回 2004。
- `ui.screenshot`：Android 11 及以上，PNG 通过相关联的二进制帧传输，最大 8 MiB。Android 14 及以上只截取当前可访问窗口；旧系统回退到显示屏截图。截图前后均检查白名单、安全策略及窗口标识，窗口变化时丢弃截图。安全窗口不可截图。
- `notification.list`、`notification.reply`：只处理白名单中非敏感应用的活动通知；回复使用通知自身的 free-form RemoteInput。多个输入字段无法由单个 text 明确区分时返回 2002，不猜测目标。
- 事件包含 UI 窗口、内容、文本、空闲、屏幕状态以及通知出现/撤下。UI 内容按 300 毫秒去抖。

设备只在用户开启会话后连接，不注册开机自启；断线指数退避重连，上限 30 秒。网络变化、亮屏和充电状态变化会唤醒重试。双方使用 WebSocket ping/pong，不把心跳混入业务事件。

默认白名单、设备锁定与密码字段检查是主要保护。敏感包名/页面关键词检查是保守启发式，不是对所有敏感界面的完整检测。界面和通知内容都是不可信数据，不能把其中的文本自动当成 host 指令。

## 自动测试

不连接手机的检查：

```bash
cargo test --workspace --locked -j8 -- --test-threads=4
cargo clippy --workspace --all-targets --locked -j8 -- -D warnings
just protocol-check
./android/gradlew -p android :app:testDebugUnitTest --max-workers=4
python3 -m unittest discover -s scripts -p 'test_*.py'
```

真机测试必须明确提供 OPPO 的 adb serial，不会自动选择或操作其他连接的手机：

```bash
python3 scripts/device_smoke.py --serial SERIAL
```

脚本只操作本应用的 DiagnosticsActivity 和本地产生的测试通知。RemoteInput 回复仅写回本应用，不发送真实聊天消息、短信、邮件或支付请求。检查 TLS/握手、应用枚举与启动、实时树与节点动作、手势、等待、PNG 二进制帧、通知列表/回复/事件、白名单与密码页面拒绝、并发请求及 host 重启重连。

脚本在专用临时目录生成身份，退出时停止测试 host、关闭测试会话、移除该次 adb reverse，并恢复原有的无障碍配置、通知访问和应用配对配置。APK 保留安装。只对明确写着 GUI Use 的 OPPO 无障碍说明框选择“保持开启”，不会批准其他应用的权限提示。OPPO 固件首次安装可能有厂商确认页面；已安装构建可用 `--skip-build --skip-install` 重跑测试。不要通过关闭系统验证或输入真实账户密码来绕过安装限制。

debug APK 含有受 `android.permission.DUMP` 保护的 `DebugControlReceiver`，供本机 adb 测试配置连接与测试页面。release APK 不包含此入口。
