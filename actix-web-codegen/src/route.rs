use std::{collections::HashSet, convert::TryFrom};

use actix_router::ResourceDef;
use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote, ToTokens, TokenStreamExt};
use syn::{parse_macro_input, AttributeArgs, Ident, LitStr, NestedMeta};

enum ResourceType {
    Async,
    Sync,
}

impl ToTokens for ResourceType {
    fn to_tokens(&self, stream: &mut TokenStream2) {
        let ident = format_ident!("to");
        stream.append(ident);
    }
}

macro_rules! method_type {
    (
        $($variant:ident, $upper:ident,)+
    ) => {
        #[derive(Debug, PartialEq, Eq, Hash)]
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
                    _ => Err(format!("Unexpected HTTP method: `{}`", method)),
                }
            }
        }
    };
}

method_type! {
    Get,       GET,
    Post,      POST,
    Put,       PUT,
    Delete,    DELETE,
    Head,      HEAD,
    Connect,   CONNECT,
    Options,   OPTIONS,
    Trace,     TRACE,
    Patch,     PATCH,
}

impl ToTokens for MethodType {
    fn to_tokens(&self, stream: &mut TokenStream2) {
        let ident = Ident::new(self.as_str(), Span::call_site());
        stream.append(ident);
    }
}

impl TryFrom<&syn::LitStr> for MethodType {
    type Error = syn::Error;

    fn try_from(value: &syn::LitStr) -> Result<Self, Self::Error> {
        Self::parse(value.value().as_str())
            .map_err(|message| syn::Error::new_spanned(value, message))
    }
}

struct Args {
    path: syn::LitStr,
    resource_name: Option<syn::LitStr>,
    guards: Vec<Ident>,
    wrappers: Vec<syn::Type>,
    methods: HashSet<MethodType>,
}

impl Args {
    fn new(args: AttributeArgs, method: Option<MethodType>) -> syn::Result<Self> {
        let mut path = None;
        let mut resource_name = None;
        let mut guards = Vec::new();
        let mut wrappers = Vec::new();
        let mut methods = HashSet::new();

        let is_route_macro = method.is_none();
        if let Some(method) = method {
            methods.insert(method);
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
                            guards.push(Ident::new(&lit.value(), Span::call_site()));
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
                            let method = MethodType::try_from(lit)?;
                            if !methods.insert(method) {
                                return Err(syn::Error::new_spanned(
                                    &nv.lit,
                                    &format!(
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
    name: syn::Ident,
    args: Args,
    ast: syn::ItemFn,
    resource_type: ResourceType,

    /// The doc comment attributes to copy to generated struct, if any.
    doc_attributes: Vec<syn::Attribute>,
}

fn guess_resource_type(typ: &syn::Type) -> ResourceType {
    let mut guess = ResourceType::Sync;

    if let syn::Type::ImplTrait(typ) = typ {
        for bound in typ.bounds.iter() {
            if let syn::TypeParamBound::Trait(bound) = bound {
                for bound in bound.path.segments.iter() {
                    if bound.ident == "Future" {
                        guess = ResourceType::Async;
                        break;
                    } else if bound.ident == "Responder" {
                        guess = ResourceType::Sync;
                        break;
                    }
                }
            }
        }
    }

    guess
}

impl Route {
    pub fn new(
        args: AttributeArgs,
        ast: syn::ItemFn,
        method: Option<MethodType>,
    ) -> syn::Result<Self> {
        if args.is_empty() {
            return Err(syn::Error::new(
                Span::call_site(),
                format!(
                    r#"invalid service definition, expected #[{}("<some path>")]"#,
                    method
                        .map_or("route", |it| it.as_str())
                        .to_ascii_lowercase()
                ),
            ));
        }

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

        let resource_type = if ast.sig.asyncness.is_some() {
            ResourceType::Async
        } else {
            match ast.sig.output {
                syn::ReturnType::Default => {
                    return Err(syn::Error::new_spanned(
                        ast,
                        "Function has no return type. Cannot be used as handler",
                    ));
                }
                syn::ReturnType::Type(_, ref typ) => guess_resource_type(typ.as_ref()),
            }
        };

        Ok(Self {
            name,
            args,
            ast,
            resource_type,
            doc_attributes,
        })
    }
}

impl ToTokens for Route {
    fn to_tokens(&self, output: &mut TokenStream2) {
        let Self {
            name,
            ast,
            args:
                Args {
                    path,
                    resource_name,
                    guards,
                    wrappers,
                    methods,
                },
            resource_type,
            doc_attributes,
        } = self;
        let resource_name = resource_name
            .as_ref()
            .map_or_else(|| name.to_string(), LitStr::value);
        let method_guards = {
            let mut others = methods.iter();
            // unwrapping since length is checked to be at least one
            let first = others.next().unwrap();

            if methods.len() > 1 {
                quote! {
                    .guard(
                        ::actix_web::guard::Any(::actix_web::guard::#first())
                            #(.or(::actix_web::guard::#others()))*
                    )
                }
            } else {
                quote! {
                    .guard(::actix_web::guard::#first())
                }
            }
        };

        let stream = quote! {
            #(#doc_attributes)*
            #[allow(non_camel_case_types, missing_docs)]
            pub struct #name;

            impl ::actix_web::dev::HttpServiceFactory for #name {
                fn register(self, __config: &mut actix_web::dev::AppService) {
                    #ast
                    let __resource = ::actix_web::Resource::new(#path)
                        .name(#resource_name)
                        #method_guards
                        #(.guard(::actix_web::guard::fn_guard(#guards)))*
                        #(.wrap(#wrappers))*
                        .#resource_type(#name);

                    ::actix_web::dev::HttpServiceFactory::register(__resource, __config)
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
