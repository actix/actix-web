extern crate proc_macro;

use std::collections::HashSet;
use std::convert::TryFrom;

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{format_ident, quote, ToTokens, TokenStreamExt};
use syn::{parse_macro_input, AttributeArgs, Ident, NestedMeta};

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
    guards: Vec<Ident>,
    wrappers: Vec<syn::Type>,
    methods: HashSet<MethodType>,
}

impl Args {
    fn new(args: AttributeArgs, method: Option<MethodType>) -> syn::Result<Self> {
        let mut path = None;
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
                    if nv.path.is_ident("guard") {
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
        input: TokenStream,
        method: Option<MethodType>,
    ) -> syn::Result<Self> {
        if args.is_empty() {
            return Err(syn::Error::new(
                Span::call_site(),
                format!(
                    r#"invalid service definition, expected #[{}("<some path>")]"#,
                    method
                        .map(|it| it.as_str())
                        .unwrap_or("route")
                        .to_ascii_lowercase()
                ),
            ));
        }
        let ast: syn::ItemFn = syn::parse(input)?;
        let name = ast.sig.ident.clone();

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
                    guards,
                    wrappers,
                    methods,
                },
            resource_type,
        } = self;
        let resource_name = name.to_string();
        let method_guards = {
            let mut others = methods.iter();
            // unwrapping since length is checked to be at least one
            let first = others.next().unwrap();

            if methods.len() > 1 {
                quote! {
                    .guard(
                        actix_web::guard::Any(actix_web::guard::#first())
                            #(.or(actix_web::guard::#others()))*
                    )
                }
            } else {
                quote! {
                    .guard(actix_web::guard::#first())
                }
            }
        };

        let stream = quote! {
            #[allow(non_camel_case_types, missing_docs)]
            pub struct #name;

            impl actix_web::dev::HttpServiceFactory for #name {
                fn register(self, __config: &mut actix_web::dev::AppService) {
                    #ast
                    let __resource = actix_web::Resource::new(#path)
                        .name(#resource_name)
                        #method_guards
                        #(.guard(actix_web::guard::fn_guard(#guards)))*
                        #(.wrap(#wrappers))*
                        .#resource_type(#name);

                    actix_web::dev::HttpServiceFactory::register(__resource, __config)
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
    match Route::new(args, input, method) {
        Ok(route) => route.into_token_stream().into(),
        Err(err) => err.to_compile_error().into(),
    }
}
