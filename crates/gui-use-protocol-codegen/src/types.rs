//! Embed Typeshare rather than invoking an external executable.

use std::collections::{BTreeMap, HashMap};

use gui_use_protocol::SERVICE;
use typeshare_core::{
    context::{ParseContext, ParseFileContext},
    language::{CrateName, Kotlin, Language, TypeScript},
    parser::{ParsedData, parse},
    reconcile::reconcile_aliases,
    rust_types::RustType,
};

pub fn kotlin_language() -> Kotlin {
    Kotlin {
        package: "dev.azurice.guiuse.protocol".into(),
        ..Default::default()
    }
}

pub fn kotlin_type(rust_type: &str) -> String {
    let ty: RustType = rust_type.parse().expect("supported Typeshare RPC type");
    kotlin_language()
        .format_type(&ty, &[])
        .expect("format Kotlin RPC type")
}

fn parse_types(source: String) -> ParsedData {
    let crate_name: CrateName = "gui_use_protocol".into();
    let parsed = parse(
        &ParseContext::default(),
        ParseFileContext {
            source_code: source,
            crate_name: crate_name.clone(),
            file_name: "lib.rs".into(),
            file_path: "gui-use-protocol/src/lib.rs".into(),
        },
    )
    .expect("parse protocol type source")
    .expect("protocol has Typeshare definitions");
    assert!(
        parsed.errors.is_empty(),
        "Typeshare errors: {:?}",
        parsed.errors
    );
    let mut crates = BTreeMap::from([(crate_name.clone(), parsed)]);
    reconcile_aliases(&mut crates);
    crates.remove(&crate_name).expect("parsed protocol crate")
}

pub fn generate(language: &mut dyn Language) -> String {
    // The service macro exports its generated Params as source. Typeshare parses
    // those declarations with the business DTOs, without macro expansion or files.
    let source = format!(
        "{}\n{}",
        include_str!("../../gui-use-protocol/src/lib.rs"),
        SERVICE.types_source,
    );
    let mut output = Vec::new();
    language
        .generate_types(&mut output, &HashMap::new(), parse_types(source))
        .expect("generate protocol types");
    String::from_utf8(output).expect("generated UTF-8 types")
}

pub fn mirrors() -> Vec<(&'static str, String)> {
    vec![
        (super::KOTLIN_TYPES_PATH, generate(&mut kotlin_language())),
        (super::TYPESCRIPT_PATH, generate(&mut TypeScript::default())),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_generator_sees_macro_generated_params_and_business_types() {
        let kotlin = generate(&mut kotlin_language());
        for expected in [
            "data class AppListParams",
            "data class Node",
            "val include_system: Boolean? = null",
            "data class UiGestureParams",
            "val strokes: List<GestureStroke>",
            "是否包含系统应用，默认 false。",
        ] {
            assert!(kotlin.contains(expected), "missing {expected}");
        }
        let ts = generate(&mut TypeScript::default());
        assert!(ts.contains("export interface AppListParams"));
        assert!(ts.contains("include_system?: boolean"));
    }

    #[test]
    fn method_types_use_the_same_mapping_as_data_classes() {
        for (rust, kotlin) in [
            ("bool", "Boolean"),
            ("i32", "Int"),
            ("String", "String"),
            ("Option<Vec<AppInfo>>", "List<AppInfo>?"),
            ("()", "Unit"),
        ] {
            assert_eq!(kotlin_type(rust), kotlin);
        }
    }
}
