//! Standard-library source, manifest, and compatibility contracts.

use std::path::{Path, PathBuf};

use serde_json::json;
use syllog_parser::{FunctionNode, Item, TypeKind, TypeNode, parse_syl};

const LIBRARIES: [&str; 5] = ["core", "alloc", "io", "async", "provider"];

fn repository() -> PathBuf {
    std::fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .expect("repository path should resolve")
}

#[test]
fn every_standard_library_is_a_valid_package_and_compiles_cleanly() {
    let repository = repository();
    for library in LIBRARIES {
        let root = repository.join("library").join(library);
        let manifest = syllog_project::load_manifest(&root.join("Syllog.toml"))
            .unwrap_or_else(|diagnostics| panic!("{library} manifest failed: {diagnostics:#?}"));
        assert_eq!(manifest.package.name, format!("syllog-{library}"));
        assert_eq!(manifest.targets.len(), 1);
        if library == "core" {
            assert!(manifest.dependencies.is_empty());
            assert_eq!(manifest.capabilities.profile, "none");
        } else {
            assert!(manifest.dependencies.contains_key("syllog-core"));
        }

        let source_path = root.join("src/lib.syl");
        let source = std::fs::read_to_string(&source_path).unwrap();
        let report = syllog_compiler::compile(source_path.display().to_string(), &source);
        assert!(
            report.diagnostics.is_empty(),
            "{library} failed: {:#?}",
            report.diagnostics
        );
    }
}

#[test]
fn public_api_snapshot_matches_every_exported_symbol() {
    let repository = repository();
    let expected = json!({
        "schema_version": 1,
        "libraries": {
            "alloc": ["fn checked_capacity(U64,U64)->Result<U64,ReserveError>", "type ByteBuffer", "type ReserveError"],
            "async": ["async fn ready(TaskCapability,U64)->U64", "type TaskCapability"],
            "core": ["fn clamp_i64(I64,I64,I64)->I64", "fn compare_i64(I64,I64)->Ordering", "fn max_i64(I64,I64)->I64", "fn min_i64(I64,I64)->I64", "type Ordering"],
            "io": ["fn validate_read(IoCapability,U64)->Result<U64,IoError>", "type IoCapability", "type IoError"],
            "provider": ["fn validate_request(ProviderCapability,String)->Result<String,ProviderError>", "type ProviderCapability", "type ProviderError"]
        }
    });
    let snapshot: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repository.join("library/api-v1.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(snapshot, expected);

    for library in LIBRARIES {
        let source =
            std::fs::read_to_string(repository.join("library").join(library).join("src/lib.syl"))
                .unwrap();
        let ast = parse_syl(&source).unwrap();
        let mut exports = ast
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Struct(node) if node.public => Some(format!("type {}", node.name)),
                Item::Enum(node) if node.public => Some(format!("type {}", node.name)),
                Item::Function(node) if node.public => Some(function_signature(node)),
                _ => None,
            })
            .collect::<Vec<_>>();
        exports.sort();
        assert_eq!(snapshot["libraries"][library], json!(exports));
    }
}

fn function_signature(node: &FunctionNode) -> String {
    let prefix = if node.asynchronous { "async " } else { "" };
    let parameters = node
        .parameters
        .iter()
        .map(|parameter| type_text(&parameter.ty))
        .collect::<Vec<_>>()
        .join(",");
    let result = node
        .return_type
        .as_ref()
        .map_or_else(|| "()".to_owned(), type_text);
    format!("{prefix}fn {}({parameters})->{result}", node.name)
}

fn type_text(node: &TypeNode) -> String {
    match &node.kind {
        TypeKind::Path {
            segments,
            arguments,
        } => {
            let path = segments.join("::");
            if arguments.is_empty() {
                path
            } else {
                format!(
                    "{path}<{}>",
                    arguments
                        .iter()
                        .map(type_text)
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
        }
        TypeKind::Array(element) => format!("[{}]", type_text(element)),
        TypeKind::Tuple(elements) => format!(
            "({})",
            elements.iter().map(type_text).collect::<Vec<_>>().join(",")
        ),
    }
}

#[test]
fn authority_bearing_apis_require_explicit_capability_parameters() {
    let repository = repository();
    for (library, function, capability) in [
        ("io", "validate_read", "IoCapability"),
        ("async", "ready", "TaskCapability"),
        ("provider", "validate_request", "ProviderCapability"),
    ] {
        let source =
            std::fs::read_to_string(repository.join("library").join(library).join("src/lib.syl"))
                .unwrap();
        let ast = parse_syl(&source).unwrap();
        let Item::Function(node) = ast
            .items
            .iter()
            .find(|item| matches!(item, Item::Function(node) if node.name == function))
            .unwrap()
        else {
            unreachable!()
        };
        assert!(node.public);
        let capability_type = ast
            .items
            .iter()
            .find_map(|item| match item {
                Item::Struct(node) if node.name == capability => Some(node),
                _ => None,
            })
            .unwrap();
        assert!(capability_type.public);
        assert!(capability_type.fields.iter().all(|field| !field.public));
        assert!(node.parameters.iter().any(|parameter| {
            matches!(
                &parameter.ty.kind,
                TypeKind::Path { segments, arguments }
                    if arguments.is_empty() && segments == &[capability]
            )
        }));
    }
}
