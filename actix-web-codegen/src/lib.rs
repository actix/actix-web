//! Routing and runtime macros for Actix Web.
//!
//! # Actix Web Re-exports
//! Actix Web re-exports a version of this crate in it's entirety so you usually don't have to
//! specify a dependency on this crate explicitly. Sometimes, however, updates are made to this
//! crate before the actix-web dependency is updated. Therefore, code examples here will show
//! explicit imports. Check the latest [actix-web attributes docs] to see which macros
//! are re-exported.
//!
//! # Runtime Setup
//! Used for setting up the actix async runtime. See [macro@main] macro docs.
//!
//! ```
//! #[actix_web_codegen::main] // or `#[actix_web::main]` in Actix Web apps
//! async fn main() {
//!     async { println!("Hello world"); }.await
//! }
//! ```
//!
//! # Single Method Handler
//! There is a macro to set up a handler for each of the most common HTTP methods that also define
//! additional guards and route-specific middleware.
//!
//! See docs for: [GET], [POST], [PATCH], [PUT], [DELETE], [HEAD], [CONNECT], [OPTIONS], [TRACE]
//!
//! ```
//! # use actix_web::HttpResponse;
//! # use actix_web_codegen::get;
//! #[get("/test")]
//! async fn get_handler() -> HttpResponse {
//!     HttpResponse::Ok().finish()
//! }
//! ```
//!
//! # Multiple Method Handlers
//! Similar to the single method handler macro but takes one or more arguments for the HTTP methods
//! it should respond to. See [macro@route] macro docs.
//!
//! ```
//! # use actix_web::HttpResponse;
//! # use actix_web_codegen::route;
//! #[route("/test", method = "GET", method = "HEAD")]
//! async fn get_and_head_handler() -> HttpResponse {
//!     HttpResponse::Ok().finish()
//! }
//! ```
//!
//! # Multiple Path Handlers
//! There are no macros to generate multi-path handlers. Let us know in [this issue].
//!
//! [this issue]: https://github.com/actix/actix-web/issues/1709
//!
//! [actix-web attributes docs]: https://docs.rs/actix-web/latest/actix_web/#attributes
//! [GET]: macro@get
//! [POST]: macro@post
//! [PUT]: macro@put
//! [HEAD]: macro@head
//! [CONNECT]: macro@macro@connect
//! [OPTIONS]: macro@options
//! [TRACE]: macro@trace
//! [PATCH]: macro@patch
//! [DELETE]: macro@delete

#![recursion_limit = "512"]
#![deny(rust_2018_idioms, nonstandard_style)]
#![warn(future_incompatible)]

use proc_macro::TokenStream;
use quote::quote;

mod route;

/// Creates resource handler, allowing multiple HTTP method guards.
///
/// # Syntax
/// ```plain
/// #[route("path", method="HTTP_METHOD"[, attributes])]
/// ```
///
/// # Attributes
/// - `"path"`: Raw literal string with path for which to register handler.
/// - `name = "resource_name"`: Specifies resource name for the handler. If not set, the function
///   name of handler is used.
/// - `method = "HTTP_METHOD"`: Registers HTTP method to provide guard for. Upper-case string,
///   "GET", "POST" for example.
/// - `guard = "function_name"`: Registers function as guard using `actix_web::guard::fn_guard`.
/// - `wrap = "Middleware"`: Registers a resource middleware.
///
/// # Notes
/// Function name can be specified as any expression that is going to be accessible to the generate
/// code, e.g `my_guard` or `my_module::my_guard`.
///
/// # Examples
/// ```
/// # use actix_web::HttpResponse;
/// # use actix_web_codegen::route;
/// #[route("/test", method = "GET", method = "HEAD")]
/// async fn example() -> HttpResponse {
///     HttpResponse::Ok().finish()
/// }
/// ```
#[proc_macro_attribute]
pub fn route(args: TokenStream, input: TokenStream) -> TokenStream {
    route::with_method(None, args, input)
}

macro_rules! method_macro {
    ($variant:ident, $method:ident) => {
#[doc = concat!("Creates route handler with `actix_web::guard::", stringify!($variant), "`.")]
///
/// # Syntax
/// ```plain
#[doc = concat!("#[", stringify!($method), r#"("path"[, attributes])]"#)]
/// ```
///
/// # Attributes
/// - `"path"`: Raw literal string with path for which to register handler.
/// - `name = "resource_name"`: Specifies resource name for the handler. If not set, the function
///   name of handler is used.
/// - `guard = "function_name"`: Registers function as guard using `actix_web::guard::fn_guard`.
/// - `wrap = "Middleware"`: Registers a resource middleware.
///
/// # Notes
/// Function name can be specified as any expression that is going to be accessible to the
/// generate code, e.g `my_guard` or `my_module::my_guard`.
///
/// # Examples
/// ```
/// # use actix_web::HttpResponse;
#[doc = concat!("# use actix_web_codegen::", stringify!($method), ";")]
#[doc = concat!("#[", stringify!($method), r#"("/")]"#)]
/// async fn example() -> HttpResponse {
///     HttpResponse::Ok().finish()
/// }
/// ```
#[proc_macro_attribute]
pub fn $method(args: TokenStream, input: TokenStream) -> TokenStream {
    route::with_method(Some(route::MethodType::$variant), args, input)
}
    };
}

method_macro!(Get, get);
method_macro!(Post, post);
method_macro!(Put, put);
method_macro!(Delete, delete);
method_macro!(Head, head);
method_macro!(Connect, connect);
method_macro!(Options, options);
method_macro!(Trace, trace);
method_macro!(Patch, patch);

/// Marks async main function as the Actix Web system entry-point.
///
/// Note that Actix Web also works under `#[tokio::main]` since version 4.0. However, this macro is
/// still necessary for actor support (since actors use a `System`). Read more in the
/// [`actix_web::rt`](https://docs.rs/actix-web/4/actix_web/rt) module docs.
///
/// # Examples
/// ```
/// #[actix_web::main]
/// async fn main() {
///     async { println!("Hello world"); }.await
/// }
/// ```
#[proc_macro_attribute]
pub fn main(_: TokenStream, item: TokenStream) -> TokenStream {
    let mut output: TokenStream = (quote! {
        #[::actix_web::rt::main(system = "::actix_web::rt::System")]
    })
    .into();

    output.extend(item);
    output
}

/// Marks async test functions to use the actix system entry-point.
///
/// # Examples
/// ```
/// #[actix_web::test]
/// async fn test() {
///     assert_eq!(async { "Hello world" }.await, "Hello world");
/// }
/// ```
#[proc_macro_attribute]
pub fn test(_: TokenStream, item: TokenStream) -> TokenStream {
    let mut output: TokenStream = (quote! {
        #[::actix_web::rt::test(system = "::actix_web::rt::System")]
    })
    .into();

    output.extend(item);
    output
}
