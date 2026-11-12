//! Wire contract shared by the host and the Android device.
//!
//! Types here are the single source of truth: serde defines the JSON shape,
//! `typeshare-core` generates the Kotlin/TypeScript mirrors, `schemars` feeds
//! OpenRPC and documentation. `Device` method signatures and their doc comments
//! define the client, Kotlin service interface, dispatch and reference docs.

extern crate self as gui_use_protocol;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use typeshare::typeshare;

pub use gui_use_protocol_macros::rpc_service;
pub use schemars;
pub use serde;

mod rpc;
pub use rpc::*;

/// 一个参数的静态描述，保留签名中的顺序、类型与注释。
#[derive(Debug, Clone, Copy)]
pub struct ParameterSpec {
    pub name: &'static str,
    pub rust_type: &'static str,
    pub docs: &'static str,
}

/// 一个方法的静态描述，供代码生成与文档使用。
#[derive(Debug, Clone, Copy)]
pub struct MethodSpec {
    pub ident: &'static str,
    pub name: &'static str,
    pub params: &'static str,
    pub result: &'static str,
    pub args: &'static [ParameterSpec],
    pub docs: &'static str,
    pub schema: fn() -> (schemars::Schema, schemars::Schema),
}

#[derive(Debug, Clone, Copy)]
pub struct ServiceSpec {
    pub name: &'static str,
    pub docs: &'static str,
    pub methods: &'static [MethodSpec],
    /// Generated request declarations, parsed in memory by typeshare-core.
    pub types_source: &'static str,
}

/// 一个事件的静态描述。
#[derive(Debug, Clone, Copy)]
pub struct EventSpec {
    pub ident: &'static str,
    pub name: &'static str,
    pub data: &'static str,
    pub merge: &'static str,
    pub docs: &'static str,
    pub schema: fn() -> schemars::Schema,
}

/// OpenRPC uses JSON Schema Draft 7; keep all exported schemas on that dialect.
pub fn rpc_schema<T: JsonSchema>() -> schemars::Schema {
    schemars::generate::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
}

/// 声明事件注册表，供代码生成与文档使用。
#[macro_export]
macro_rules! events {
    ($($(#[doc = $doc:literal])* $marker:ident => $data:ty, $name:literal, $merge:literal;)*) => {
        $(
            $(#[doc = $doc])*
            #[derive(Debug, Clone, Copy)]
            pub struct $marker;

            impl $crate::RpcEvent for $marker {
                const NAME: &'static str = $name;
                type Data = $data;
            }
        )*

        pub const EVENTS: &[$crate::EventSpec] = &[
            $($crate::EventSpec {
                ident: stringify!($marker),
                name: $name,
                data: stringify!($data),
                merge: $merge,
                docs: concat!($($doc, "\n",)*),
                schema: $crate::rpc_schema::<$data>,
            },)*
        ];
    };
}

pub fn find_method(name: &str) -> Option<&'static MethodSpec> {
    METHODS.iter().find(|m| m.name == name)
}

// ---------------------------------------------------------------- 应用

/// 手机主动连接 host 后的认证与能力协商。
#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeviceInfo {
    pub id: String,
    pub model: String,
    pub sdk: i32,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HelloParams {
    pub protocol: i32,
    pub token: String,
    pub device: DeviceInfo,
    pub capabilities: Vec<String>,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HelloResult {
    pub protocol: i32,
    pub events: Vec<String>,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AppInfo {
    pub package_name: String,
    pub label: String,
    pub launchable: bool,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AppListResult {
    pub apps: Vec<AppInfo>,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AppLaunchResult {
    pub accepted: bool,
}

// ---------------------------------------------------------------- 界面

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Bounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NodeFlags {
    pub clickable: Option<bool>,
    pub editable: Option<bool>,
    pub scrollable: Option<bool>,
    pub checkable: Option<bool>,
    pub enabled: Option<bool>,
    pub focused: Option<bool>,
    pub selected: Option<bool>,
    pub checked: Option<bool>,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Node {
    pub id: String,
    pub parent: Option<String>,
    pub class_name: String,
    pub text: Option<String>,
    pub desc: Option<String>,
    pub view_id: Option<String>,
    pub bounds: Bounds,
    pub flags: NodeFlags,
    pub actions: Vec<String>,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UiSnapshotResult {
    pub seq: i32,
    pub package_name: String,
    pub activity: String,
    pub truncated: bool,
    pub nodes: Vec<Node>,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UiActResult {
    pub changed: bool,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GesturePoint {
    pub x: i32,
    pub y: i32,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GestureStroke {
    pub points: Vec<GesturePoint>,
    pub start_ms: Option<i32>,
    pub duration_ms: Option<i32>,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UiGestureResult {
    pub performed: bool,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UiWaitResult {
    pub matched: bool,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UiScreenshotInfo {
    pub mime: String,
    pub width: i32,
    pub height: i32,
    pub bytes: i32,
}

// ---------------------------------------------------------------- 通知

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NotificationAction {
    pub index: i32,
    pub title: String,
    pub remote_input: bool,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NotificationInfo {
    pub key: String,
    pub package_name: String,
    pub title: Option<String>,
    pub text: Option<String>,
    pub posted_at: String,
    pub ongoing: bool,
    pub actions: Vec<NotificationAction>,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NotificationListResult {
    pub notifications: Vec<NotificationInfo>,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NotificationReplyResult {
    pub sent: bool,
}

/// Android 设备提供的操作。Rust 通过 `DeviceClient` 调用，Kotlin 实现同名接口。
#[rpc_service]
pub trait Device {
    /// 列出可启动的应用。
    ///
    /// 默认只返回带 LAUNCHER 入口的包。
    ///
    /// ```json
    /// {"include_system": false}
    /// ```
    ///
    /// ```json
    /// {"apps": [{"package_name": "com.tencent.mm", "label": "微信", "launchable": true}]}
    /// ```
    #[rpc(name = "app.list")]
    async fn app_list(
        /// 是否包含系统应用，默认 false。
        include_system: Option<bool>,
    ) -> AppListResult;

    /// 启动应用。
    ///
    /// 返回只表示请求已被接受，真正成功以 `ui.window_changed` 为准。
    ///
    /// ```json
    /// {"package_name": "com.tencent.mm"}
    /// ```
    ///
    /// ```json
    /// {"accepted": true}
    /// ```
    #[rpc(name = "app.launch")]
    async fn app_launch(
        package_name: String,
        /// 指定 Activity，省略则用 LAUNCHER 入口。
        activity: Option<String>,
        /// 非特权 Android 应用不能强制停止其他应用；true 返回 2002。
        force_stop: Option<bool>,
    ) -> AppLaunchResult;

    /// 读取当前窗口的节点树。
    ///
    /// `seq` 对应该次快照观察到的最后一个事件序号。
    /// 节点 id 只在本次快照内有效，跨快照需要按指纹重新匹配。
    ///
    /// ```json
    /// {"filter": "actionable"}
    /// ```
    ///
    /// ```json
    /// {"seq": 42, "package_name": "com.tencent.mm", "activity": ".ui.LauncherUI", "truncated": false, "nodes": []}
    /// ```
    #[rpc(name = "ui.snapshot")]
    async fn ui_snapshot(
        /// `actionable` 或 `all`，默认 `actionable`。
        filter: Option<String>,
        /// 节点上限，默认 800。
        max_nodes: Option<i32>,
    ) -> UiSnapshotResult;

    /// 按节点执行语义动作。
    ///
    /// 节点动作不被支持时返回 `2002`，由调用方决定是否改用祖先节点或手势。
    ///
    /// ```json
    /// {"node": "n7", "action": "click"}
    /// ```
    ///
    /// ```json
    /// {"changed": true}
    /// ```
    #[rpc(name = "ui.act")]
    async fn ui_act(
        node: String,
        /// `click`、`long_click`、`set_text`、`clear_text`、`focus`、`scroll_forward`、`scroll_backward`、`select`。
        action: String,
        /// `set_text` 使用。
        text: Option<String>,
    ) -> UiActResult;

    /// 执行原始手势。
    ///
    /// 同一手势最多 10 个 stroke，总时长不超过 60 秒。
    ///
    /// ```json
    /// {"strokes": [{"points": [{"x": 540, "y": 1800}, {"x": 540, "y": 600}], "duration_ms": 300}]}
    /// ```
    ///
    /// ```json
    /// {"performed": true}
    /// ```
    #[rpc(name = "ui.gesture")]
    async fn ui_gesture(strokes: Vec<GestureStroke>, timeout_ms: Option<i32>) -> UiGestureResult;

    /// 等待界面进入稳定状态。
    ///
    /// `idle` 表示连续 300 毫秒没有新的界面事件。
    ///
    /// ```json
    /// {"until": "idle", "timeout_ms": 5000}
    /// ```
    ///
    /// ```json
    /// {"matched": true}
    /// ```
    #[rpc(name = "ui.wait")]
    async fn ui_wait(
        /// `idle`、`window` 或 `text`。
        until: String,
        /// `window` 时为包名，`text` 时为子串。
        value: Option<String>,
        timeout_ms: Option<i32>,
    ) -> UiWaitResult;

    /// 抓取一帧画面。
    ///
    /// JSON-RPC 返回图片元数据；图片字节使用独立的二进制消息传输。
    ///
    /// ```json
    /// {"max_width": 720}
    /// ```
    ///
    /// ```json
    /// {"mime": "image/png", "width": 540, "height": 1200, "bytes": 184320}
    /// ```
    #[rpc(name = "ui.screenshot")]
    async fn ui_screenshot(
        /// 缩放上限，默认不缩放。
        max_width: Option<i32>,
    ) -> UiScreenshotInfo;

    /// 读取当前活跃通知。
    ///
    /// 没有历史回填；服务连接时仍然活跃的通知可以读取。
    ///
    /// ```json
    /// {"limit": 20}
    /// ```
    ///
    /// ```json
    /// {"notifications": [{"key": "0|com.tencent.mm|1|null|10001", "package_name": "com.tencent.mm", "title": "小明", "text": "明天几点出发？", "posted_at": "2026-11-12T01:23:45.678Z", "ongoing": false, "actions": []}]}
    /// ```
    #[rpc(name = "notification.list")]
    async fn notification_list(limit: Option<i32>) -> NotificationListResult;

    /// 通过通知自带的 RemoteInput 回复。
    ///
    /// 不打开界面。
    ///
    /// ```json
    /// {"key": "0|com.tencent.mm|1|null|10001", "text": "八点"}
    /// ```
    ///
    /// ```json
    /// {"sent": true}
    /// ```
    #[rpc(name = "notification.reply")]
    async fn notification_reply(
        key: String,
        text: String,
        /// 默认为第一个带 remote input 的 action。
        action_index: Option<i32>,
    ) -> NotificationReplyResult;
}

// ---------------------------------------------------------------- 事件数据

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NotificationPostedData {
    pub key: String,
    pub package_name: String,
    pub title: Option<String>,
    pub text: Option<String>,
    pub posted_at: String,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NotificationRemovedData {
    pub key: String,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UiWindowChangedData {
    pub package_name: String,
    pub activity: String,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UiContentChangedData {
    pub package_name: String,
    pub activity: String,
    pub kinds: Vec<String>,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UiTextChangedData {
    pub node_hint: String,
    pub text: String,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UiIdleData {
    pub since_seq: i32,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ScreenStateData {
    pub on: bool,
    pub interactive: bool,
    pub locked: bool,
}

events! {
    /// 通知出现或内容更新。
    NotificationPosted => NotificationPostedData, "notification.posted", "不合并，按 key 更新";
    /// 通知被撤下。
    NotificationRemoved => NotificationRemovedData, "notification.removed", "不合并";
    /// 前台窗口变化。
    UiWindowChanged => UiWindowChangedData, "ui.window_changed", "保留最后一条";
    /// 界面内容变化。
    UiContentChanged => UiContentChangedData, "ui.content_changed", "300ms 内合并";
    /// 输入框文本变化。
    UiTextChanged => UiTextChangedData, "ui.text_changed", "300ms 内合并";
    /// 连续 300 毫秒没有界面事件。
    UiIdle => UiIdleData, "ui.idle", "保留最后一条";
    /// 熄屏、锁屏、交互状态变化。
    ScreenState => ScreenStateData, "screen.state", "保留最后一条";
}
