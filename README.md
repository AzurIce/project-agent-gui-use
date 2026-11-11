# project-agent-gui-use

让 agent 把一台真实 Android 手机当作可读、可操作、可感知的“身体”。

手机端不是远程控制器，而是一个常驻的能力守护进程：无障碍服务提供界面树与语义动作，通知监听提供通知快照与事件，host 侧只负责决策。

## 技术栈

| 层 | 选择 |
| --- | --- |
| 手机端 | Kotlin + Android framework（无 AppCompat/Compose 依赖） |
| host 端 | Rust |
| 通信 | JSON-RPC 2.0 over WebSocket，手机主动外连 |
| 环境 | Nix flake（JDK 21、Android SDK 36、Gradle、Rust） |

adb 只用于首次安装、开权限和救火，不进入运行时链路。

## 结构

```
android/           手机端 App
  app/src/main/java/dev/azurice/guiuse/
    MainActivity.kt         权限状态与引导
    DeviceService.kt        无障碍：界面树、动作、事件
    NotificationService.kt  通知：快照与增量事件
crates/host/       host 端（设备会话、协议、agent 工具）
docs/              设计文档
```

## 环境

```bash
nix develop
```

进入后 `JAVA_HOME`、`ANDROID_HOME`、`adb`、`gradle`、`cargo` 都已就位。

## 构建

```bash
just build-android   # ./gradlew :app:assembleDebug
just install-android # 安装到已连接设备
just build-host
```

## 路线图

1. 应用清单
2. 打开应用
3. 读取 UI 树
4. 操作 UI 树
5. 读取通知
6. 通知事件推送

配套基础设施：设备会话与鉴权、前台/屏幕状态事件、事件总线（seq/ack/重放）、语义捷径优先。

## 状态

脚手架阶段。两个 Service 只有生命周期日志，协议与 host 端尚未实现。
