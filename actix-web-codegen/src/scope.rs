use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::LitStr;

use crate::{
    input_and_compile_error,
    route::{MethodType, RouteArgs},
};

pub fn with_scope(args: TokenStream, input: TokenStream) -> TokenStream {
    match with_scope_inner(args, input.clone()) {
        Ok(stream) => stream,
        Err(err) => input_and_compile_error(input, err),
    }
}

fn with_scope_inner(args: TokenStream, input: TokenStream) -> syn::Result<TokenStream> {
    if args.is_empty() {
        // macro args are missing
        return Err(syn::Error::new(
            Span::call_site(),
            "missing arguments for scope macro, \
                expected: #[scope(\"/prefix\")]",
        ));
    }

    let scope_prefix = syn::parse::<LitStr>(args.clone()).map_err(|err| {
        // first macro arg is not a string literal
        syn::Error::new(
            err.span(),
            "argument to scope macro is not a string literal, \
            expected: #[scope(\"/prefix\")]",
        )
    })?;

    let mut module = syn::parse::<syn::ItemMod>(input).map_err(|err| {
        syn::Error::new(err.span(), "#[scope] macro must be attached to a module")
    })?;

    // modify any routing macros (method or route[s]) attached to
    // functions by prefixing them with this scope macro's argument
    if let Some((_, ref mut items)) = module.content {
        for item in items {
            if let syn::Item::Fn(fun) = item {
                fun.attrs = fun
                    .attrs
                    .iter()
                    .map(|attr| modify_attribute_with_scope(attr, &scope_prefix.value()))
                    .collect();
            }
        }
    }

    Ok(TokenStream::from(quote! { #module }))
}

// Check if the attribute is a method type and has a route path, then modify it
fn modify_attribute_with_scope(attr: &syn::Attribute, scope_path: &str) -> syn::Attribute {
    match (attr.parse_args::<RouteArgs>(), attr.clone().meta) {
        (Ok(route_args), syn::Meta::List(meta_list)) if has_allowed_methods_in_scope(attr) => {
            let modified_path = format!("{}{}", scope_path, route_args.path.value());

            let options_tokens: Vec<TokenStream2> = route_args
                .options
                .iter()
                .map(|option| {
                    quote! { ,#option }
                })
                .collect();

            let combined_options_tokens: TokenStream2 =
                options_tokens
                    .into_iter()
                    .fold(TokenStream2::new(), |mut acc, ts| {
                        acc.extend(std::iter::once(ts));
                        acc
                    });

            syn::Attribute {
                meta: syn::Meta::List(syn::MetaList {
                    tokens: quote! { #modified_path #combined_options_tokens },
                    ..meta_list.clone()
                }),
                ..attr.clone()
            }
        }
        _ => attr.clone(),
    }
}

fn has_allowed_methods_in_scope(attr: &syn::Attribute) -> bool {
    MethodType::from_path(attr.path()).is_ok()
        || attr.path().is_ident("route")
        || attr.path().is_ident("ROUTE")
}
