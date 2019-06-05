extern crate proc_macro;

use std::fmt;

use proc_macro::TokenStream;
use quote::quote;

enum ResourceType {
    Async,
    Sync,
}

impl fmt::Display for ResourceType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &ResourceType::Async => write!(f, "to_async"),
            &ResourceType::Sync => write!(f, "to"),
        }
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

impl fmt::Display for GuardType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &GuardType::Get => write!(f, "Get"),
            &GuardType::Post => write!(f, "Post"),
            &GuardType::Put => write!(f, "Put"),
            &GuardType::Delete => write!(f, "Delete"),
            &GuardType::Head => write!(f, "Head"),
            &GuardType::Connect => write!(f, "Connect"),
            &GuardType::Options => write!(f, "Options"),
            &GuardType::Trace => write!(f, "Trace"),
            &GuardType::Patch => write!(f, "Patch"),
        }
    }
}

pub struct Args {
    name: syn::Ident,
    path: String,
    ast: syn::ItemFn,
    resource_type: ResourceType,
    pub guard: GuardType,
    pub extra_guards: Vec<String>,
}

impl fmt::Display for Args {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let ast = &self.ast;
        let guards = format!(".guard(actix_web::guard::{}())", self.guard);
        let guards = self.extra_guards.iter().fold(guards, |acc, val| {
            format!("{}.guard(actix_web::guard::fn_guard({}))", acc, val)
        });

        write!(
            f,
            "
#[allow(non_camel_case_types)]
pub struct {name};

impl actix_web::dev::HttpServiceFactory for {name} {{
    fn register(self, config: &mut actix_web::dev::AppService) {{
        {ast}

        let resource = actix_web::Resource::new(\"{path}\"){guards}.{to}({name});

        actix_web::dev::HttpServiceFactory::register(resource, config)
    }}
}}",
            name = self.name,
            ast = quote!(#ast),
            path = self.path,
            guards = guards,
            to = self.resource_type
        )
    }
}

fn guess_resource_type(typ: &syn::Type) -> ResourceType {
    let mut guess = ResourceType::Sync;

    match typ {
        syn::Type::ImplTrait(typ) => {
            for bound in typ.bounds.iter() {
                match bound {
                    syn::TypeParamBound::Trait(bound) => {
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
                    _ => (),
                }
            }
        }
        _ => (),
    }

    guess
}

impl Args {
    pub fn new(
        args: &Vec<syn::NestedMeta>,
        input: TokenStream,
        guard: GuardType,
    ) -> Self {
        if args.is_empty() {
            panic!(
                "invalid server definition, expected: #[{}(\"some path\")]",
                guard
            );
        }

        let ast: syn::ItemFn = syn::parse(input).expect("Parse input as function");
        let name = ast.ident.clone();

        let mut extra_guards = Vec::new();
        let mut path = None;
        for arg in args {
            match arg {
                syn::NestedMeta::Literal(syn::Lit::Str(ref fname)) => {
                    if path.is_some() {
                        panic!("Multiple paths specified! Should be only one!")
                    }
                    let fname = quote!(#fname).to_string();
                    path = Some(fname.as_str()[1..fname.len() - 1].to_owned())
                }
                syn::NestedMeta::Meta(syn::Meta::NameValue(ident)) => {
                    match ident.ident.to_string().to_lowercase().as_str() {
                        "guard" => match ident.lit {
                            syn::Lit::Str(ref text) => extra_guards.push(text.value()),
                            _ => panic!("Attribute guard expects literal string!"),
                        },
                        attr => panic!(
                            "Unknown attribute key is specified: {}. Allowed: guard",
                            attr
                        ),
                    }
                }
                attr => panic!("Unknown attribute{:?}", attr),
            }
        }

        let resource_type = if ast.asyncness.is_some() {
            ResourceType::Async
        } else {
            match ast.decl.output {
                syn::ReturnType::Default => {
                    panic!("Function {} has no return type. Cannot be used as handler")
                }
                syn::ReturnType::Type(_, ref typ) => guess_resource_type(typ.as_ref()),
            }
        };

        let path = path.unwrap();

        Self {
            name,
            path,
            ast,
            resource_type,
            guard,
            extra_guards,
        }
    }

    pub fn generate(&self) -> TokenStream {
        let text = self.to_string();

        match text.parse() {
            Ok(res) => res,
            Err(error) => panic!("Error: {:?}\nGenerated code: {}", error, text),
        }
    }
}
