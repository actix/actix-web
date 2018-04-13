extern crate version_check;

fn main() {
    match version_check::is_nightly() {
        Some(true) => println!("cargo:rustc-cfg=actix_nightly"),
        Some(false) => (),
        None => (),
    };
}
