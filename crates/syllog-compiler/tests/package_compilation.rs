//! Package compiler input and linking contracts.

use syllog_compiler::{PackageSource, compile_package};

#[test]
fn package_compiler_rejects_empty_and_duplicate_source_sets() {
    let empty = compile_package(Vec::new());
    assert!(!empty.success());
    assert_eq!(empty.diagnostics[0].code, "SYL9001");

    let source = PackageSource {
        file: "src/main.syl".into(),
        source: "module app;\nfn main() -> I64 { 0 }\n".into(),
    };
    let duplicate = compile_package(vec![source.clone(), source]);
    assert!(!duplicate.success());
    assert_eq!(duplicate.diagnostics[0].code, "SYL9002");
    assert!(duplicate.hir.is_none());
}
