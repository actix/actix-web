use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse_macro_input, Data, DataStruct, DeriveInput, Field, Fields, FieldsNamed, Ident,
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
        let Field { ident, .. } = f;

        // TODO: parse field attributes find
        // #[multipart(max_size = n)]

        quote! { stringify!(#ident) => Some(8096) }
    });

    let build_fields = fields.iter().map(|f| {
        let Field { ident, .. } = f;
        quote! { #ident: self.#ident.unwrap() }
    });

    let field_appending = fields.iter().map(|f| {
        let Field { ident, ty, .. } = f;

        quote! {
           stringify!(#ident) => match builder.#ident {
               Some(ref mut field) => {
                   field.append(chunk);
               }
               None => {
                   let mut field = #ty::default();
                   field.append(chunk);
                   builder.#ident.replace(field);
               }
           },

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
                use futures_util::future::FutureExt;
                use futures_util::stream::StreamExt;
                use actix_multipart::BuildFromBytes;

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
                        println!("FIELD: {}", name);

                        let mut size = 0;

                        while let Some(chunk) = field.next().await {
                            let chunk = chunk?;
                            size += chunk.len();

                            if (size > #b_ident::max_size(&name).unwrap_or(std::usize::MAX)) {
                                return Err(::actix_web::error::ErrorPayloadTooLarge("field is too large"));
                            }

                            match name {
                                #(#field_appending)*

                                _ => todo!("unknown field"),
                            }
                        }

                        println!();
                    }

                    builder.build()
                }
                .boxed_local()
            }
        }
    };

    expanded.into()
}
