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
//! - [route](attr.route.html)
//!
//! ### Attributes:
//!
//! - `"path"` - *Required*, raw literal string with path for which to register handle
//! - `method="HTTP_METHOD"` - Secondary HTTP method accepted, uppercased string. "GET", "POST" for example
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
    route::with_method(None, args, input)
}

macro_rules! doc_comment {
    ($($x:expr)*; $($tt:tt)*) => {
        $(#[doc = $x])*
        $($tt)*
    };
}

macro_rules! method_macro {
    (
        $(
            ($method:ident, $variant:ident, $upper:ident);
        )+
    ) => {
        $(
            doc_comment! {
                concat!("Creates route handler with `", stringify!($upper), "` method guard.")
                concat!("")
                concat!("Syntax: `#[", stringify!($method), "(\"path\" [, attributes])]`")
                concat!("")
                concat!("## Attributes:")
                concat!("")
                concat!("- `\"path\"` - *required* Raw literal string with path for which to register handler")
                concat!("- `guard = \"function_name\"` - Register function as guard using `actix_web::guard::fn_guard`")
                concat!("- `wrap = \"Middleware\"` - Register a resource middleware.");
                #[proc_macro_attribute]
                pub fn $method(args: TokenStream, input: TokenStream) -> TokenStream {
                    route::with_method(Some(route::MethodType::$variant), args, input)
                }
            }
        )+
    };
}

method_macro! {
    (get,       Get,        GET);
    (post,      Post,       POST);
    (put,       Put,        PUT);
    (delete,    Delete,     DELETE);
    (head,      Head,       HEAD);
    (connect,   Connect,    CONNECT);
    (options,   Options,    OPTIONS);
    (trace,     Trace,      TRACE);
    (patch,     Patch,      PATCH);
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
