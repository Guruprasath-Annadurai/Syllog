//! Incremental compiler database contracts.

use std::sync::Arc;

use syllog_compiler::database::{
    CompilerDatabase, IncrementalCompilerDatabase, PackageId, SourceFileId,
};

#[test]
fn editing_one_file_preserves_unrelated_parse_query_identity() {
    let left = SourceFileId(0);
    let right = SourceFileId(1);
    let package = PackageId(0);
    let mut database = IncrementalCompilerDatabase::new();
    database.set_package_files(package, vec![left, right]);
    database.set_source(left, Arc::from("fn left() -> U64 { 1 }"));
    database.set_source(right, Arc::from("fn right() -> U64 { 2 }"));

    let left_before = database.parse(left);
    let right_before = database.parse(right);
    let package_before = database.hir(package);

    database.set_source(left, Arc::from("fn left() -> U64 { 3 }"));

    let left_after = database.parse(left);
    let right_after = database.parse(right);
    let package_after = database.hir(package);
    assert!(!Arc::ptr_eq(&left_before, &left_after));
    assert!(Arc::ptr_eq(&right_before, &right_after));
    assert!(!Arc::ptr_eq(&package_before, &package_after));
    assert!(
        package_after.program.is_some(),
        "{:#?}",
        package_after.diagnostics
    );
    assert_eq!(package_after.program.as_ref().unwrap().modules.len(), 2);
}

#[test]
fn cancellation_never_poison_caches() {
    let file = SourceFileId(0);
    let package = PackageId(0);
    let mut database = IncrementalCompilerDatabase::new();
    database.set_package_files(package, vec![file]);
    database.set_source(file, Arc::from("fn main() -> U64 { 42 }"));

    database.cancel();
    let cancelled = database.hir(package);
    assert!(cancelled.cancelled);
    assert!(cancelled.program.is_none());

    database.reset_cancellation();
    let completed = database.hir(package);
    assert!(!completed.cancelled);
    assert!(completed.program.is_some(), "{:#?}", completed.diagnostics);
}
