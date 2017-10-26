extern crate skeptic;
use std::{env, fs};


#[cfg(unix)]
fn main() {
    if env::var("USE_SKEPTIC").is_ok() {
        // generates doc tests for `README.md`.
        skeptic::generate_doc_tests(&["README.md"]);
    } else {
        let f = env::var("OUT_DIR").unwrap() + "/skeptic-tests.rs";
        let _ = fs::File::create(f);
    }
}

#[cfg(not(unix))]
fn main() {
}
