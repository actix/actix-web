#[test]
fn compile_macros() {
    let t = trybuild::TestCases::new();

    t.pass("tests/trybuild/01-basic.rs");
    t.pass("tests/trybuild/02-max-size.rs");
    // t.pass("tests/trybuild/03-inert-filter.rs");
    // t.compile_fail("tests/trybuild/02-only-async.rs");
}
