extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::{quote, ToTokens, TokenStreamExt};
use syn::{AttributeArgs, Ident, NestedMeta};

enum ResourceType {
    Async,
    Sync,
}

impl ToTokens for ResourceType {
    fn to_tokens(&self, stream: &mut TokenStream2) {
        let ident = match self {
            ResourceType::Async => "to",
            ResourceType::Sync => "to",
        };
        let ident = Ident::new(ident, Span::call_site());
        stream.append(ident);
    }
}

#[derive(PartialEq)]
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
        }
    }
}

impl ToTokens for GuardType {
    fn to_tokens(&self, stream: &mut TokenStream2) {
        let ident = self.as_str();
        let ident = Ident::new(ident, Span::call_site());
        stream.append(ident);
    }
}

struct Args {
    path: syn::LitStr,
    guards: Vec<Ident>,
}

impl Args {
    fn new(args: AttributeArgs) -> syn::Result<Self> {
        let mut path = None;
        let mut guards = Vec::new();
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
                    } else {
                        return Err(syn::Error::new_spanned(
                            nv.path,
                            "Unknown attribute key is specified. Allowed: guard",
                        ));
                    }
                }
                arg => {
                    return Err(syn::Error::new_spanned(arg, "Unknown attribute"));
                }
            }
        }
        Ok(Args {
            path: path.unwrap(),
            guards,
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

    pub fn generate(&self) -> TokenStream {
        let name = &self.name;
        let resource_name = name.to_string();
        let guard = &self.guard;
        let ast = &self.ast;
        let path = &self.args.path;
        let extra_guards = &self.args.guards;
        let resource_type = &self.resource_type;
        let stream = quote! {
            #[allow(non_camel_case_types)]
            pub struct #name;

            impl actix_web::dev::HttpServiceFactory for #name {
                fn register(self, config: &mut actix_web::dev::AppService) {
                    #ast
                    let resource = actix_web::Resource::new(#path)
                        .name(#resource_name)
                        .guard(actix_web::guard::#guard())
                        #(.guard(actix_web::guard::fn_guard(#extra_guards)))*
                        .#resource_type(#name);

                    actix_web::dev::HttpServiceFactory::register(resource, config)
                }
            }
        };
        stream.into()
    }
}
