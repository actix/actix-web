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

#[derive(Debug, PartialEq, Eq, Hash)]
pub enum GuardType {
    Get,
    Post,
    Put,
    Delete,
    Head,
    Connect,
    Options,
    Trace,
    Patch,
    Multi,
}

impl GuardType {
    fn as_str(&self) -> &'static str {
        match self {
            GuardType::Get => "Get",
            GuardType::Post => "Post",
            GuardType::Put => "Put",
            GuardType::Delete => "Delete",
            GuardType::Head => "Head",
            GuardType::Connect => "Connect",
            GuardType::Options => "Options",
            GuardType::Trace => "Trace",
            GuardType::Patch => "Patch",
            GuardType::Multi => "Multi",
        }
    }
}

impl ToTokens for GuardType {
    fn to_tokens(&self, stream: &mut TokenStream2) {
        let ident = Ident::new(self.as_str(), Span::call_site());
        stream.append(ident);
    }
}

impl TryFrom<&syn::LitStr> for GuardType {
    type Error = syn::Error;

    fn try_from(value: &syn::LitStr) -> Result<Self, Self::Error> {
        match value.value().as_str() {
            "CONNECT" => Ok(GuardType::Connect),
            "DELETE" => Ok(GuardType::Delete),
            "GET" => Ok(GuardType::Get),
            "HEAD" => Ok(GuardType::Head),
            "OPTIONS" => Ok(GuardType::Options),
            "PATCH" => Ok(GuardType::Patch),
            "POST" => Ok(GuardType::Post),
            "PUT" => Ok(GuardType::Put),
            "TRACE" => Ok(GuardType::Trace),
            _ => Err(syn::Error::new_spanned(
                value,
                &format!("Unexpected HTTP Method: `{}`", value.value()),
            )),
        }
    }
}

struct Args {
    path: syn::LitStr,
    guards: Vec<Ident>,
    wrappers: Vec<syn::Type>,
    methods: HashSet<GuardType>,
}

impl Args {
    fn new(args: AttributeArgs) -> syn::Result<Self> {
        let mut path = None;
        let mut guards = Vec::new();
        let mut wrappers = Vec::new();
        let mut methods = HashSet::new();
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
                        if let syn::Lit::Str(ref lit) = nv.lit {
                            let guard = GuardType::try_from(lit)?;
                            if !methods.insert(guard) {
                                return Err(syn::Error::new_spanned(
                                    &nv.lit,
                                    &format!(
                                        "HTTP Method defined more than once: `{}`",
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
    guard: GuardType,
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
        guard: GuardType,
    ) -> syn::Result<Self> {
        if args.is_empty() {
            return Err(syn::Error::new(
                Span::call_site(),
                format!(
                    r#"invalid server definition, expected #[{}("<some path>")]"#,
                    guard.as_str().to_ascii_lowercase()
                ),
            ));
        }
        let ast: syn::ItemFn = syn::parse(input)?;
        let name = ast.sig.ident.clone();

        let args = Args::new(args)?;

        if guard == GuardType::Multi && args.methods.is_empty() {
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
            guard,
        })
    }
}

impl ToTokens for Route {
    fn to_tokens(&self, output: &mut TokenStream2) {
        let Self {
            name,
            guard,
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
        let mut methods = methods.iter();

        let method_guards = if *guard == GuardType::Multi {
            // unwrapping since length is checked to be at least one
            let first = methods.next().unwrap();

            quote! {
                .guard(
                    actix_web::guard::Any(actix_web::guard::#first())
                        #(.or(actix_web::guard::#methods()))*
                )
            }
        } else {
            quote! {
                .guard(actix_web::guard::#guard())
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

pub(crate) fn generate(
    args: TokenStream,
    input: TokenStream,
    guard: GuardType,
) -> TokenStream {
    let args = parse_macro_input!(args as syn::AttributeArgs);
    match Route::new(args, input, guard) {
        Ok(route) => route.into_token_stream().into(),
        Err(err) => err.to_compile_error().into(),
    }
}
