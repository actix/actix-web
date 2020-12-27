#[test]
fn compile_macros() {
    let t = trybuild::TestCases::new();

    t.pass("tests/trybuild/simple.rs");
    t.compile_fail("tests/trybuild/simple-fail.rs");

    t.pass("tests/trybuild/route-ok.rs");
    t.compile_fail("tests/trybuild/route-missing-method-fail.rs");
    t.compile_fail("tests/trybuild/route-duplicate-method-fail.rs");
    t.compile_fail("tests/trybuild/route-unexpected-method-fail.rs");
}

// #[rustversion::not(nightly)]
// fn skip_on_nightly(t: &trybuild::TestCases) {
//
// }

// #[rustversion::nightly]
// fn skip_on_nightly(_t: &trybuild::TestCases) {}
