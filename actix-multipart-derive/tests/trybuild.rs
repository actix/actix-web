#[rustversion::stable(1.72)] // MSRV
#[test]
fn compile_macros() {
    let t = trybuild::TestCases::new();

    t.pass("tests/trybuild/all-required.rs");
    t.pass("tests/trybuild/optional-and-list.rs");
    t.pass("tests/trybuild/rename.rs");
    t.pass("tests/trybuild/deny-unknown.rs");

    t.pass("tests/trybuild/deny-duplicates.rs");
    t.compile_fail("tests/trybuild/deny-parse-fail.rs");

    t.pass("tests/trybuild/size-limits.rs");
    t.compile_fail("tests/trybuild/size-limit-parse-fail.rs");
}
