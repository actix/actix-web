#![recursion_limit = "512"]

extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::parse_macro_input;

/// #[get("path")] attribute
#[proc_macro_attribute]
pub fn get(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as syn::AttributeArgs);
    if args.is_empty() {
        panic!("invalid server definition, expected: #[get(\"some path\")]");
    }

    // path
    let path = match args[0] {
        syn::NestedMeta::Literal(syn::Lit::Str(ref fname)) => {
            let fname = quote!(#fname).to_string();
            fname.as_str()[1..fname.len() - 1].to_owned()
        }
        _ => panic!("resource path"),
    };

    let ast: syn::ItemFn = syn::parse(input).unwrap();
    let name = ast.ident.clone();

    (quote! {
        #[allow(non_camel_case_types)]
        struct #name;

        impl<P: 'static> actix_web::dev::HttpServiceFactory<P> for #name {
            fn register(self, config: &mut actix_web::dev::AppConfig<P>) {
                #ast
                actix_web::dev::HttpServiceFactory::register(
                    actix_web::Resource::new(#path)
                        .guard(actix_web::guard::Get())
                        .to(#name), config);
            }
        }
    })
    .into()
}

/// #[post("path")] attribute
#[proc_macro_attribute]
pub fn post(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as syn::AttributeArgs);
    if args.is_empty() {
        panic!("invalid server definition, expected: #[get(\"some path\")]");
    }

    // path
    let path = match args[0] {
        syn::NestedMeta::Literal(syn::Lit::Str(ref fname)) => {
            let fname = quote!(#fname).to_string();
            fname.as_str()[1..fname.len() - 1].to_owned()
        }
        _ => panic!("resource path"),
    };

    let ast: syn::ItemFn = syn::parse(input).unwrap();
    let name = ast.ident.clone();

    (quote! {
        #[allow(non_camel_case_types)]
        struct #name;

        impl<P: 'static> actix_web::dev::HttpServiceFactory<P> for #name {
            fn register(self, config: &mut actix_web::dev::AppConfig<P>) {
                #ast
                actix_web::dev::HttpServiceFactory::register(
                    actix_web::Resource::new(#path)
                        .guard(actix_web::guard::Post())
                        .to(#name), config);
            }
        }
    })
    .into()
}

/// #[put("path")] attribute
#[proc_macro_attribute]
pub fn put(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as syn::AttributeArgs);
    if args.is_empty() {
        panic!("invalid server definition, expected: #[get(\"some path\")]");
    }

    // path
    let path = match args[0] {
        syn::NestedMeta::Literal(syn::Lit::Str(ref fname)) => {
            let fname = quote!(#fname).to_string();
            fname.as_str()[1..fname.len() - 1].to_owned()
        }
        _ => panic!("resource path"),
    };

    let ast: syn::ItemFn = syn::parse(input).unwrap();
    let name = ast.ident.clone();

    (quote! {
        #[allow(non_camel_case_types)]
        struct #name;

        impl<P: 'static> actix_web::dev::HttpServiceFactory<P> for #name {
            fn register(self, config: &mut actix_web::dev::AppConfig<P>) {
                #ast
                actix_web::dev::HttpServiceFactory::register(
                    actix_web::Resource::new(#path)
                        .guard(actix_web::guard::Put())
                        .to(#name), config);
            }
        }
    })
    .into()
}
