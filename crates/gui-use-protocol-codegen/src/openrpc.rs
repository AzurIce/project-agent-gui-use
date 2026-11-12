//! OpenRPC is an export alongside Kotlin, not an input to Kotlin generation.

use std::collections::{BTreeMap, BTreeSet};

use gui_use_protocol::{EVENTS, METHODS, SERVICE};
use serde_json::{Value, json};

#[derive(Default)]
struct Schemas(BTreeMap<String, Value>);

fn pointer_name(name: &str) -> String {
    name.replace('~', "~0").replace('/', "~1")
}

fn reference(name: &str) -> Value {
    json!({"$ref": format!("#/components/schemas/{}", pointer_name(name))})
}

fn rewrite_refs(schema: &mut Value, root: &str) {
    match schema {
        Value::Object(object) => {
            if let Some(Value::String(value)) = object.get_mut("$ref") {
                if value == "#" {
                    *value = format!("#/components/schemas/{}", pointer_name(root));
                } else if let Some(name) = value.strip_prefix("#/definitions/") {
                    *value = format!("#/components/schemas/{name}");
                }
            }
            for value in object.values_mut() {
                rewrite_refs(value, root);
            }
        }
        Value::Array(values) => {
            for value in values {
                rewrite_refs(value, root);
            }
        }
        _ => {}
    }
}

impl Schemas {
    fn insert(&mut self, name: String, schema: Value) {
        if let Some(existing) = self.0.get(&name) {
            assert_eq!(
                existing, &schema,
                "conflicting protocol schemas named {name}"
            );
        } else {
            self.0.insert(name, schema);
        }
    }

    fn add(&mut self, name: &str, mut schema: Value) -> Value {
        if let Some(object) = schema.as_object_mut() {
            object.remove("$schema");
            if let Some(Value::Object(definitions)) = object.remove("definitions") {
                for (definition, mut value) in definitions {
                    rewrite_refs(&mut value, name);
                    self.insert(definition, value);
                }
            }
        }
        rewrite_refs(&mut schema, name);
        self.insert(name.into(), schema);
        reference(name)
    }
}

pub fn generate() -> Value {
    let mut schemas = Schemas::default();
    let mut methods = Vec::new();
    for method in METHODS {
        let (params, result) = (method.schema)();
        let params = serde_json::to_value(params).expect("parameter schema");
        let result = serde_json::to_value(result).expect("result schema");
        schemas.add(method.params, params.clone());
        let result_name = result
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| format!("{}Result", method.params.trim_end_matches("Params")));
        let result_ref = schemas.add(&result_name, result);
        let required: BTreeSet<_> = params
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let parameters: Vec<_> = method
            .args
            .iter()
            .map(|arg| {
                let mut schema = params["properties"][arg.name].clone();
                assert!(
                    !schema.is_null(),
                    "missing parameter schema for {}",
                    arg.name
                );
                rewrite_refs(&mut schema, method.params);
                json!({
                    "name": arg.name,
                    "description": arg.docs,
                    "required": required.contains(arg.name),
                    "schema": schema,
                })
            })
            .collect();
        methods.push(json!({
            "name": method.name,
            "summary": method.docs.lines().next().unwrap_or(""),
            "description": method.docs,
            "paramStructure": "by-name",
            "params": parameters,
            "result": {"name": "result", "schema": result_ref},
        }));
    }

    let mut names = BTreeSet::new();
    let events: Vec<_> = EVENTS
        .iter()
        .map(|event| {
            assert!(names.insert(event.name), "duplicate event {}", event.name);
            let schema = serde_json::to_value((event.schema)()).expect("event schema");
            let reference = schemas.add(event.data, schema);
            json!({
                "name": event.name,
                "description": event.docs.trim(),
                "schema": reference,
                "x-merge": event.merge,
            })
        })
        .collect();

    json!({
        "openrpc": "1.3.2",
        "info": {
            "title": SERVICE.name,
            "version": env!("CARGO_PKG_VERSION"),
            "description": SERVICE.docs,
        },
        "methods": methods,
        "components": {"schemas": schemas.0},
        "x-events": events,
        "x-event-envelope": {
            "direction": "device-to-host",
            "method": "events",
            "typeField": "params.type",
            "dataField": "params.data",
            "sequenceField": "params.seq",
            "timestampField": "params.ts",
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_refs(value: &Value, document: &Value) {
        match value {
            Value::Object(object) => {
                if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                    let pointer = reference.strip_prefix('#').expect("local schema reference");
                    assert!(
                        document.pointer(pointer).is_some(),
                        "unresolved {reference}"
                    );
                    assert!(reference.starts_with("#/components/schemas/"));
                }
                assert!(!object.contains_key("$defs"));
                assert!(!object.contains_key("$schema"));
                for value in object.values() {
                    check_refs(value, document);
                }
            }
            Value::Array(values) => {
                for value in values {
                    check_refs(value, document);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn exports_methods_parameter_order_docs_events_and_resolved_schemas() {
        let doc = generate();
        assert_eq!(doc["openrpc"], "1.3.2");
        assert_eq!(doc["methods"].as_array().unwrap().len(), METHODS.len());
        assert_eq!(doc["methods"][0]["description"], METHODS[0].docs);
        assert_eq!(doc["methods"][0]["params"][0]["required"], false);
        assert_eq!(doc["methods"][1]["params"][0]["name"], "package_name");
        assert_eq!(doc["methods"][1]["params"][0]["required"], true);
        assert_eq!(doc["methods"][1]["params"][1]["name"], "activity");
        assert!(doc["components"]["schemas"]["Node"].is_object());
        assert_eq!(doc["x-events"].as_array().unwrap().len(), EVENTS.len());
        assert_eq!(doc["x-events"][0]["name"], "notification.posted");
        check_refs(&doc, &doc);
    }

    #[test]
    fn root_and_definition_refs_are_hoisted_and_escaped() {
        let mut schemas = Schemas::default();
        schemas.add("Tree", json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "properties": {"children": {"items": {"$ref": "#"}}, "value": {"$ref": "#/definitions/A~1B"}},
            "definitions": {"A/B": {"type": "string"}},
        }));
        assert_eq!(
            schemas.0["Tree"]["properties"]["children"]["items"],
            reference("Tree")
        );
        assert_eq!(schemas.0["Tree"]["properties"]["value"], reference("A/B"));
        assert!(schemas.0.contains_key("A/B"));
    }

    #[test]
    #[should_panic(expected = "conflicting protocol schemas")]
    fn conflicting_names_fail_instead_of_overwriting_a_type() {
        let mut schemas = Schemas::default();
        schemas.add("Type", json!({"type": "string"}));
        schemas.add("Type", json!({"type": "boolean"}));
    }
}
