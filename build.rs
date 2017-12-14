extern crate skeptic;
extern crate version_check;

use std::{env, fs};


#[cfg(unix)]
fn main() {
    let f = env::var("OUT_DIR").unwrap() + "/skeptic-tests.rs";
    if env::var("USE_SKEPTIC").is_ok() {
        let _ = fs::remove_file(f);
        // generates doc tests for `README.md`.
        skeptic::generate_doc_tests(
            &["README.md",
              "guide/src/qs_1.md",
              "guide/src/qs_2.md",
              "guide/src/qs_3.md",
              "guide/src/qs_3_5.md",
              "guide/src/qs_4.md",
              "guide/src/qs_4_5.md",
              "guide/src/qs_5.md",
              "guide/src/qs_7.md",
              "guide/src/qs_9.md",
              "guide/src/qs_10.md",
              "guide/src/qs_12.md",
              "guide/src/qs_13.md",
            ]);
    } else {
        let _ = fs::File::create(f);
    }

    match version_check::is_nightly() {
        Some(true) => println!("cargo:rustc-cfg=actix_nightly"),
        Some(false) => (),
        None => (),
    };
}

#[cfg(not(unix))]
fn main() {
    match version_check::is_nightly() {
        Some(true) => println!("cargo:rustc-cfg=actix_nightly"),
        Some(false) => (),
        None => (),
    };
}
