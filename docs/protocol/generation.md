# 协议定义与生成

Rust 是协议的唯一来源。业务数据类型、服务签名、事件声明和 `///` 注释统一维护在 `crates/gui-use-protocol/src/lib.rs`。

## 服务声明

服务使用 tarpc 风格的普通参数列表，不写 `&self`、请求包装或传输错误类型：

```rust
#[rpc_service]
pub trait Device {
    /// 启动应用。
    #[rpc(name = "app.launch")]
    async fn app_launch(
        package_name: String,
        /// 指定 Activity，省略则用 LAUNCHER 入口。
        activity: Option<String>,
        force_stop: Option<bool>,
    ) -> AppLaunchResult;
}
```

宏生成 `AppLaunchParams`、实现 `RpcMethod` 的 `AppLaunchRequest` 和 `DeviceClient`。Rust 调用返回 `RpcResult<AppLaunchResult>`：

```rust
device.app_launch("com.example.app".into(), None, None).await?;
```

需要泛型入口时，方法标记绑定方法名、参数和结果，不能独立配错：

```rust
rpc.call::<AppLaunchRequest>(AppLaunchParams {
    package_name: "com.example.app".into(),
    activity: None,
    force_stop: None,
}).await?;
```

线上 `params` 始终为按参数名构成的 JSON 对象，不是位置数组。单个业务配置对象也是一个命名参数，不会自动展开其字段。`Option<T>` 允许省略或传 `null`；宏不从注释推断业务默认值，默认行为由设备实现负责。

Kotlin 自动得到对应的普通参数接口及 JSON 分发：

```kotlin
interface Device {
    suspend fun appLaunch(
        packageName: String,
        activity: String? = null,
        forceStop: Boolean? = null,
    ): AppLaunchResult
}
```

设备只实现生成的接口，不维护第二份协议定义。函数和参数使用 Kotlin 驼峰命名，数据类字段保留 Rust 的线缆名称。

## 事件

事件声明关联标记类型、数据类型和线缆名称，没有 Result：

```rust
events! {
    /// 通知被撤下。
    NotificationRemoved => NotificationRemovedData, "notification.removed", "不合并";
}
```

Rust 用 `RpcNotification::event::<NotificationRemoved>(seq, ts, data)` 构造信封，用 `decode_event::<NotificationRemoved>()` 校验并读取载荷。Kotlin 用生成的 `ProtocolEvents.notificationRemoved(seq, ts, data, json)` 构造相同信封。连接与发送由公共 WebSocket 运行时负责。

## 生成与构建

在仓库根目录运行：

```bash
cargo run --locked -p gui-use-protocol-codegen
```

也可用 `just protocol`。生成器直接链接 `typeshare-core`，业务 DTO 与宏导出的 Params 在内存中一起解析，不需要 Typeshare CLI、Node、`cargo expand` 或 Gradle 生成插件。

产物包括：

- `android/app/src/main/java/dev/azurice/guiuse/protocol/Types.kt`：数据类型。
- `android/app/src/main/java/dev/azurice/guiuse/protocol/Methods.kt`：接口、分发及事件构造。
- `protocol/gui-use.d.ts`：TypeScript 数据类型。
- `protocol/openrpc.json`：OpenRPC 导出。
- `protocol/registry.json`：Notist 消费的方法、类型、事件及原始 Markdown 注释。

生成产物应随 Rust 定义一起提交。CI 运行 `cargo run --locked -p gui-use-protocol-codegen -- --check`，只比较、不写文件，产物过期时失败。

普通 Android 构建直接运行 `android/gradlew`，只消费已提交的 Kotlin，不运行 Cargo 或生成器。Rust host 编译同样不生成 Kotlin。Notist 的 `.notc` 组件与 SSG extension 消费导出数据来构造页面。

## 类型与 OpenRPC

业务 DTO 使用 Serde、`JsonSchema` 和 `#[typeshare]`；生成器使用 Typeshare 自身的类型映射生成数据类和接口参数，避免两份映射。服务方法不支持泛型、借用参数或模式解构。整数优先使用 `i32`，时间使用 RFC 3339 字符串；Typeshare 不支持的类型会让生成失败。

OpenRPC 与 Kotlin 由同一份 Rust 定义并行生成，Kotlin 不经过 OpenRPC 中转。OpenRPC 的公共类型使用 JSON Schema Draft 7，引用归入 `components.schemas`，方法参数结构为 `by-name`。

设备推送事件在 `x-events` 中记录名称、载荷 Schema 和 `x-merge` 策略；`x-event-envelope` 记录固定的 `events` 方法及各字段位置。这些扩展描述设备到 host 的 notification，不把事件伪装成有响应的请求。
