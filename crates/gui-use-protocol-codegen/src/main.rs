//! 从 `gui-use-protocol` 的方法注册表生成 Kotlin 接口与机器可读文档数据。
//!
//! 文档正文由 `docs/_components/protocol/lib.notc` 读取 `protocol/registry.json`
//! 后构造，这里只负责把 Rust 类型编译成数据。
//!
//! 用法：
//!   cargo run -p gui-use-protocol-codegen
//!   cargo run -p gui-use-protocol-codegen -- --check

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use gui_use_protocol::{EVENTS, METHODS, SERVICE};
use serde_json::{Value, json};

mod kotlin;
mod openrpc;
mod types;

const KOTLIN_PATH: &str = "android/app/src/main/java/dev/azurice/guiuse/protocol/Methods.kt";
const REGISTRY_PATH: &str = "protocol/registry.json";
const KOTLIN_TYPES_PATH: &str = "android/app/src/main/java/dev/azurice/guiuse/protocol/Types.kt";
const TYPESCRIPT_PATH: &str = "protocol/gui-use.d.ts";
const OPENRPC_PATH: &str = "protocol/openrpc.json";

fn main() {
    let check = std::env::args().any(|a| a == "--check");
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repository root")
        .to_path_buf();

    let mut files = vec![
        (KOTLIN_PATH, kotlin::generate()),
        (REGISTRY_PATH, registry()),
        (OPENRPC_PATH, pretty(&openrpc::generate())),
    ];
    files.extend(types::mirrors());

    if check {
        let mut stale = Vec::new();
        for (path, content) in &files {
            let full = root.join(path);
            match fs::read_to_string(&full) {
                Ok(existing) if existing == *content => {}
                _ => stale.push(*path),
            }
        }
        if stale.is_empty() {
            println!("protocol artifacts are up to date");
            return;
        }
        for path in stale {
            eprintln!("out of date: {path}");
        }
        std::process::exit(1);
    }

    for (path, content) in &files {
        let full = root.join(path);
        write_if_changed(&full, content);
        println!("wrote {}", full.display());
    }
}

fn write_if_changed(path: &PathBuf, content: &str) {
    if fs::read_to_string(path).ok().as_deref() == Some(content) {
        return;
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create output directory");
    }
    fs::write(path, content).expect("write generated file");
}

fn registry() -> String {
    let mut defs: BTreeMap<String, Value> = BTreeMap::new();
    let mut methods = Vec::new();

    for spec in METHODS {
        let (params, result) = (spec.schema)();
        let params = serde_json::to_value(&params).expect("serialize params schema");
        let result = serde_json::to_value(&result).expect("serialize result schema");
        collect_defs(&params, &mut defs);
        collect_defs(&result, &mut defs);

        methods.push(json!({
            "name": spec.name,
            "summary": spec.docs.lines().next().unwrap_or(""),
            "docs": spec.docs,
            "params_type": spec.params,
            "result_type": spec.result,
            "params": fields_of(&params),
            "result": fields_of(&result),
        }));
    }

    let mut events = Vec::new();
    for spec in EVENTS {
        let schema = serde_json::to_value((spec.schema)()).expect("serialize event schema");
        collect_defs(&schema, &mut defs);
        events.push(json!({
            "name": spec.name,
            "data_type": spec.data,
            "merge": spec.merge,
            "docs": spec.docs.trim(),
            "fields": fields_of(&schema),
        }));
    }

    let types: Vec<Value> = defs
        .iter()
        .map(|(name, schema)| {
            json!({
                "name": name,
                "fields": fields_of(schema),
            })
        })
        .collect();

    let doc = json!({
        "protocol": 1,
        "service": { "name": SERVICE.name, "docs": SERVICE.docs },
        "methods": methods,
        "events": events,
        "types": types,
    });
    pretty(&doc)
}

fn pretty(doc: &Value) -> String {
    let mut text = serde_json::to_string_pretty(doc).expect("serialize protocol JSON");
    text.push('\n');
    text
}

fn collect_defs(schema: &Value, defs: &mut BTreeMap<String, Value>) {
    if let Some(map) = schema.get("definitions").and_then(Value::as_object) {
        for (name, value) in map {
            defs.insert(name.clone(), value.clone());
        }
    }
}

fn fields_of(schema: &Value) -> Vec<Value> {
    let required: BTreeSet<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .collect::<BTreeSet<_>>()
        })
        .unwrap_or_default();

    let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };

    properties
        .iter()
        .map(|(name, value)| {
            json!({
                "name": name,
                "type": render_type(value),
                "required": required.contains(name.as_str()),
                "description": value.get("description").and_then(Value::as_str).unwrap_or(""),
            })
        })
        .collect()
}

fn render_type(schema: &Value) -> String {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        return reference
            .rsplit('/')
            .next()
            .unwrap_or(reference)
            .to_string();
    }

    if let Some(any) = schema.get("anyOf").and_then(Value::as_array) {
        let non_null: Vec<&Value> = any
            .iter()
            .filter(|value| value.get("type").and_then(Value::as_str) != Some("null"))
            .collect();
        let nullable = non_null.len() != any.len();
        let rendered = if non_null.is_empty() {
            "null".to_string()
        } else if non_null.len() == 1 {
            render_type(non_null[0])
        } else {
            non_null
                .iter()
                .map(|value| render_type(value))
                .collect::<Vec<_>>()
                .join(" | ")
        };
        return if nullable {
            format!("{rendered}?")
        } else {
            rendered
        };
    }

    if let Some(types) = schema.get("type") {
        if let Some(list) = types.as_array() {
            let non_null: Vec<&str> = list
                .iter()
                .filter_map(Value::as_str)
                .filter(|value| *value != "null")
                .collect();
            let nullable = non_null.len() != list.len();
            let rendered = non_null
                .iter()
                .map(|value| render_scalar_type(value))
                .collect::<Vec<_>>()
                .join(" | ");
            return if nullable {
                format!("{rendered}?")
            } else {
                rendered
            };
        }
        if let Some(value) = types.as_str() {
            if value == "array" {
                let item = schema
                    .get("items")
                    .map(render_type)
                    .unwrap_or_else(|| "any".to_string());
                return format!("{item}[]");
            }
            return render_scalar_type(value);
        }
    }

    "any".to_string()
}

fn render_scalar_type(name: &str) -> String {
    match name {
        "string" => "string".to_string(),
        "integer" | "number" => "int".to_string(),
        "boolean" => "bool".to_string(),
        "array" => "array".to_string(),
        "object" => "object".to_string(),
        "null" => "null".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_preserves_method_comments_and_infers_schema_from_signatures() {
        let registry: Value = serde_json::from_str(&registry()).unwrap();
        assert_eq!(registry["service"]["name"], "Device");
        assert_eq!(registry["methods"].as_array().unwrap().len(), METHODS.len());
        let method = &registry["methods"][0];
        assert_eq!(method["docs"], METHODS[0].docs);
        assert_eq!(method["params_type"], "AppListParams");
        assert_eq!(method["result_type"], "AppListResult");
        assert_eq!(method["params"][0]["name"], "include_system");
        assert_eq!(
            method["params"][0]["description"],
            "是否包含系统应用，默认 false。"
        );
        assert!(method.get("request").is_none());
        assert!(method.get("notes").is_none());
    }
}
