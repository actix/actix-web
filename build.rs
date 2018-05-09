extern crate version_check;

fn main() {
    let mut has_impl_trait = true;

    match version_check::is_min_version("1.26.0") {
        Some((true, _)) => println!("cargo:rustc-cfg=actix_impl_trait"),
        _ => (),
    };
    match version_check::is_nightly() {
        Some(true) => {
            println!("cargo:rustc-cfg=actix_nightly");
            println!("cargo:rustc-cfg=actix_impl_trait");
        }
        Some(false) => (),
        None => (),
    };
}
