use std::{collections::HashSet, convert::TryFrom};

use actix_router::ResourceDef;
use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{quote, ToTokens, TokenStreamExt};
use syn::{parse_macro_input, AttributeArgs, Ident, LitStr, Meta, NestedMeta, Path};

macro_rules! standard_method_type {
    (
        $($variant:ident, $upper:ident, $lower:ident,)+
    ) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash)]
        pub enum MethodType {
            $(
                $variant,
            )+
        }

        impl MethodType {
            fn as_str(&self) -> &'static str {
                match self {
                    $(Self::$variant => stringify!($variant),)+
                }
            }

            fn parse(method: &str) -> Result<Self, String> {
                match method {
                    $(stringify!($upper) => Ok(Self::$variant),)+
                    _ => Err(format!("HTTP method must be uppercase: `{}`", method)),
                }
            }

            fn from_path(method: &Path) -> Result<Self, ()> {
                match () {
                    $(_ if method.is_ident(stringify!($lower)) => Ok(Self::$variant),)+
                    _ => Err(()),
                }
            }
        }
    };
}

standard_method_type! {
    Get,       GET,     get,
    Post,      POST,    post,
    Put,       PUT,     put,
    Delete,    DELETE,  delete,
    Head,      HEAD,    head,
    Connect,   CONNECT, connect,
    Options,   OPTIONS, options,
    Trace,     TRACE,   trace,
    Patch,     PATCH,   patch,
}

impl TryFrom<&syn::LitStr> for MethodType {
    type Error = syn::Error;

    fn try_from(value: &syn::LitStr) -> Result<Self, Self::Error> {
        Self::parse(value.value().as_str())
            .map_err(|message| syn::Error::new_spanned(value, message))
    }
}

impl ToTokens for MethodType {
    fn to_tokens(&self, stream: &mut TokenStream2) {
        let ident = Ident::new(self.as_str(), Span::call_site());
        stream.append(ident);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum MethodTypeExt {
    Standard(MethodType),
    Custom(LitStr),
}

impl MethodTypeExt {
    /// Returns a single method guard token stream.
    fn to_tokens_single_guard(&self) -> TokenStream2 {
        match self {
            MethodTypeExt::Standard(method) => {
                quote! {
                    .guard(::actix_web::guard::#method())
                }
            }
            MethodTypeExt::Custom(lit) => {
                quote! {
                    .guard(::actix_web::guard::Method(
                        ::actix_web::http::Method::from_bytes(#lit.as_bytes()).unwrap()
                    ))
                }
            }
        }
    }

    /// Returns a multi-method guard chain token stream.
    fn to_tokens_multi_guard(&self, or_chain: Vec<impl ToTokens>) -> TokenStream2 {
        debug_assert!(
            !or_chain.is_empty(),
            "empty or_chain passed to multi-guard constructor"
        );

        match self {
            MethodTypeExt::Standard(method) => {
                quote! {
                    .guard(
                        ::actix_web::guard::Any(::actix_web::guard::#method())
                            #(#or_chain)*
                    )
                }
            }
            MethodTypeExt::Custom(lit) => {
                quote! {
                    .guard(
                        ::actix_web::guard::Any(
                            ::actix_web::guard::Method(
                                ::actix_web::http::Method::from_bytes(#lit.as_bytes()).unwrap()
                            )
                        )
                        #(#or_chain)*
                    )
                }
            }
        }
    }

    /// Returns a token stream containing the `.or` chain to be passed in to
    /// [`MethodTypeExt::to_tokens_multi_guard()`].
    fn to_tokens_multi_guard_or_chain(&self) -> TokenStream2 {
        match self {
            MethodTypeExt::Standard(method_type) => {
                quote! {
                    .or(::actix_web::guard::#method_type())
                }
            }
            MethodTypeExt::Custom(lit) => {
                quote! {
                    .or(
                        ::actix_web::guard::Method(
                            ::actix_web::http::Method::from_bytes(#lit.as_bytes()).unwrap()
                        )
                    )
                }
            }
        }
    }
}

impl ToTokens for MethodTypeExt {
    fn to_tokens(&self, stream: &mut TokenStream2) {
        match self {
            MethodTypeExt::Custom(lit_str) => {
                let ident = Ident::new(lit_str.value().as_str(), Span::call_site());
                stream.append(ident);
            }
            MethodTypeExt::Standard(method) => method.to_tokens(stream),
        }
    }
}

impl TryFrom<&syn::LitStr> for MethodTypeExt {
    type Error = syn::Error;

    fn try_from(value: &syn::LitStr) -> Result<Self, Self::Error> {
        match MethodType::try_from(value) {
            Ok(method) => Ok(MethodTypeExt::Standard(method)),
            Err(_) if value.value().chars().all(|c| c.is_ascii_uppercase()) => {
                Ok(MethodTypeExt::Custom(value.clone()))
            }
            Err(err) => Err(err),
        }
    }
}

struct Args {
    path: syn::LitStr,
    resource_name: Option<syn::LitStr>,
    guards: Vec<Path>,
    wrappers: Vec<syn::Type>,
    methods: HashSet<MethodTypeExt>,
}

impl Args {
    fn new(args: AttributeArgs, method: Option<MethodType>) -> syn::Result<Self> {
        let mut path = None;
        let mut resource_name = None;
        let mut guards = Vec::new();
        let mut wrappers = Vec::new();
        let mut methods = HashSet::new();

        if args.is_empty() {
            return Err(syn::Error::new(
                Span::call_site(),
                format!(
                    r#"invalid service definition, expected #[{}("<path>")]"#,
                    method
                        .map_or("route", |it| it.as_str())
                        .to_ascii_lowercase()
                ),
            ));
        }

        let is_route_macro = method.is_none();
        if let Some(method) = method {
            methods.insert(MethodTypeExt::Standard(method));
        }

        for arg in args {
            match arg {
                NestedMeta::Lit(syn::Lit::Str(lit)) => match path {
                    None => {
                        let _ = ResourceDef::new(lit.value());
                        path = Some(lit);
                    }
                    _ => {
                        return Err(syn::Error::new_spanned(
                            lit,
                            "Multiple paths specified! Should be only one!",
                        ));
                    }
                },

                NestedMeta::Meta(syn::Meta::NameValue(nv)) => {
                    if nv.path.is_ident("name") {
                        if let syn::Lit::Str(lit) = nv.lit {
                            resource_name = Some(lit);
                        } else {
                            return Err(syn::Error::new_spanned(
                                nv.lit,
                                "Attribute name expects literal string!",
                            ));
                        }
                    } else if nv.path.is_ident("guard") {
                        if let syn::Lit::Str(lit) = nv.lit {
                            guards.push(lit.parse::<Path>()?);
                        } else {
                            return Err(syn::Error::new_spanned(
                                nv.lit,
                                "Attribute guard expects literal string!",
                            ));
                        }
                    } else if nv.path.is_ident("wrap") {
                        if let syn::Lit::Str(lit) = nv.lit {
                            wrappers.push(lit.parse()?);
                        } else {
                            return Err(syn::Error::new_spanned(
                                nv.lit,
                                "Attribute wrap expects type",
                            ));
                        }
                    } else if nv.path.is_ident("method") {
                        if !is_route_macro {
                            return Err(syn::Error::new_spanned(
                                &nv,
                                "HTTP method forbidden here. To handle multiple methods, use `route` instead",
                            ));
                        } else if let syn::Lit::Str(ref lit) = nv.lit {
                            if !methods.insert(MethodTypeExt::try_from(lit)?) {
                                return Err(syn::Error::new_spanned(
                                    &nv.lit,
                                    format!(
                                        "HTTP method defined more than once: `{}`",
                                        lit.value()
                                    ),
                                ));
                            }
                        } else {
                            return Err(syn::Error::new_spanned(
                                nv.lit,
                                "Attribute method expects literal string!",
                            ));
                        }
                    } else {
                        return Err(syn::Error::new_spanned(
                            nv.path,
                            "Unknown attribute key is specified. Allowed: guard, method and wrap",
                        ));
                    }
                }

                arg => {
                    return Err(syn::Error::new_spanned(arg, "Unknown attribute."));
                }
            }
        }

        Ok(Args {
            path: path.unwrap(),
            resource_name,
            guards,
            wrappers,
            methods,
        })
    }
}

pub struct Route {
    /// Name of the handler function being annotated.
    name: syn::Ident,

    /// Args passed to routing macro.
    ///
    /// When using `#[routes]`, this will contain args for each specific routing macro.
    args: Vec<Args>,

    /// AST of the handler function being annotated.
    ast: syn::ItemFn,

    /// The doc comment attributes to copy to generated struct, if any.
    doc_attributes: Vec<syn::Attribute>,
}

impl Route {
    pub fn new(
        args: AttributeArgs,
        ast: syn::ItemFn,
        method: Option<MethodType>,
    ) -> syn::Result<Self> {
        let name = ast.sig.ident.clone();

        // Try and pull out the doc comments so that we can reapply them to the generated struct.
        // Note that multi line doc comments are converted to multiple doc attributes.
        let doc_attributes = ast
            .attrs
            .iter()
            .filter(|attr| attr.path.is_ident("doc"))
            .cloned()
            .collect();

        let args = Args::new(args, method)?;

        if args.methods.is_empty() {
            return Err(syn::Error::new(
                Span::call_site(),
                "The #[route(..)] macro requires at least one `method` attribute",
            ));
        }

        if matches!(ast.sig.output, syn::ReturnType::Default) {
            return Err(syn::Error::new_spanned(
                ast,
                "Function has no return type. Cannot be used as handler",
            ));
        }

        Ok(Self {
            name,
            args: vec![args],
            ast,
            doc_attributes,
        })
    }

    fn multiple(args: Vec<Args>, ast: syn::ItemFn) -> syn::Result<Self> {
        let name = ast.sig.ident.clone();

        // Try and pull out the doc comments so that we can reapply them to the generated struct.
        // Note that multi line doc comments are converted to multiple doc attributes.
        let doc_attributes = ast
            .attrs
            .iter()
            .filter(|attr| attr.path.is_ident("doc"))
            .cloned()
            .collect();

        if matches!(ast.sig.output, syn::ReturnType::Default) {
            return Err(syn::Error::new_spanned(
                ast,
                "Function has no return type. Cannot be used as handler",
            ));
        }

        Ok(Self {
            name,
            args,
            ast,
            doc_attributes,
        })
    }
}

impl ToTokens for Route {
    fn to_tokens(&self, output: &mut TokenStream2) {
        let Self {
            name,
            ast,
            args,
            doc_attributes,
        } = self;

        let registrations: TokenStream2 = args
            .iter()
            .map(|args| {
                let Args {
                    path,
                    resource_name,
                    guards,
                    wrappers,
                    methods,
                } = args;

                let resource_name = resource_name
                    .as_ref()
                    .map_or_else(|| name.to_string(), LitStr::value);

                let method_guards = {
                    debug_assert!(!methods.is_empty(), "Args::methods should not be empty");

                    let mut others = methods.iter();
                    let first = others.next().unwrap();

                    if methods.len() > 1 {
                        let other_method_guards = others
                            .map(|method_ext| method_ext.to_tokens_multi_guard_or_chain())
                            .collect();

                        first.to_tokens_multi_guard(other_method_guards)
                    } else {
                        first.to_tokens_single_guard()
                    }
                };

                quote! {
                    let __resource = ::actix_web::Resource::new(#path)
                        .name(#resource_name)
                        #method_guards
                        #(.guard(::actix_web::guard::fn_guard(#guards)))*
                        #(.wrap(#wrappers))*
                        .to(#name);
                    ::actix_web::dev::HttpServiceFactory::register(__resource, __config);
                }
            })
            .collect();

        let stream = quote! {
            #(#doc_attributes)*
            #[allow(non_camel_case_types, missing_docs)]
            pub struct #name;

            impl ::actix_web::dev::HttpServiceFactory for #name {
                fn register(self, __config: &mut actix_web::dev::AppService) {
                    #ast
                    #registrations
                }
            }
        };

        output.extend(stream);
    }
}

pub(crate) fn with_method(
    method: Option<MethodType>,
    args: TokenStream,
    input: TokenStream,
) -> TokenStream {
    let args = parse_macro_input!(args as syn::AttributeArgs);

    let ast = match syn::parse::<syn::ItemFn>(input.clone()) {
        Ok(ast) => ast,
        // on parse error, make IDEs happy; see fn docs
        Err(err) => return input_and_compile_error(input, err),
    };

    match Route::new(args, ast, method) {
        Ok(route) => route.into_token_stream().into(),
        // on macro related error, make IDEs happy; see fn docs
        Err(err) => input_and_compile_error(input, err),
    }
}

pub(crate) fn with_methods(input: TokenStream) -> TokenStream {
    let mut ast = match syn::parse::<syn::ItemFn>(input.clone()) {
        Ok(ast) => ast,
        // on parse error, make IDEs happy; see fn docs
        Err(err) => return input_and_compile_error(input, err),
    };

    let (methods, others) = ast
        .attrs
        .into_iter()
        .map(|attr| match MethodType::from_path(&attr.path) {
            Ok(method) => Ok((method, attr)),
            Err(_) => Err(attr),
        })
        .partition::<Vec<_>, _>(Result::is_ok);

    ast.attrs = others.into_iter().map(Result::unwrap_err).collect();

    let methods =
        match methods
            .into_iter()
            .map(Result::unwrap)
            .map(|(method, attr)| {
                attr.parse_meta().and_then(|args| {
                    if let Meta::List(args) = args {
                        Args::new(args.nested.into_iter().collect(), Some(method))
                    } else {
                        Err(syn::Error::new_spanned(attr, "Invalid input for macro"))
                    }
                })
            })
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(methods) if methods.is_empty() => return input_and_compile_error(
                input,
                syn::Error::new(
                    Span::call_site(),
                    "The #[routes] macro requires at least one `#[<method>(..)]` attribute.",
                ),
            ),
            Ok(methods) => methods,
            Err(err) => return input_and_compile_error(input, err),
        };

    match Route::multiple(methods, ast) {
        Ok(route) => route.into_token_stream().into(),
        // on macro related error, make IDEs happy; see fn docs
        Err(err) => input_and_compile_error(input, err),
    }
}

/// Converts the error to a token stream and appends it to the original input.
///
/// Returning the original input in addition to the error is good for IDEs which can gracefully
/// recover and show more precise errors within the macro body.
///
/// See <https://github.com/rust-analyzer/rust-analyzer/issues/10468> for more info.
fn input_and_compile_error(mut item: TokenStream, err: syn::Error) -> TokenStream {
    let compile_err = TokenStream::from(err.to_compile_error());
    item.extend(compile_err);
    item
}
