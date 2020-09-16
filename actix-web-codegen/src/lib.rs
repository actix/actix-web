#![recursion_limit = "512"]

//! Helper and convenience macros for Actix-web.
//!
//! ## Runtime Setup
//!
//! - [main](attr.main.html)
//!
//! ## Resource Macros:
//!
//! - [get](attr.get.html)
//! - [post](attr.post.html)
//! - [put](attr.put.html)
//! - [delete](attr.delete.html)
//! - [head](attr.head.html)
//! - [connect](attr.connect.html)
//! - [options](attr.options.html)
//! - [trace](attr.trace.html)
//! - [patch](attr.patch.html)
//!
//! ### Attributes:
//!
//! - `"path"` - Raw literal string with path for which to register handle. Mandatory.
//! - `guard="function_name"` - Registers function as guard using `actix_web::guard::fn_guard`
//! - `wrap="Middleware"` - Registers a resource middleware.
//!
//! ### Notes
//!
//! Function name can be specified as any expression that is going to be accessible to the generate
//! code (e.g `my_guard` or `my_module::my_guard`)
//!
//! ### Example:
//!
//! ```rust
//! use actix_web::HttpResponse;
//! use actix_web_codegen::get;
//!
//! #[get("/test")]
//! async fn async_test() -> Result<HttpResponse, actix_web::Error> {
//!     Ok(HttpResponse::Ok().finish())
//! }
//! ```

extern crate proc_macro;

mod route;

use proc_macro::TokenStream;

/// Creates route handler with `GET` method guard.
///
/// Syntax: `#[get("path" [, attributes])]`
///
/// ## Attributes:
///
/// - `"path"` - Raw literal string with path for which to register handler. Mandatory.
/// - `guard = "function_name"` - Registers function as guard using `actix_web::guard::fn_guard`
/// - `wrap = "Middleware"` - Registers a resource middleware.
#[proc_macro_attribute]
pub fn get(args: TokenStream, input: TokenStream) -> TokenStream {
    route::generate(args, input, route::GuardType::Get)
}

/// Creates route handler with `POST` method guard.
///
/// Syntax: `#[post("path" [, attributes])]`
///
/// Attributes are the same as in [get](attr.get.html)
#[proc_macro_attribute]
pub fn post(args: TokenStream, input: TokenStream) -> TokenStream {
    route::generate(args, input, route::GuardType::Post)
}

/// Creates route handler with `PUT` method guard.
///
/// Syntax: `#[put("path" [, attributes])]`
///
/// Attributes are the same as in [get](attr.get.html)
#[proc_macro_attribute]
pub fn put(args: TokenStream, input: TokenStream) -> TokenStream {
    route::generate(args, input, route::GuardType::Put)
}

/// Creates route handler with `DELETE` method guard.
///
/// Syntax: `#[delete("path" [, attributes])]`
///
/// Attributes are the same as in [get](attr.get.html).
#[proc_macro_attribute]
pub fn delete(args: TokenStream, input: TokenStream) -> TokenStream {
    route::generate(args, input, route::GuardType::Delete)
}

/// Creates route handler with `HEAD` method guard.
///
/// Syntax: `#[head("path" [, attributes])]`
///
/// Attributes are the same as in [get](attr.get.html).
#[proc_macro_attribute]
pub fn head(args: TokenStream, input: TokenStream) -> TokenStream {
    route::generate(args, input, route::GuardType::Head)
}

/// Creates route handler with `CONNECT` method guard.
///
/// Syntax: `#[connect("path" [, attributes])]`
///
/// Attributes are the same as in [get](attr.get.html).
#[proc_macro_attribute]
pub fn connect(args: TokenStream, input: TokenStream) -> TokenStream {
    route::generate(args, input, route::GuardType::Connect)
}

/// Creates route handler with `OPTIONS` method guard.
///
/// Syntax: `#[options("path" [, attributes])]`
///
/// Attributes are the same as in [get](attr.get.html).
#[proc_macro_attribute]
pub fn options(args: TokenStream, input: TokenStream) -> TokenStream {
    route::generate(args, input, route::GuardType::Options)
}

/// Creates route handler with `TRACE` method guard.
///
/// Syntax: `#[trace("path" [, attributes])]`
///
/// Attributes are the same as in [get](attr.get.html).
#[proc_macro_attribute]
pub fn trace(args: TokenStream, input: TokenStream) -> TokenStream {
    route::generate(args, input, route::GuardType::Trace)
}

/// Creates route handler with `PATCH` method guard.
///
/// Syntax: `#[patch("path" [, attributes])]`
///
/// Attributes are the same as in [get](attr.get.html).
#[proc_macro_attribute]
pub fn patch(args: TokenStream, input: TokenStream) -> TokenStream {
    route::generate(args, input, route::GuardType::Patch)
}

/// Creates resource handler, allowing multiple HTTP method guards.
///
/// Syntax: `#[route("path"[, attributes])]`
///
/// Example: `#[route("/", method="GET", method="HEAD")]`
///
/// ## Attributes
///
/// - `"path"` - Raw literal string with path for which to register handler. Mandatory.
/// - `method="HTTP_METHOD"` - Registers HTTP method to provide guard for.
/// - `guard="function_name"` - Registers function as guard using `actix_web::guard::fn_guard`
/// - `wrap="Middleware"` - Registers a resource middleware.
#[proc_macro_attribute]
pub fn route(args: TokenStream, input: TokenStream) -> TokenStream {
    route::generate(args, input, route::GuardType::Multi)
}

/// Marks async main function as the actix system entry-point.
///
/// ## Usage
///
/// ```rust
/// #[actix_web::main]
/// async fn main() {
///     async { println!("Hello world"); }.await
/// }
/// ```
#[proc_macro_attribute]
#[cfg(not(test))] // Work around for rust-lang/rust#62127
pub fn main(_: TokenStream, item: TokenStream) -> TokenStream {
    use quote::quote;

    let mut input = syn::parse_macro_input!(item as syn::ItemFn);
    let attrs = &input.attrs;
    let vis = &input.vis;
    let sig = &mut input.sig;
    let body = &input.block;
    let name = &sig.ident;

    if sig.asyncness.is_none() {
        return syn::Error::new_spanned(sig.fn_token, "only async fn is supported")
            .to_compile_error()
            .into();
    }

    sig.asyncness = None;

    (quote! {
        #(#attrs)*
        #vis #sig {
            actix_web::rt::System::new(stringify!(#name))
                .block_on(async move { #body })
        }
    })
    .into()
}
