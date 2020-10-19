#[test]
fn compile_macros() {
    let t = trybuild::TestCases::new();

    t.pass("tests/trybuild/simple.rs");
    t.compile_fail("tests/trybuild/simple-fail.rs");

    t.pass("tests/trybuild/route-ok.rs");

    test_route_duplicate_unexpected_method(&t);
    test_route_missing_method(&t)
}

#[rustversion::stable(1.42)]
fn test_route_missing_method(t: &trybuild::TestCases) {
    t.compile_fail("tests/trybuild/route-missing-method-fail-msrv.rs");
}

#[rustversion::not(stable(1.42))]
#[rustversion::not(nightly)]
fn test_route_missing_method(t: &trybuild::TestCases) {
    t.compile_fail("tests/trybuild/route-missing-method-fail.rs");
}

#[rustversion::nightly]
fn test_route_missing_method(_t: &trybuild::TestCases) {}

// FIXME: Re-test them on nightly once rust-lang/rust#77993 is fixed.
#[rustversion::not(nightly)]
fn test_route_duplicate_unexpected_method(t: &trybuild::TestCases) {
    t.compile_fail("tests/trybuild/route-duplicate-method-fail.rs");
    t.compile_fail("tests/trybuild/route-unexpected-method-fail.rs");
}

#[rustversion::nightly]
fn test_route_duplicate_unexpected_method(_t: &trybuild::TestCases) {}
