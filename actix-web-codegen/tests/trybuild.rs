#[rustversion::stable(1.72)] // MSRV
#[test]
fn compile_macros() {
    let t = trybuild::TestCases::new();

    t.pass("tests/trybuild/simple.rs");
    t.compile_fail("tests/trybuild/simple-fail.rs");

    t.pass("tests/trybuild/route-ok.rs");
    t.compile_fail("tests/trybuild/route-missing-method-fail.rs");
    t.compile_fail("tests/trybuild/route-duplicate-method-fail.rs");
    t.compile_fail("tests/trybuild/route-malformed-path-fail.rs");

    t.pass("tests/trybuild/route-custom-method.rs");
    t.compile_fail("tests/trybuild/route-custom-lowercase.rs");

    t.pass("tests/trybuild/routes-ok.rs");
    t.compile_fail("tests/trybuild/routes-missing-method-fail.rs");
    t.compile_fail("tests/trybuild/routes-missing-args-fail.rs");

    t.compile_fail("tests/trybuild/scope-on-handler.rs");
    t.compile_fail("tests/trybuild/scope-missing-args.rs");
    t.compile_fail("tests/trybuild/scope-invalid-args.rs");
    t.compile_fail("tests/trybuild/scope-trailing-slash.rs");

    t.pass("tests/trybuild/docstring-ok.rs");

    t.pass("tests/trybuild/test-runtime.rs");
}
