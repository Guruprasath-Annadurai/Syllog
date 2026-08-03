//! Cross-file module graph, visibility, and invalidation contracts.

use syllog_parser::parse_syl;
use syllog_semantic::{ModuleSource, analyze_modules};

fn source(file: &str, text: &str) -> ModuleSource {
    ModuleSource {
        file: file.to_owned(),
        ast: parse_syl(text).expect("module fixture should parse"),
    }
}

#[test]
fn public_imports_and_aliases_resolve_to_stable_qualified_definitions() {
    let math = source(
        "src/math.syl",
        "module math;\npub fn add(value: I64) -> I64 { value }\nfn secret() -> I64 { 0 }\n",
    );
    let app = source(
        "src/app.syl",
        "module app;\nuse math::add as sum;\nfn main() -> I64 { sum(41) }\n",
    );

    let analysis = analyze_modules(vec![app, math]);

    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    let math = analysis.module("math").expect("math module should exist");
    let app = analysis.module("app").expect("app module should exist");
    assert_eq!(app.imports["sum"].definition, math.definitions["add"].id);
    assert_eq!(app.imports["sum"].span.line, 2);
    assert_ne!(app.id, math.id);
    assert!(
        analysis.source_analyses["src/app.syl"]
            .symbols
            .contains_value("sum")
    );
}

#[test]
fn unimported_cross_file_values_remain_unresolved() {
    let library = source(
        "src/library.syl",
        "module library;\npub fn available() -> I64 { 1 }\n",
    );
    let app = source(
        "src/app.syl",
        "module app;\nfn main() -> I64 { available() }\n",
    );

    let analysis = analyze_modules(vec![library, app]);

    assert!(analysis.diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "SYL2003"
            && diagnostic.file == "src/app.syl"
            && diagnostic.message.contains("available")
    }));
}

#[test]
fn private_and_unknown_imports_are_rejected_at_the_use_site() {
    let library = source(
        "src/library.syl",
        "module library;\nfn hidden() -> I64 { 0 }\n",
    );
    let app = source(
        "src/app.syl",
        "module app;\nuse library::hidden;\nuse missing::value;\nfn main() -> I64 { 0 }\n",
    );

    let analysis = analyze_modules(vec![library, app]);
    let diagnostics = analysis
        .diagnostics
        .iter()
        .map(|diagnostic| {
            (
                diagnostic.code.as_str(),
                diagnostic.file.as_str(),
                diagnostic.span.line,
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(
        diagnostics,
        [("SYL2402", "src/app.syl", 2), ("SYL2401", "src/app.syl", 3),]
    );
}

#[test]
fn duplicate_exports_across_files_in_one_module_are_deterministic() {
    let first = source(
        "src/first.syl",
        "module shared;\npub fn duplicate() -> I64 { 1 }\n",
    );
    let second = source(
        "src/second.syl",
        "module shared;\npub fn duplicate() -> I64 { 2 }\n",
    );

    let forward = analyze_modules(vec![first.clone(), second.clone()]);
    let reverse = analyze_modules(vec![second, first]);

    assert_eq!(forward, reverse);
    assert_eq!(forward.diagnostics.len(), 1);
    assert_eq!(forward.diagnostics[0].code, "SYL2403");
    assert_eq!(forward.diagnostics[0].file, "src/second.syl");
    assert_eq!(forward.diagnostics[0].span.line, 2);
}

#[test]
fn dependency_cycles_report_the_complete_stable_cycle() {
    let a = source(
        "src/a.syl",
        "module a;\nuse b::run_b;\npub fn run_a() -> I64 { 1 }\n",
    );
    let b = source(
        "src/b.syl",
        "module b;\nuse a::run_a;\npub fn run_b() -> I64 { 2 }\n",
    );

    let analysis = analyze_modules(vec![b, a]);

    let cycles = analysis
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "SYL2404")
        .collect::<Vec<_>>();
    assert_eq!(cycles.len(), 1);
    assert_eq!(cycles[0].message, "module dependency cycle: a -> b -> a");
    assert_eq!(cycles[0].file, "src/a.syl");
    assert_eq!(cycles[0].span.line, 2);
}

#[test]
fn public_interface_hash_ignores_private_bodies_but_tracks_public_signatures() {
    let original = analyze_modules(vec![source(
        "src/api.syl",
        "module api;\npub fn stable(value: I64) -> I64 { value }\nfn detail() -> I64 { 1 }\n",
    )]);
    let private_change = analyze_modules(vec![source(
        "src/api.syl",
        "module api;\npub fn stable(value: I64) -> I64 { value }\nfn detail() -> I64 { 999 }\n",
    )]);
    let public_change = analyze_modules(vec![source(
        "src/api.syl",
        "module api;\npub fn stable(value: I64) -> Bool { true }\nfn detail() -> I64 { 999 }\n",
    )]);

    let original = original.module("api").unwrap().interface_hash;
    assert_eq!(
        original,
        private_change.module("api").unwrap().interface_hash
    );
    assert_ne!(
        original,
        public_change.module("api").unwrap().interface_hash
    );
}

#[test]
fn public_interface_hash_is_independent_of_source_layout() {
    let compact = analyze_modules(vec![source(
        "src/api.syl",
        "module api;\npub enum Status { Ready, Failed(I64) }\n",
    )]);
    let expanded = analyze_modules(vec![source(
        "src/api.syl",
        "module api;\n\npub enum Status {\n    Ready,\n    Failed(I64),\n}\n",
    )]);

    assert_eq!(
        compact.module("api").unwrap().interface_hash,
        expanded.module("api").unwrap().interface_hash
    );
}
