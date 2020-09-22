#![recursion_limit = "512"]

extern crate proc_macro;

use proc_macro::TokenStream;

mod route;

/// Creates resource handler, allowing multiple HTTP method guards.
///
/// ## Syntax
/// ```text
/// #[route("path", method="HTTP_METHOD"[, attributes])]
/// ```
///
/// ### Attributes
/// - `"path"` - Raw literal string with path for which to register handler.
/// - `method="HTTP_METHOD"` - Registers HTTP method to provide guard for. Upper-case string, "GET", "POST" for example.
/// - `guard="function_name"` - Registers function as guard using `actix_web::guard::fn_guard`
/// - `wrap="Middleware"` - Registers a resource middleware.
///
/// ### Notes
/// Function name can be specified as any expression that is going to be accessible to the generate
/// code, e.g `my_guard` or `my_module::my_guard`.
///
/// ## Example
///
/// ```rust
/// use actix_web::HttpResponse;
/// use actix_web_codegen::route;
///
/// #[route("/", method="GET", method="HEAD")]
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

## Syntax
```text
#[", stringify!($method), r#"("path"[, attributes])]
```

### Attributes
- `"path"` - Raw literal string with path for which to register handler.
- `guard="function_name"` - Registers function as guard using `actix_web::guard::fn_guard`.
- `wrap="Middleware"` - Registers a resource middleware.

### Notes
Function name can be specified as any expression that is going to be accessible to the generate
code, e.g `my_guard` or `my_module::my_guard`.

## Example

```rust
use actix_web::HttpResponse;
use actix_web_codegen::"#, stringify!($method), ";

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
