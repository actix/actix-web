//! Checks that test macro does not cause problems in the presence of imports named "test" that
//! could be either a module with test items or the "test with runtime" macro itself.
//!
//! Before actix/actix-net#399 was implemented, this macro was running twice. The first run output
//! `#[test]` and it got run again and since it was in scope.
//!
//! Prevented by using the fully-qualified test marker (`#[::core::prelude::v1::test]`).

use actix_web::test;

#[actix_web::test]
async fn test_macro_naming_conflict() {
    let _req = test::TestRequest::default();
    assert_eq!(async { 1 }.await, 1);
}
