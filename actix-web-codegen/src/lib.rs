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
//! #[route("/test", method="GET", method="HEAD")]
//! async fn get_and_head_handler() -> HttpResponse {
//!     HttpResponse::Ok().finish()
//! }
//! ```
//!
//! [actix-web attributes docs]: https://docs.rs/actix-web/*/actix_web/#attributes
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

use proc_macro::TokenStream;

mod route;

/// Creates resource handler, allowing multiple HTTP method guards.
///
/// # Syntax
/// ```text
/// #[route("path", method="HTTP_METHOD"[, attributes])]
/// ```
///
/// # Attributes
/// - `"path"` - Raw literal string with path for which to register handler.
/// - `name="resource_name"` - Specifies resource name for the handler. If not set, the function name of handler is used.
/// - `method="HTTP_METHOD"` - Registers HTTP method to provide guard for. Upper-case string, "GET", "POST" for example.
/// - `guard="function_name"` - Registers function as guard using `actix_web::guard::fn_guard`
/// - `wrap="Middleware"` - Registers a resource middleware.
///
/// # Notes
/// Function name can be specified as any expression that is going to be accessible to the generate
/// code, e.g `my_guard` or `my_module::my_guard`.
///
/// # Example
///
/// ```
/// # use actix_web::HttpResponse;
/// # use actix_web_codegen::route;
/// #[route("/test", method="GET", method="HEAD")]
/// async fn example() -> HttpResponse {
///     HttpResponse::Ok().finish()
/// }
/// ```
#[proc_macro_attribute]
pub fn route(args: TokenStream, input: TokenStream) -> TokenStream {
    route::with_method(None, args, input)
}

macro_rules! doc_comment {
    ($x:expr; $($tt:tt)*) => {
        #[doc = $x]
        $($tt)*
    };
}

macro_rules! method_macro {
    (
        $($variant:ident, $method:ident,)+
    ) => {
        $(doc_comment! {
concat!("
Creates route handler with `actix_web::guard::", stringify!($variant), "`.

# Syntax
```text
#[", stringify!($method), r#"("path"[, attributes])]
```

# Attributes
- `"path"` - Raw literal string with path for which to register handler.
- `name="resource_name"` - Specifies resource name for the handler. If not set, the function name of handler is used.
- `guard="function_name"` - Registers function as guard using `actix_web::guard::fn_guard`.
- `wrap="Middleware"` - Registers a resource middleware.

# Notes
Function name can be specified as any expression that is going to be accessible to the generate
code, e.g `my_guard` or `my_module::my_guard`.

# Example

```
# use actix_web::HttpResponse;
# use actix_web_codegen::"#, stringify!($method), ";
#[", stringify!($method), r#"("/")]
async fn example() -> HttpResponse {
    HttpResponse::Ok().finish()
}
```
"#);
            #[proc_macro_attribute]
            pub fn $method(args: TokenStream, input: TokenStream) -> TokenStream {
                route::with_method(Some(route::MethodType::$variant), args, input)
            }
        })+
    };
}

method_macro! {
    Get,       get,
    Post,      post,
    Put,       put,
    Delete,    delete,
    Head,      head,
    Connect,   connect,
    Options,   options,
    Trace,     trace,
    Patch,     patch,
}

/// Marks async main function as the actix system entry-point.
///
/// # Actix Web Re-export
/// This macro can be applied with `#[actix_web::main]` when used in Actix Web applications.
///
/// # Examples
/// ```
/// #[actix_web_codegen::main]
/// async fn main() {
///     async { println!("Hello world"); }.await
/// }
/// ```
#[proc_macro_attribute]
pub fn main(_: TokenStream, item: TokenStream) -> TokenStream {
    use quote::quote;
    let input = syn::parse_macro_input!(item as syn::ItemFn);
    (quote! {
        #[actix_web::rt::main(system = "::actix_web::rt::System")]
        #input
    })
    .into()
}
