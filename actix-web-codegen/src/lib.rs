#![recursion_limit = "512"]
//! Actix-web codegen module
//!
//! Generators for routes and scopes
//!
//! ## Route
//!
//! Macros:
//!
//! - [get](attr.get.html)
//! - [post](attr.post.html)
//! - [put](attr.put.html)
//! - [delete](attr.delete.html)
//!
//! ### Attributes:
//!
//! - `"path"` - Raw literal string with path for which to register handle. Mandatory.
//! - `guard="function_name"` - Registers function as guard using `actix_web::guard::fn_guard`
//!
//! ## Notes
//!
//! Function name can be specified as any expression that is going to be accessible to the generate
//! code (e.g `my_guard` or `my_module::my_guard`)
//!
//! ## Example:
//!
//! ```rust
//! use actix_web::HttpResponse;
//! use actix_web_codegen::get;
//! use futures::{future, Future};
//!
//! #[get("/test")]
//! fn async_test() -> impl Future<Item=HttpResponse, Error=actix_web::Error> {
//!     future::ok(HttpResponse::Ok().finish())
//! }
//! ```

extern crate proc_macro;

mod route;

use proc_macro::TokenStream;
use syn::parse_macro_input;

/// Creates route handler with `GET` method guard.
///
/// Syntax: `#[get("path"[, attributes])]`
///
/// ## Attributes:
///
/// - `"path"` - Raw literal string with path for which to register handler. Mandatory.
/// - `guard="function_name"` - Registers function as guard using `actix_web::guard::fn_guard`
#[proc_macro_attribute]
pub fn get(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as syn::AttributeArgs);
    let gen = route::Args::new(&args, input, route::GuardType::Get);
    gen.generate()
}

/// Creates route handler with `POST` method guard.
///
/// Syntax: `#[post("path"[, attributes])]`
///
/// Attributes are the same as in [get](attr.get.html)
#[proc_macro_attribute]
pub fn post(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as syn::AttributeArgs);
    let gen = route::Args::new(&args, input, route::GuardType::Post);
    gen.generate()
}

/// Creates route handler with `PUT` method guard.
///
/// Syntax: `#[put("path"[, attributes])]`
///
/// Attributes are the same as in [get](attr.get.html)
#[proc_macro_attribute]
pub fn put(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as syn::AttributeArgs);
    let gen = route::Args::new(&args, input, route::GuardType::Put);
    gen.generate()
}

/// Creates route handler with `DELETE` method guard.
///
/// Syntax: `#[delete("path"[, attributes])]`
///
/// Attributes are the same as in [get](attr.get.html)
#[proc_macro_attribute]
pub fn delete(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as syn::AttributeArgs);
    let gen = route::Args::new(&args, input, route::GuardType::Delete);
    gen.generate()
}
