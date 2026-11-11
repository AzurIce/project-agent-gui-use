//! Wire contract shared by the host and the Android device.
//!
//! Types here are the single source of truth: serde defines the JSON shape,
//! `typeshare` generates the Kotlin/TypeScript mirrors, and `methods!` records
//! the method registry that drives code generation.

use serde::{Deserialize, Serialize};
use typeshare::typeshare;

/// 一个方法的静态描述，供代码生成与文档使用。
#[derive(Debug, Clone, Copy)]
pub struct MethodSpec {
    pub ident: &'static str,
    pub name: &'static str,
    pub params: &'static str,
    pub result: &'static str,
    pub summary: &'static str,
}

/// 声明方法注册表。每个条目同时决定 Rust 侧的元数据和生成产物。
#[macro_export]
macro_rules! methods {
    ($($ident:ident : $name:literal => $params:ty, $result:ty, $summary:literal;)*) => {
        pub const METHODS: &[$crate::MethodSpec] = &[
            $(
                $crate::MethodSpec {
                    ident: stringify!($ident),
                    name: $name,
                    params: stringify!($params),
                    result: stringify!($result),
                    summary: $summary,
                },
            )*
        ];
    };
}

pub fn find_method(name: &str) -> Option<&'static MethodSpec> {
    METHODS.iter().find(|m| m.name == name)
}

// ---------------------------------------------------------------- 应用

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInfo {
    pub package_name: String,
    pub label: String,
    pub launchable: bool,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppListParams {
    pub include_system: Option<bool>,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppListResult {
    pub apps: Vec<AppInfo>,
}

// ---------------------------------------------------------------- 界面

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bounds {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSnapshotParams {
    pub filter: Option<String>,
    pub max_nodes: Option<i32>,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSnapshotResult {
    pub seq: i32,
    pub package_name: String,
    pub activity: String,
    pub truncated: bool,
    pub nodes: Vec<Node>,
}

// ---------------------------------------------------------------- 通知

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationAction {
    pub index: i32,
    pub title: String,
    pub remote_input: bool,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationListParams {
    pub limit: Option<i32>,
}

#[typeshare]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationListResult {
    pub notifications: Vec<NotificationInfo>,
}

methods! {
    app_list: "app.list" => AppListParams, AppListResult, "列出可启动的应用";
    ui_snapshot: "ui.snapshot" => UiSnapshotParams, UiSnapshotResult, "读取当前窗口的节点树";
    notification_list: "notification.list" => NotificationListParams, NotificationListResult, "读取当前活跃通知";
}
