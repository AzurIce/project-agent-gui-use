//! Wire contract shared by the host and the Android device.
//!
//! Types here are the single source of truth: serde defines the JSON shape,
//! `typeshare` generates the Kotlin/TypeScript mirrors, `schemars` feeds the
//! documentation generator, and `methods!` records the registry that drives
//! code generation.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use typeshare::typeshare;

pub use schemars;

/// 一个方法的静态描述，供代码生成与文档使用。
#[derive(Debug, Clone, Copy)]
pub struct MethodSpec {
    pub ident: &'static str,
    pub name: &'static str,
    pub params: &'static str,
    pub result: &'static str,
    pub summary: &'static str,
    pub notes: &'static str,
    pub request: &'static str,
    pub response: &'static str,
}

/// 一个事件的静态描述。
#[derive(Debug, Clone, Copy)]
pub struct EventSpec {
    pub name: &'static str,
    pub data: &'static str,
    pub merge: &'static str,
    pub notes: &'static str,
}

/// 声明事件注册表，供代码生成与文档使用。
#[macro_export]
macro_rules! events {
    ($($name:literal => $data:ty, $merge:literal, $notes:literal;)*) => {
        pub const EVENTS: &[$crate::EventSpec] = &[
            $($crate::EventSpec {
                name: $name,
                data: stringify!($data),
                merge: $merge,
                notes: $notes,
            },)*
        ];

        pub fn event_schemas() -> Vec<schemars::Schema> {
            vec![$( $crate::schemars::schema_for!($data), )*]
        }
    };
}

/// 声明方法注册表。每个条目同时决定 Rust 元数据、schema 和生成产物。
#[macro_export]
macro_rules! methods {
    ($(
        $ident:ident : $name:literal => $params:ty, $result:ty,
        $summary:literal, $notes:literal, $request:literal, $response:literal;
    )*) => {
        pub const METHODS: &[$crate::MethodSpec] = &[
            $(
                $crate::MethodSpec {
                    ident: stringify!($ident),
                    name: $name,
                    params: stringify!($params),
                    result: stringify!($result),
                    summary: $summary,
                    notes: $notes,
                    request: $request,
                    response: $response,
                },
            )*
        ];

        pub fn schemas() -> Vec<(schemars::Schema, schemars::Schema)> {
            vec![
                $(
                    (
                        $crate::schemars::schema_for!($params),
                        $crate::schemars::schema_for!($result),
                    ),
                )*
            ]
        }
    };
}

pub fn find_method(name: &str) -> Option<&'static MethodSpec> {
    METHODS.iter().find(|m| m.name == name)
}

// ---------------------------------------------------------------- 应用

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AppInfo {
    pub package_name: String,
    pub label: String,
    pub launchable: bool,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AppListParams {
    #[schemars(description = "是否包含系统应用，默认 false")]
    pub include_system: Option<bool>,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AppListResult {
    pub apps: Vec<AppInfo>,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AppLaunchParams {
    pub package_name: String,
    #[schemars(description = "指定 Activity，省略则用 LAUNCHER 入口")]
    pub activity: Option<String>,
    #[schemars(description = "启动前是否强制停止")]
    pub force_stop: Option<bool>,
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
pub struct UiSnapshotParams {
    #[schemars(description = "`actionable` 或 `all`，默认 `actionable`")]
    pub filter: Option<String>,
    #[schemars(description = "节点上限，默认 800")]
    pub max_nodes: Option<i32>,
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
pub struct UiActParams {
    pub node: String,
    #[schemars(
        description = "`click`、`long_click`、`set_text`、`clear_text`、`focus`、`scroll_forward`、`scroll_backward`、`select`"
    )]
    pub action: String,
    #[schemars(description = "`set_text` 使用")]
    pub text: Option<String>,
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
pub struct UiGestureParams {
    pub strokes: Vec<GestureStroke>,
    pub timeout_ms: Option<i32>,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UiGestureResult {
    pub performed: bool,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UiWaitParams {
    #[schemars(description = "`idle`、`window` 或 `text`")]
    pub until: String,
    #[schemars(description = "`window` 时为包名，`text` 时为子串")]
    pub value: Option<String>,
    pub timeout_ms: Option<i32>,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UiWaitResult {
    pub matched: bool,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UiScreenshotParams {
    #[schemars(description = "缩放上限，默认不缩放")]
    pub max_width: Option<i32>,
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
pub struct NotificationListParams {
    pub limit: Option<i32>,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NotificationListResult {
    pub notifications: Vec<NotificationInfo>,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NotificationReplyParams {
    pub key: String,
    pub text: String,
    #[schemars(description = "默认为第一个带 remote input 的 action")]
    pub action_index: Option<i32>,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NotificationReplyResult {
    pub sent: bool,
}

methods! {
    app_list: "app.list" => AppListParams, AppListResult,
        "列出可启动的应用",
        "默认只返回带 LAUNCHER 入口的包。",
        r#"{"include_system": false}"#,
        r#"{"apps": [{"package_name": "com.tencent.mm", "label": "微信", "launchable": true}]}"#;
    app_launch: "app.launch" => AppLaunchParams, AppLaunchResult,
        "启动应用",
        "返回只表示请求已被接受，真正成功以 `ui.window_changed` 为准。",
        r#"{"package_name": "com.tencent.mm"}"#,
        r#"{"accepted": true}"#;
    ui_snapshot: "ui.snapshot" => UiSnapshotParams, UiSnapshotResult,
        "读取当前窗口的节点树",
        "`seq` 对应该次快照观察到的最后一个事件序号。节点 id 只在本次快照内有效，跨快照需要按指纹重新匹配。",
        r#"{"filter": "actionable"}"#,
        r#"{"seq": 42, "package_name": "com.tencent.mm", "activity": ".ui.LauncherUI", "truncated": false, "nodes": []}"#;
    ui_act: "ui.act" => UiActParams, UiActResult,
        "按节点执行语义动作",
        "执行顺序为节点动作、最近的可操作祖先、坐标手势。节点动作不被支持时返回 `2002`。",
        r#"{"node": "n7", "action": "click"}"#,
        r#"{"changed": true}"#;
    ui_gesture: "ui.gesture" => UiGestureParams, UiGestureResult,
        "执行原始手势",
        "同一手势最多 10 个 stroke，总时长不超过 60 秒。",
        r#"{"strokes": [{"points": [{"x": 540, "y": 1800}, {"x": 540, "y": 600}], "duration_ms": 300}]}"#,
        r#"{"performed": true}"#;
    ui_wait: "ui.wait" => UiWaitParams, UiWaitResult,
        "等待界面进入稳定状态",
        "`idle` 表示连续 300 毫秒没有新的界面事件。",
        r#"{"until": "idle", "timeout_ms": 5000}"#,
        r#"{"matched": true}"#;
    ui_screenshot: "ui.screenshot" => UiScreenshotParams, UiScreenshotInfo,
        "抓取一帧画面",
        "返回二进制帧而不是 base64。",
        r#"{"max_width": 720}"#,
        r#"{"mime": "image/png", "width": 540, "height": 1200, "bytes": 184320}"#;
    notification_list: "notification.list" => NotificationListParams, NotificationListResult,
        "读取当前活跃通知",
        "没有历史回填，服务启用之前发生的事看不到。",
        r#"{"limit": 20}"#,
        r#"{"notifications": [{"key": "0|com.tencent.mm|1|null|10001", "package_name": "com.tencent.mm", "title": "小明", "text": "明天几点出发？", "posted_at": "2026-11-12T01:23:45.678Z", "ongoing": false, "actions": []}]}"#;
    notification_reply: "notification.reply" => NotificationReplyParams, NotificationReplyResult,
        "通过通知自带的 RemoteInput 回复",
        "不打开界面。",
        r#"{"key": "0|com.tencent.mm|1|null|10001", "text": "八点"}"#,
        r#"{"sent": true}"#;
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
    "notification.posted" => NotificationPostedData, "不合并，按 key 更新", "通知出现或内容更新";
    "notification.removed" => NotificationRemovedData, "不合并", "通知被撤下";
    "ui.window_changed" => UiWindowChangedData, "保留最后一条", "前台窗口变化";
    "ui.content_changed" => UiContentChangedData, "300ms 内合并", "界面内容变化";
    "ui.text_changed" => UiTextChangedData, "300ms 内合并", "输入框文本变化";
    "ui.idle" => UiIdleData, "保留最后一条", "连续 300 毫秒没有界面事件";
    "screen.state" => ScreenStateData, "保留最后一条", "熄屏、锁屏、交互状态变化";
}
