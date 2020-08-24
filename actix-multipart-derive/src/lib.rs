use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse_macro_input, Data, DataStruct, DeriveInput, Field, Fields, FieldsNamed, Ident,
    Meta, MetaList, MetaNameValue, NestedMeta, Path,
};

#[proc_macro_derive(MultipartForm, attributes(multipart))]
pub fn derive(input: TokenStream) -> TokenStream {
    let ast = parse_macro_input!(input as DeriveInput);

    let name = ast.ident;

    let b_name = format!("{}MultipartBuilder", name);
    let b_ident = Ident::new(&b_name, name.span());

    let fields = if let Data::Struct(DataStruct {
        fields: Fields::Named(FieldsNamed { ref named, .. }),
        ..
    }) = ast.data
    {
        named
    } else {
        unimplemented!()
    };

    let optioned = fields.iter().map(|f| {
        let Field { ident, ty, .. } = f;
        quote! { #ident: ::std::option::Option<#ty> }
    });

    let field_max_sizes = fields.iter().map(|f| {
        let Field { ident, attrs, .. } = f;

        for attr in attrs {
            // TODO: use something like https://github.com/TedDriggs/darling ??

            if let Ok(m) = attr.parse_meta() {
                if let Meta::List(MetaList { path, nested, .. }) = m {
                    if path.get_ident().unwrap()
                        != &Ident::new("multipart", proc_macro2::Span::call_site())
                    {
                        continue;
                    }

                    // it's our meta list, marked by multipart

                    if let Some(NestedMeta::Meta(Meta::NameValue(MetaNameValue {
                        path: Path { segments, .. },
                        lit,
                        ..
                    }))) = nested.first()
                    {
                        for seg in segments {
                            // if there's a max_size attr in the list, extract the lit
                            if &seg.ident
                                == &Ident::new(
                                    "max_size",
                                    proc_macro2::Span::call_site(),
                                )
                            {
                                // TODO: ensure literal is numeric
                                return quote! { stringify!(#ident) => Some(#lit) };
                            }
                        }
                    }
                }
            }
        }

        quote! { stringify!(#ident) => None }
    });

    let build_fields = fields.iter().map(|f| {
        let Field { ident, .. } = f;
        quote! { #ident: self.#ident.unwrap() }
    });

    let bytes_appending = fields.iter().map(|f| {
        let Field { ident, .. } = f;

        quote! {
           stringify!(#ident) => field_bytes.put(chunk),
        }
    });

    let fields_from_bytes = fields.iter().map(|f| {
        let Field { ident, ty, .. } = f;

        quote! {
           stringify!(#ident) => {
                builder.#ident.replace(#ty::from_bytes(field_bytes));
            }
        }
    });

    let expanded = quote! {
        #[derive(Debug, Clone, Default)]
        struct #b_ident {
            #(#optioned,)*
        }

        impl #b_ident {
            fn max_size(field: &str) -> Option<usize> {
                match field {
                    #(#field_max_sizes,)*
                    _ => None,
                }
            }

            fn build(self) -> Result<#name, ::actix_web::Error> {
                Ok(Form {
                    #(#build_fields,)*
                })
            }
        }

        impl ::actix_web::FromRequest for #name {
            type Error = ::actix_web::Error;
            type Future = ::futures_util::future::LocalBoxFuture<'static, Result<Self, Self::Error>>;
            type Config = ();

            fn from_request(req: &::actix_web::HttpRequest, payload: &mut ::actix_web::dev::Payload) -> Self::Future {
                use ::futures_util::future::FutureExt;
                use ::futures_util::stream::StreamExt;
                use ::actix_web::error;
                use ::actix_multipart::{FromBytes, Multipart};
                use ::bytes::{BufMut, BytesMut};

                let pl = payload.take();
                let req2 = req.clone();

                async move {
                    let mut mp = ::actix_multipart::Multipart::new(req2.headers(), pl);

                    let mut builder = #b_ident::default();

                    while let Some(item) = mp.next().await {
                        let mut field = item?;

                        let headers = field.headers();

                        let cd = field.content_disposition().unwrap();
                        let name = cd.get_name().unwrap();

                        let mut size = 0;
                        let mut field_bytes = BytesMut::new();

                        while let Some(chunk) = field.next().await {
                            let chunk = chunk?;
                            size += chunk.len();

                            if (size > #b_ident::max_size(&name).unwrap_or(std::usize::MAX)) {
                                return Err(error::ErrorPayloadTooLarge("field is too large"));
                            }

                            match name {
                                #(#bytes_appending)*

                                _ => {
                                    // unknown field
                                },
                            }
                        }

                        let field_bytes = field_bytes.freeze();

                        match name {
                            #(#fields_from_bytes)*

                            _ => {
                                // unknown field
                            },
                        }
                    }

                    builder.build()
                }
                .boxed_local()
            }
        }
    };

    expanded.into()
}
