//! Multipart form derive macro for Actix Web.

#![deny(rust_2018_idioms, nonstandard_style)]
#![warn(future_incompatible)]
#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]
#![cfg_attr(docsrs, feature(doc_cfg))]

use std::{collections::HashSet, convert::TryFrom as _};

use darling::{FromDeriveInput, FromField, FromMeta};
use parse_size::parse_size;
use proc_macro2::Ident;
use quote::quote;
use syn::{parse_macro_input, Type};

#[derive(FromDeriveInput, Default)]
#[darling(attributes(multipart), default)]
struct MultipartFormAttrs {
    deny_unknown_fields: bool,
    duplicate_action: DuplicateAction,
}

#[derive(FromMeta)]
enum DuplicateAction {
    Ignore,
    Deny,
    Replace,
}

impl Default for DuplicateAction {
    fn default() -> Self {
        Self::Ignore
    }
}

#[derive(FromField, Default)]
#[darling(attributes(multipart), default)]
struct FieldAttrs {
    rename: Option<String>,
    limit: Option<String>,
}

struct ParsedField<'t> {
    serialization_name: String,
    rust_name: &'t Ident,
    limit: Option<usize>,
    ty: &'t Type,
}

/// Implements the `MultipartFormTrait` for a struct so that it can be used with the `MultipartForm`
/// extractor.
///
/// # Examples
///
/// ## Basic Use
///
/// Each field type should implement the `FieldReader` trait:
///
/// ```
/// # use actix_multipart::form::tempfile::Tempfile;
/// # use actix_multipart::form::text::Text;
/// # use actix_multipart::form::MultipartForm;
/// #[derive(MultipartForm)]
/// struct ImageUpload {
///     description: Text<String>,
///     timestamp: Text<i64>,
///     image: Tempfile,
/// }
/// ```
///
/// ## Optional and List Fields
///
/// You can also use `Vec<T>` and `Option<T>` provided that `T: FieldReader`.
///
/// A [`Vec`] field corresponds to an upload with multiple parts under the
/// [same field name](https://www.rfc-editor.org/rfc/rfc7578#section-4.3).
///
/// ```
/// # use actix_multipart::form::tempfile::Tempfile;
/// # use actix_multipart::form::text::Text;
/// # use actix_multipart::form::MultipartForm;
/// #[derive(MultipartForm)]
/// struct Form {
///     category: Option<Text<String>>,
///     files: Vec<Tempfile>,
/// }
/// ```
///
/// ## Field Renaming
///
/// You can use the `#[multipart(rename = "foo")]` attribute to receive a field by a different name.
///
/// ```
/// # use actix_multipart::form::tempfile::Tempfile;
/// # use actix_multipart::form::MultipartForm;
/// #[derive(MultipartForm)]
/// struct Form {
///     #[multipart(rename = "files[]")]
///     files: Vec<Tempfile>,
/// }
/// ```
///
/// ## Field Limits
///
/// You can use the `#[multipart(limit = "<size>")]` attribute to set field level limits. The limit
/// string is parsed using [parse_size].
///
/// Note: the form is also subject to the global limits configured using `MultipartFormConfig`.
///
/// ```
/// # use actix_multipart::form::tempfile::Tempfile;
/// # use actix_multipart::form::text::Text;
/// # use actix_multipart::form::MultipartForm;
/// #[derive(MultipartForm)]
/// struct Form {
///     #[multipart(limit = "2KiB")]
///     description: Text<String>,
///     #[multipart(limit = "512MiB")]
///     files: Vec<Tempfile>,
/// }
/// ```
///
/// ## Unknown Fields
///
/// By default fields with an unknown name are ignored. You can change this using the
/// `#[multipart(deny_unknown_fields)]` attribute:
///
/// ```
/// # use actix_multipart::form::MultipartForm;
/// #[derive(MultipartForm)]
/// #[multipart(deny_unknown_fields)]
/// struct Form { }
/// ```
///
/// ## Duplicate Fields
///
/// You can change the behaviour for when multiple fields are received with the same name using the
/// `#[multipart(duplicate_action = "")]` attribute:
///
/// - "ignore": Extra fields are ignored (default).
/// - "replace": Each field is processed, but only the last one is persisted.
/// - "deny": A `MultipartError::UnsupportedField` error is returned.
///
/// (Note this option does not apply to `Vec` fields)
///
/// ```
/// # use actix_multipart::form::MultipartForm;
/// #[derive(MultipartForm)]
/// #[multipart(duplicate_action = "deny")]
/// struct Form { }
/// ```
///
/// [parse_size]: https://docs.rs/parse-size/1/parse_size
#[proc_macro_derive(MultipartForm, attributes(multipart))]
pub fn impl_multipart_form(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input: syn::DeriveInput = parse_macro_input!(input);

    let name = &input.ident;
    let str = match &input.data {
        syn::Data::Struct(s) => s,
        _ => panic!("This trait can only be derived for a struct"),
    };
    let fields = match &str.fields {
        syn::Fields::Named(n) => n,
        _ => panic!("This trait can only be derived for a struct"),
    };

    let attrs: MultipartFormAttrs = match MultipartFormAttrs::from_derive_input(&input) {
        Ok(attrs) => attrs,
        Err(e) => return e.write_errors().into(),
    };

    // Parse the field attributes
    let parsed = match fields
        .named
        .iter()
        .map(|field| {
            let rust_name = field.ident.as_ref().unwrap();
            let attrs: FieldAttrs = FieldAttrs::from_field(field)?;
            let serialization_name = attrs.rename.unwrap_or_else(|| rust_name.to_string());

            let limit = attrs.limit.map(|limit| {
                usize::try_from(
                    parse_size(&limit)
                        .unwrap_or_else(|_| panic!("Unable to parse limit `{}`", limit)),
                )
                .unwrap()
            });

            Ok(ParsedField {
                serialization_name,
                rust_name,
                limit,
                ty: &field.ty,
            })
        })
        .collect::<Result<Vec<_>, darling::Error>>()
    {
        Ok(attrs) => attrs,
        Err(e) => return e.write_errors().into(),
    };

    // Check that field names are unique
    let mut set = HashSet::new();
    for f in &parsed {
        if !set.insert(f.serialization_name.clone()) {
            panic!("Multiple fields named: `{}`", f.serialization_name);
        }
    }

    // Return value when a field name is not supported by the form
    let unknown_field_result = if attrs.deny_unknown_fields {
        quote!(::std::result::Result::Err(
            ::actix_multipart::MultipartError::UnsupportedField(field.name().to_string())
        ))
    } else {
        quote!(::std::result::Result::Ok(()))
    };

    // Value for duplicate action
    let duplicate_action = match attrs.duplicate_action {
        DuplicateAction::Ignore => quote!(::actix_multipart::form::DuplicateAction::Ignore),
        DuplicateAction::Deny => quote!(::actix_multipart::form::DuplicateAction::Deny),
        DuplicateAction::Replace => quote!(::actix_multipart::form::DuplicateAction::Replace),
    };

    // read_field() implementation
    let mut read_field_impl = quote!();
    for field in &parsed {
        let name = &field.serialization_name;
        let ty = &field.ty;
        read_field_impl.extend(quote!(
            #name => ::std::boxed::Box::pin(
                <#ty as ::actix_multipart::form::FieldGroupReader>::handle_field(req, field, limits, state, #duplicate_action)
            ),
        ));
    }

    // limit() implementation
    let mut limit_impl = quote!();
    for field in &parsed {
        let name = &field.serialization_name;
        if let Some(value) = field.limit {
            limit_impl.extend(quote!(
                #name => ::std::option::Option::Some(#value),
            ));
        }
    }

    // from_state() implementation
    let mut from_state_impl = quote!();
    for field in &parsed {
        let name = &field.serialization_name;
        let rust_name = &field.rust_name;
        let ty = &field.ty;
        from_state_impl.extend(quote!(
            #rust_name: <#ty as ::actix_multipart::form::FieldGroupReader>::from_state(#name, &mut state)?,
        ));
    }

    let gen = quote! {
        impl ::actix_multipart::form::MultipartFormTrait for #name {
            fn limit(field_name: &str) -> ::std::option::Option<usize> {
                match field_name {
                    #limit_impl
                    _ => None,
                }
            }

            fn handle_field<'t>(
                req: &'t ::actix_web::HttpRequest,
                field: ::actix_multipart::Field,
                limits: &'t mut ::actix_multipart::form::Limits,
                state: &'t mut ::actix_multipart::form::State,
            ) -> ::std::pin::Pin<::std::boxed::Box<dyn ::std::future::Future<Output = ::std::result::Result<(), ::actix_multipart::MultipartError>> + 't>> {
                match field.name() {
                    #read_field_impl
                    _ => return ::std::boxed::Box::pin(::std::future::ready(#unknown_field_result)),
                }
            }

            fn from_state(mut state: ::actix_multipart::form::State) -> ::std::result::Result<Self, ::actix_multipart::MultipartError> {
                Ok(Self {
                    #from_state_impl
                })
            }

        }
    };
    gen.into()
}
