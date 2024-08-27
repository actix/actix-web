//! Multipart form derive macro for Actix Web.
//!
//! See [`macro@MultipartForm`] for usage examples.

#![doc(html_logo_url = "https://actix.rs/img/logo.png")]
#![doc(html_favicon_url = "https://actix.rs/favicon.ico")]
#![cfg_attr(docsrs, feature(doc_auto_cfg))]
#![allow(clippy::disallowed_names)] // false positives in some macro expansions

use std::collections::HashSet;

use darling::{FromDeriveInput, FromField, FromMeta};
use parse_size::parse_size;
use proc_macro::TokenStream;
use proc_macro2::Ident;
use quote::quote;
use syn::{parse_macro_input, Type};

#[derive(FromMeta)]
enum DuplicateField {
    Ignore,
    Deny,
    Replace,
}

impl Default for DuplicateField {
    fn default() -> Self {
        Self::Ignore
    }
}

#[derive(FromDeriveInput, Default)]
#[darling(attributes(multipart), default)]
struct MultipartFormAttrs {
    deny_unknown_fields: bool,
    duplicate_field: DuplicateField,
}

#[allow(clippy::disallowed_names)] // false positive in macro expansion
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

/// Implements `MultipartCollect` for a struct so that it can be used with the `MultipartForm`
/// extractor.
///
/// # Basic Use
///
/// Each field type should implement the `FieldReader` trait:
///
/// ```
/// use actix_multipart::form::{tempfile::TempFile, text::Text, MultipartForm};
///
/// #[derive(MultipartForm)]
/// struct ImageUpload {
///     description: Text<String>,
///     timestamp: Text<i64>,
///     image: TempFile,
/// }
/// ```
///
/// # Optional and List Fields
///
/// You can also use `Vec<T>` and `Option<T>` provided that `T: FieldReader`.
///
/// A [`Vec`] field corresponds to an upload with multiple parts under the [same field
/// name](https://www.rfc-editor.org/rfc/rfc7578#section-4.3).
///
/// ```
/// use actix_multipart::form::{tempfile::TempFile, text::Text, MultipartForm};
///
/// #[derive(MultipartForm)]
/// struct Form {
///     category: Option<Text<String>>,
///     files: Vec<TempFile>,
/// }
/// ```
///
/// # Field Renaming
///
/// You can use the `#[multipart(rename = "foo")]` attribute to receive a field by a different name.
///
/// ```
/// use actix_multipart::form::{tempfile::TempFile, MultipartForm};
///
/// #[derive(MultipartForm)]
/// struct Form {
///     #[multipart(rename = "files[]")]
///     files: Vec<TempFile>,
/// }
/// ```
///
/// # Field Limits
///
/// You can use the `#[multipart(limit = "<size>")]` attribute to set field level limits. The limit
/// string is parsed using [parse_size].
///
/// Note: the form is also subject to the global limits configured using `MultipartFormConfig`.
///
/// ```
/// use actix_multipart::form::{tempfile::TempFile, text::Text, MultipartForm};
///
/// #[derive(MultipartForm)]
/// struct Form {
///     #[multipart(limit = "2 KiB")]
///     description: Text<String>,
///
///     #[multipart(limit = "512 MiB")]
///     files: Vec<TempFile>,
/// }
/// ```
///
/// # Unknown Fields
///
/// By default fields with an unknown name are ignored. They can be rejected using the
/// `#[multipart(deny_unknown_fields)]` attribute:
///
/// ```
/// # use actix_multipart::form::MultipartForm;
/// #[derive(MultipartForm)]
/// #[multipart(deny_unknown_fields)]
/// struct Form { }
/// ```
///
/// # Duplicate Fields
///
/// The behaviour for when multiple fields with the same name are received can be changed using the
/// `#[multipart(duplicate_field = "<behavior>")]` attribute:
///
/// - "ignore": (default) Extra fields are ignored. I.e., the first one is persisted.
/// - "deny": A `MultipartError::UnknownField` error response is returned.
/// - "replace": Each field is processed, but only the last one is persisted.
///
/// Note that `Vec` fields will ignore this option.
///
/// ```
/// # use actix_multipart::form::MultipartForm;
/// #[derive(MultipartForm)]
/// #[multipart(duplicate_field = "deny")]
/// struct Form { }
/// ```
///
/// [parse_size]: https://docs.rs/parse-size/1/parse_size
#[proc_macro_derive(MultipartForm, attributes(multipart))]
pub fn impl_multipart_form(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input: syn::DeriveInput = parse_macro_input!(input);

    let name = &input.ident;

    let data_struct = match &input.data {
        syn::Data::Struct(data_struct) => data_struct,
        _ => {
            return compile_err(syn::Error::new(
                input.ident.span(),
                "`MultipartForm` can only be derived for structs",
            ))
        }
    };

    let fields = match &data_struct.fields {
        syn::Fields::Named(fields_named) => fields_named,
        _ => {
            return compile_err(syn::Error::new(
                input.ident.span(),
                "`MultipartForm` can only be derived for a struct with named fields",
            ))
        }
    };

    let attrs = match MultipartFormAttrs::from_derive_input(&input) {
        Ok(attrs) => attrs,
        Err(err) => return err.write_errors().into(),
    };

    // Parse the field attributes
    let parsed = match fields
        .named
        .iter()
        .map(|field| {
            let rust_name = field.ident.as_ref().unwrap();
            let attrs = FieldAttrs::from_field(field).map_err(|err| err.write_errors())?;
            let serialization_name = attrs.rename.unwrap_or_else(|| rust_name.to_string());

            let limit = match attrs.limit.map(|limit| match parse_size(&limit) {
                Ok(size) => Ok(usize::try_from(size).unwrap()),
                Err(err) => Err(syn::Error::new(
                    field.ident.as_ref().unwrap().span(),
                    format!("Could not parse size limit `{}`: {}", limit, err),
                )),
            }) {
                Some(Err(err)) => return Err(compile_err(err)),
                limit => limit.map(Result::unwrap),
            };

            Ok(ParsedField {
                serialization_name,
                rust_name,
                limit,
                ty: &field.ty,
            })
        })
        .collect::<Result<Vec<_>, TokenStream>>()
    {
        Ok(attrs) => attrs,
        Err(err) => return err,
    };

    // Check that field names are unique
    let mut set = HashSet::new();
    for field in &parsed {
        if !set.insert(field.serialization_name.clone()) {
            return compile_err(syn::Error::new(
                field.rust_name.span(),
                format!("Multiple fields named: `{}`", field.serialization_name),
            ));
        }
    }

    // Return value when a field name is not supported by the form
    let unknown_field_result = if attrs.deny_unknown_fields {
        quote!(::std::result::Result::Err(
            ::actix_multipart::MultipartError::UnknownField(field.name().unwrap().to_string())
        ))
    } else {
        quote!(::std::result::Result::Ok(()))
    };

    // Value for duplicate action
    let duplicate_field = match attrs.duplicate_field {
        DuplicateField::Ignore => quote!(::actix_multipart::form::DuplicateField::Ignore),
        DuplicateField::Deny => quote!(::actix_multipart::form::DuplicateField::Deny),
        DuplicateField::Replace => quote!(::actix_multipart::form::DuplicateField::Replace),
    };

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

    // handle_field() implementation
    let mut handle_field_impl = quote!();
    for field in &parsed {
        let name = &field.serialization_name;
        let ty = &field.ty;

        handle_field_impl.extend(quote!(
            #name => ::std::boxed::Box::pin(
                <#ty as ::actix_multipart::form::FieldGroupReader>::handle_field(req, field, limits, state, #duplicate_field)
            ),
        ));
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
        impl ::actix_multipart::form::MultipartCollect for #name {
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
                match field.name().unwrap() {
                    #handle_field_impl
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

/// Transform a syn error into a token stream for returning.
fn compile_err(err: syn::Error) -> TokenStream {
    TokenStream::from(err.to_compile_error())
}
